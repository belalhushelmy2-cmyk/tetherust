//! NetworkManager configures only an application-owned TUN. The GUI and packet engine stay unprivileged.
use anyhow::{Result, ensure};
use std::{
    fs::File,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    pin::Pin,
    task::{Context as TaskContext, Poll, ready},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, unix::AsyncFd};
pub const INTERFACE: &str = "tetherust0";
pub struct Tun(AsyncFd<File>);
impl Tun {
    pub fn open() -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open("/dev/net/tun")?;
        // SAFETY: zero is valid for all ifreq fields; name fits IFNAMSIZ, fd stays live.
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        for (out, input) in request.ifr_name.iter_mut().zip(INTERFACE.bytes()) {
            *out = input as libc::c_char;
        }
        request.ifr_ifru.ifru_flags = (libc::IFF_TUN | libc::IFF_NO_PI) as libc::c_short;
        // NetworkManager sets this TUN's owner/group to this user's IDs before we open it.
        let result = unsafe { libc::ioctl(file.as_raw_fd(), libc::TUNSETIFF, &request) };
        ensure!(
            result >= 0,
            "Cannot open the user-owned TUN: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: this object exclusively owns the File; it cannot be closed or replaced while registered.
        Ok(Self(unsafe { AsyncFd::register(file) }.map_err(|_| {
            anyhow::anyhow!("Register TUN readiness failed")
        })?))
    }
}
impl AsyncRead for Tun {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        loop {
            let mut guard = ready!(self.0.poll_read_ready(cx))?;
            match guard.try_io(|fd| fd.get_ref().read(buffer.initialize_unfilled())) {
                Ok(Ok(n)) => {
                    buffer.advance(n);
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(e)) => return Poll::Ready(Err(e)),
                Err(_) => continue,
            }
        }
    }
}
impl AsyncWrite for Tun {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        loop {
            let mut guard = ready!(self.0.poll_write_ready(cx))?;
            match guard.try_io(|fd| fd.get_ref().write(buffer)) {
                Ok(result) => return Poll::Ready(result),
                Err(_) => continue,
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
pub fn args(endpoint: std::net::SocketAddr, password: &str) -> Result<tun2proxy::Args> {
    ensure!(
        endpoint.ip().is_loopback() && password.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid local proxy configuration"
    );
    let proxy = tun2proxy::ProxyParameters::try_from(
        format!("socks5://tetherust:{password}@{endpoint}").as_str(),
    )
    .map_err(|_| anyhow::anyhow!("Invalid proxy URL"))?;
    Ok(tun2proxy::Args {
        proxy,
        setup: false,
        ipv6_enabled: true,
        dns: tun2proxy::ArgDns::Virtual,
        dns_addr: "198.18.0.1".parse()?,
        max_sessions: 128,
        ..Default::default()
    })
}
