use crate::device::{Device, Transport, valid_interface};
use anyhow::{Context, Result, bail, ensure};
use http_body_util::{BodyExt, Empty};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io::Read,
    net::{IpAddr, SocketAddr},
    os::fd::AsRawFd,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpSocket, TcpStream, UdpSocket},
    time::timeout,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Health {
    pub public_ip: String,
    pub latency_ms: u64,
    pub dns_ms: u64,
    pub address_family: String,
    pub preferred_route: bool,
}
fn interface_index(interface: &str) -> Result<u32> {
    ensure!(valid_interface(interface), "Invalid interface");
    let name = std::ffi::CString::new(interface)?;
    // SAFETY: name is a valid NUL-terminated string.
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    ensure!(index != 0, "Interface disappeared");
    Ok(index)
}
// Linux permits an initial SO_BINDTODEVICE on a fresh socket on current kernels.
// Older kernels / desktop policy may reject it: fail closed, never silently use another link.
fn bind_interface(fd: std::os::fd::RawFd, interface: &str) -> Result<()> {
    ensure!(valid_interface(interface), "Invalid interface");
    let name = std::ffi::CString::new(interface)?;
    // SAFETY: name and its NUL terminator are live for the syscall; fd is a live socket.
    let result = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            name.as_ptr().cast(),
            name.as_bytes_with_nul().len() as libc::socklen_t,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error()).context("Desktop policy/kernel rejected interface binding; no alternate-interface fallback is permitted");
    }
    Ok(())
}
fn scoped_address(mut address: SocketAddr, index: u32) -> SocketAddr {
    if let SocketAddr::V6(v6) = &mut address
        && v6.ip().is_unicast_link_local()
    {
        v6.set_scope_id(index);
    }
    address
}
async fn tcp(interface: &str, endpoint: SocketAddr) -> Result<TcpStream> {
    let index = interface_index(interface)?;
    let socket = if endpoint.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    bind_interface(socket.as_raw_fd(), interface)?;
    let stream = socket.connect(scoped_address(endpoint, index)).await?;
    stream.set_nodelay(true)?;
    Ok(stream)
}
// Interface-selected DNS and HTTPS. No system resolver, proxy environment, or direct fallback.
pub async fn verify(device: &Device) -> Result<Health> {
    timeout(Duration::from_secs(12), verify_inner(device))
        .await
        .context("Interface DNS/TLS verification timed out")?
}
async fn verify_inner(device: &Device) -> Result<Health> {
    ensure!(valid_interface(&device.interface), "Invalid interface");
    ensure!(
        device.state == 100,
        "NetworkManager has not activated this interface"
    );
    let start = Instant::now();
    let mut failures = Vec::new();
    let ipv4 = device
        .addresses
        .iter()
        .any(|a| a.parse::<IpAddr>().is_ok_and(|a| a.is_ipv4()));
    let ipv6 = device.addresses.iter().any(|a| {
        a.parse::<IpAddr>()
            .is_ok_and(|a| matches!(a, IpAddr::V6(a) if !a.is_unicast_link_local()))
    });
    for kind in [1u16, 28] {
        if kind == 1 && !ipv4 || kind == 28 && !ipv6 {
            continue;
        }
        for dns in device.dns.iter().take(3) {
            let Ok(server) = dns.parse::<IpAddr>() else {
                continue;
            };
            let dns_start = Instant::now();
            let resolved = timeout(
                Duration::from_secs(2),
                dns_lookup(&device.interface, server, kind),
            )
            .await;
            let addresses = match resolved {
                Ok(Ok(a)) => a,
                Ok(Err(e)) => {
                    failures.push(format!("{e:#}"));
                    continue;
                }
                Err(_) => {
                    failures.push("phone DNS timed out".into());
                    continue;
                }
            };
            let dns_ms = dns_start.elapsed().as_millis() as u64;
            for address in addresses.into_iter().take(2) {
                let probe = async { trace(tcp(&device.interface, address).await?).await };
                match timeout(Duration::from_secs(5), probe).await {
                    Ok(Ok(public_ip)) => {
                        return Ok(Health {
                            public_ip,
                            latency_ms: start.elapsed().as_millis() as u64,
                            dns_ms,
                            address_family: if address.is_ipv4() { "IPv4" } else { "IPv6" }.into(),
                            preferred_route: preferred_route(&device.interface, address.ip())
                                .await
                                .unwrap_or(false),
                        });
                    }
                    Ok(Err(e)) => failures.push(format!("{e:#}")),
                    Err(_) => failures.push("HTTPS timed out".into()),
                }
            }
        }
    }
    bail!(
        "Interface-selected DNS/HTTPS verification failed: {}",
        if failures.is_empty() {
            "no usable phone address or DNS server".into()
        } else {
            failures.join("; ")
        }
    )
}
pub async fn trace<S>(stream: S) -> Result<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tls = TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from("one.one.one.one")?, stream)
        .await
        .context("Certificate-verified TLS probe failed")?;
    let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
        .max_buf_size(8192)
        .handshake(hyper_util::rt::TokioIo::new(tls))
        .await?;
    let driver = tokio::spawn(connection);
    struct Abort(tokio::task::JoinHandle<Result<(), hyper::Error>>);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _abort = Abort(driver);
    let request = hyper::Request::builder()
        .uri("/cdn-cgi/trace")
        .header("Host", "one.one.one.one")
        .header("Connection", "close")
        .body(Empty::<bytes::Bytes>::new())?;
    let response = sender.send_request(request).await?;
    ensure!(
        response.status().is_success(),
        "HTTPS probe returned {}",
        response.status()
    );
    let mut incoming = response.into_body();
    let mut body = Vec::new();
    while let Some(frame) = incoming.frame().await {
        let frame = frame?;
        if let Some(chunk) = frame.data_ref() {
            ensure!(
                body.len() + chunk.len() <= 8192,
                "Unexpectedly large probe response"
            );
            body.extend_from_slice(chunk);
        }
    }
    let text = std::str::from_utf8(&body)?;
    let ip = text
        .lines()
        .find_map(|line| line.strip_prefix("ip="))
        .context("Internet probe returned no public IP; possible captive portal")?;
    let _: IpAddr = ip.parse().context("Invalid public IP response")?;
    Ok(ip.into())
}
async fn dns_lookup(interface: &str, server: IpAddr, kind: u16) -> Result<Vec<SocketAddr>> {
    let index = interface_index(interface)?;
    let family = if server.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(family, Type::DGRAM, Some(Protocol::UDP))?;
    bind_interface(socket.as_raw_fd(), interface)?;
    socket.set_nonblocking(true)?;
    let address = if server.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    }
    .parse::<SocketAddr>()?;
    socket.bind(&address.into())?;
    let udp = UdpSocket::from_std(socket.into())?;
    udp.connect(scoped_address(SocketAddr::new(server, 53), index))
        .await?;
    let mut id = [0u8; 2];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut id)?;
    let query = dns_query(id, kind);
    udp.send(&query).await?;
    let mut buffer = [0u8; 4096];
    let n = udp.recv(&mut buffer).await?;
    parse_dns(&query, &buffer[..n])
}
pub fn dns_query(id: [u8; 2], kind: u16) -> Vec<u8> {
    let mut q = vec![id[0], id[1], 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for part in ["one", "one", "one", "one"] {
        q.push(part.len() as u8);
        q.extend_from_slice(part.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&kind.to_be_bytes());
    q.extend_from_slice(&1u16.to_be_bytes());
    q
}
fn word(bytes: &[u8], at: usize) -> Result<u16> {
    let data = bytes.get(at..at + 2).context("Truncated DNS message")?;
    Ok(u16::from_be_bytes([data[0], data[1]]))
}
fn skip_name(bytes: &[u8], offset: &mut usize) -> Result<()> {
    for _ in 0..128 {
        let b = *bytes.get(*offset).context("Truncated DNS name")?;
        *offset += 1;
        if b == 0 {
            return Ok(());
        }
        if b & 0xc0 == 0xc0 {
            ensure!(*offset < bytes.len(), "Truncated DNS pointer");
            *offset += 1;
            return Ok(());
        }
        ensure!(b <= 63, "Invalid DNS label");
        *offset += b as usize;
        ensure!(*offset <= bytes.len(), "Truncated DNS label");
    }
    bail!("DNS name too long")
}
pub fn parse_dns(query: &[u8], response: &[u8]) -> Result<Vec<SocketAddr>> {
    ensure!(
        query.len() >= 16 && response.len() >= query.len(),
        "Truncated DNS response"
    );
    ensure!(response[..2] == query[..2], "DNS transaction mismatch");
    ensure!(
        response[2] & 0x80 != 0 && response[2] & 0x7a == 0 && response[3] & 15 == 0,
        "DNS refused, truncated or invalid response"
    );
    ensure!(
        word(response, 4)? == 1 && response[12..query.len()] == query[12..],
        "DNS question mismatch"
    );
    let mut offset = query.len();
    let mut result = Vec::new();
    for _ in 0..word(response, 6)?.min(128) {
        skip_name(response, &mut offset)?;
        let kind = word(response, offset)?;
        let class = word(response, offset + 2)?;
        let len = word(response, offset + 8)? as usize;
        offset += 10;
        let data = response
            .get(offset..offset + len)
            .context("Truncated DNS record")?;
        let address = match (class, kind, len) {
            (1, 1, 4) => Some(IpAddr::V4(std::net::Ipv4Addr::new(
                data[0], data[1], data[2], data[3],
            ))),
            (1, 28, 16) => Some(IpAddr::V6(std::net::Ipv6Addr::from(<[u8; 16]>::try_from(
                data,
            )?))),
            _ => None,
        };
        if let Some(address) = address {
            ensure!(
                !address.is_unspecified() && !address.is_loopback() && !address.is_multicast(),
                "Unsafe probe DNS address"
            );
            result.push(SocketAddr::new(address, 443));
        }
        offset += len;
    }
    ensure!(!result.is_empty(), "DNS returned no usable address");
    Ok(result)
}
pub async fn preferred_route(interface: &str, target: IpAddr) -> Result<bool> {
    let output = timeout(
        Duration::from_secs(2),
        tokio::process::Command::new("ip")
            .args(["-j", "route", "get", &target.to_string()])
            .kill_on_drop(true)
            .output(),
    )
    .await??;
    ensure!(output.status.success(), "Kernel route lookup failed");
    let routes: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(routes
        .as_array()
        .and_then(|a| a.first())
        .and_then(|r| r.get("dev"))
        .and_then(|d| d.as_str())
        == Some(interface))
}
// Diagnostic only. Standard HTTP CONNECT carries TCP; it does not configure system routes.
pub async fn verify_proxy(device: &Device, endpoint: SocketAddr) -> Result<Health> {
    ensure!(
        matches!(device.transport, Transport::Wifi | Transport::WifiDirect),
        "Proxy probing requires a selected Wi-Fi interface"
    );
    ensure!(
        endpoint.port() != 0 && endpoint.ip().to_string() == device.gateway,
        "Only the selected interface's advertised gateway may be probed"
    );
    timeout(Duration::from_secs(10), async {
        let start = Instant::now(); let mut stream = tcp(&device.interface, endpoint).await?;
        stream.write_all(b"CONNECT one.one.one.one:443 HTTP/1.1\r\nHost: one.one.one.one:443\r\nProxy-Connection: keep-alive\r\n\r\n").await?;
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            ensure!(header.len() < 8192, "Proxy response headers too large");
            header.push(stream.read_u8().await.context("Proxy closed CONNECT response")?);
        }
        let status = std::str::from_utf8(&header)?.lines().next().context("Proxy returned no status")?;
        let mut parts = status.split_whitespace();
        ensure!(matches!(parts.next(), Some("HTTP/1.0" | "HTTP/1.1")) && parts.next() == Some("200"), "Proxy rejected CONNECT (authentication or compatibility may be required)");
        let ip = trace(stream).await?;
        Ok(Health { public_ip: ip, latency_ms: start.elapsed().as_millis() as u64, dns_ms: 0,
            address_family: "HTTP CONNECT / TCP only".into(), preferred_route: false })
    }).await.context("Proxy diagnostic timed out")?
}
