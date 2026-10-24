//! Android companion transport. No ADB action is performed by module initialization or tests.
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use tokio_rustls::{
    TlsConnector,
    client::TlsStream,
    rustls::{
        self, ClientConfig, DigitallySignedStruct, SignatureScheme,
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        pki_types::{CertificateDer, ServerName, UnixTime},
    },
};
use tun2proxy::CancellationToken;

pub const PHONE_PORT: u16 = 38473;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Phone {
    pub serial: String,
    pub name: String,
    pub authorized: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pairing {
    pub version: u32,
    pub name: String,
    pub token: String,
    pub certificate: String,
}
impl Pairing {
    pub fn validate(&self) -> Result<Vec<u8>> {
        ensure!(self.version == 1, "Unsupported companion protocol version");
        ensure!(
            self.token.len() == 64 && self.token.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid pairing token"
        );
        ensure!(self.certificate.len() <= 8192, "Certificate too large");
        let cert = STANDARD.decode(&self.certificate)?;
        ensure!(
            (200..=4096).contains(&cert.len()),
            "Invalid certificate size"
        );
        Ok(cert)
    }
    pub fn load(data: &Path, serial: &str) -> Result<Self> {
        let path = pairing_path(data, serial)?;
        let bytes = std::fs::read(&path).context(
            "Phone is not paired. Start the companion on Android, then choose Pair phone",
        )?;
        ensure!(bytes.len() <= 16384, "Pairing file too large");
        let pair: Self = serde_json::from_slice(&bytes)?;
        pair.validate()?;
        Ok(pair)
    }
}
fn pairing_path(data: &Path, serial: &str) -> Result<PathBuf> {
    ensure!(valid_serial(serial), "Invalid ADB serial");
    Ok(data.join("pairings").join(format!("{serial}.json")))
}
pub fn valid_serial(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}
pub fn adb_path() -> PathBuf {
    if let Some(path) = std::env::var_os("TETHERUST_ADB") {
        return path.into();
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
        && parent.join("adb").is_file()
    {
        return parent.join("adb");
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let private = parent.join("../lib/tetherust/adb");
        if private.is_file() {
            return private;
        }
    }
    "adb".into()
}
async fn adb(arguments: &[&str]) -> Result<Vec<u8>> {
    ensure!(
        std::env::var_os("TETHERUST_ADB_DISABLED").is_none(),
        "ADB disabled for this run"
    );
    let mut child = tokio::process::Command::new(adb_path())
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("ADB not found. Install android-tools or use the packaged ADB")?;
    timeout(Duration::from_secs(20), async {
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .context("ADB stdout unavailable")?
            .take(32769)
            .read_to_end(&mut output)
            .await?;
        ensure!(output.len() <= 32768, "ADB response too large");
        ensure!(
            child.wait().await?.success(),
            "ADB operation failed. Check USB authorization and whether the companion is installed"
        );
        Ok(output)
    })
    .await
    .context("ADB operation timed out")?
}
pub fn parse_devices(bytes: &[u8]) -> Vec<Phone> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 2
                || !valid_serial(fields[0])
                || !["device", "unauthorized", "offline"].contains(&fields[1])
            {
                return None;
            }
            Some(Phone {
                serial: fields[0].into(),
                authorized: fields[1] == "device",
                name: fields
                    .iter()
                    .find_map(|v| v.strip_prefix("model:"))
                    .unwrap_or("Android phone")
                    .replace('_', " "),
            })
        })
        .collect()
}
pub async fn phones() -> Result<Vec<Phone>> {
    Ok(parse_devices(&adb(&["devices", "-l"]).await?))
}
pub async fn pair(data: &Path, serial: &str) -> Result<()> {
    ensure!(valid_serial(serial), "Invalid phone selection");
    let bytes = adb(&[
        "-s",
        serial,
        "shell",
        "content",
        "read",
        "--uri",
        "content://org.tetherust.companion.pairing/identity",
    ])
    .await?;
    let pairing: Pairing = serde_json::from_slice(&bytes)
        .context("Companion pairing endpoint unavailable. Install/open the APK first")?;
    pairing.validate()?;
    crate::storage::atomic_write(
        &pairing_path(data, serial)?,
        &serde_json::to_vec_pretty(&pairing)?,
    )
}
pub async fn install(serial: &str, apk: &Path) -> Result<()> {
    ensure!(valid_serial(serial), "Invalid phone selection");
    ensure!(apk.is_file(), "Companion APK not found");
    let apk = apk.to_str().context("APK path must be UTF-8")?;
    adb(&["-s", serial, "install", "-r", apk]).await?;
    Ok(())
}
pub fn apk_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TETHERUST_COMPANION_APK") {
        return Some(path.into());
    }
    let executable = std::env::current_exe().ok()?;
    let parent = executable.parent()?;
    [
        parent.join("../share/tetherust/Tetherust-Companion-0.3.0.apk"),
        parent.join("Tetherust-Companion-0.3.0.apk"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}
pub fn valid_wifi_endpoint(endpoint: SocketAddr) -> bool {
    endpoint.port() != 0
        && match endpoint.ip() {
            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
            std::net::IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local(),
        }
}
/// Load only identities already paired through authorized USB; discovery never establishes trust.
pub fn paired_phones(data: &Path) -> Vec<Phone> {
    let Ok(entries) = std::fs::read_dir(data.join("pairings")) else {
        return Vec::new();
    };
    entries
        .take(64)
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let serial = entry.path().file_stem()?.to_str()?.to_owned();
            let pair = Pairing::load(data, &serial).ok()?;
            Some(Phone {
                serial,
                name: pair.name,
                authorized: true,
            })
        })
        .collect()
}
pub async fn discover_wifi() -> Result<Vec<SocketAddr>> {
    ensure!(
        std::env::var_os("TETHERUST_ADB_DISABLED").is_none(),
        "Phone access disabled for this run"
    );
    let mut child = tokio::process::Command::new("avahi-browse")
        .args(["--resolve", "--terminate", "--parsable", "_tetherust._tcp"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context(
            "Install avahi tools for Wi-Fi discovery, or enter the phone's LAN IP:38473 manually",
        )?;
    timeout(Duration::from_secs(8), async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .context("Discovery output unavailable")?
            .take(16385)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(
            bytes.len() <= 16384 && child.wait().await?.success(),
            "Wi-Fi discovery failed; enter a reachable LAN endpoint manually"
        );
        Ok(parse_discovery(&bytes))
    })
    .await
    .context("Wi-Fi discovery timed out")?
}
pub fn parse_discovery(bytes: &[u8]) -> Vec<SocketAddr> {
    let mut found = Vec::new();
    for line in String::from_utf8_lossy(bytes).lines() {
        let fields: Vec<_> = line.split(';').collect();
        if fields.len() < 9 || fields[0] != "=" {
            continue;
        }
        if let (Ok(ip), Ok(port)) = (fields[7].parse(), fields[8].parse()) {
            let endpoint = SocketAddr::new(ip, port);
            if valid_wifi_endpoint(endpoint) && !found.contains(&endpoint) {
                found.push(endpoint);
            }
        }
    }
    found
}
pub struct AdbForward {
    serial: String,
    pub port: u16,
}
impl AdbForward {
    pub async fn create(serial: &str) -> Result<Self> {
        ensure!(valid_serial(serial), "Invalid ADB serial");
        let value = adb(&[
            "-s",
            serial,
            "forward",
            "tcp:0",
            &format!("tcp:{PHONE_PORT}"),
        ])
        .await?;
        let port = std::str::from_utf8(&value)?.trim().parse()?;
        ensure!(port != 0, "Invalid forwarded port");
        Ok(Self {
            serial: serial.into(),
            port,
        })
    }
    pub async fn close(&self) -> Result<()> {
        adb(&[
            "-s",
            &self.serial,
            "forward",
            "--remove",
            &format!("tcp:{}", self.port),
        ])
        .await?;
        Ok(())
    }
}
// Pin the exact paired certificate and verify every TLS handshake signature using ring.
// This is private identity authentication, not accept-all or disabled TLS verification.
#[derive(Debug)]
struct PinnedCertificate(Vec<u8>);
impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if bool::from(end.as_ref().ct_eq(&self.0)) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "Phone certificate changed. Pair again only after confirming the phone identity"
                    .into(),
            ))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathStatus {
    pub protocol: u32,
    pub epoch: u64,
    pub vpn: bool,
    pub ipv4_default: bool,
    pub ipv6_default: bool,
    pub blocked: bool,
    pub available: bool,
    pub mode: String,
    pub udp_supported: bool,
}
impl PathStatus {
    pub fn validate(&self, require_vpn: bool) -> Result<()> {
        ensure!(
            self.protocol == 2 && self.epoch > 0,
            "Update both companion and Linux client"
        );
        ensure!(
            self.available && !self.blocked,
            "Android blocks this app or has no default network"
        );
        ensure!(
            ["default", "http", "socks5"].contains(&self.mode.as_str()),
            "Unknown phone routing mode"
        );
        if require_vpn {
            ensure!(
                self.vpn,
                "No VPN applies to Tetherust. Include the companion in the VPN app's allowed list; a VPN icon alone is insufficient"
            );
            ensure!(
                self.ipv4_default && self.mode == "default",
                "Strict mode requires VPN default IPv4 routes and app-default mode; split routes and local proxies cannot satisfy it"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct StrictAnchor {
    pub epoch: u64,
    pub exit_ip: String,
}
#[derive(Clone)]
struct RoutingPolicy {
    epoch: u64,
    require_vpn: bool,
}
#[derive(Clone)]
pub struct Remote {
    pub endpoint: SocketAddr,
    pairing: Pairing,
    config: Arc<ClientConfig>,
    policy: Option<RoutingPolicy>,
}
impl Remote {
    pub fn new(endpoint: SocketAddr, pairing: Pairing) -> Result<Self> {
        ensure!(
            endpoint.ip().is_loopback() || valid_wifi_endpoint(endpoint),
            "Companion endpoint must be a loopback USB forward or private LAN address"
        );
        let cert = pairing.validate()?;
        let config = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedCertificate(cert)))
            .with_no_client_auth();
        Ok(Self {
            endpoint,
            pairing,
            config: Arc::new(config),
            policy: None,
        })
    }
    async fn authenticated(&self) -> Result<TlsStream<TcpStream>> {
        timeout(Duration::from_secs(12), async {
            let tcp = TcpStream::connect(self.endpoint).await?;
            tcp.set_nodelay(true)?;
            let mut stream = TlsConnector::from(self.config.clone())
                .connect(ServerName::try_from("tetherust.companion")?, tcp)
                .await
                .context("Pinned phone TLS failed; start sharing on Android and check pairing")?;
            stream.write_all(&[5, 1, 2]).await?;
            let mut reply = [0; 2];
            stream.read_exact(&mut reply).await?;
            ensure!(reply == [5, 2], "Companion authentication method mismatch");
            let user = b"tetherust";
            stream.write_all(&[1, user.len() as u8]).await?;
            stream.write_all(user).await?;
            stream.write_u8(self.pairing.token.len() as u8).await?;
            stream.write_all(self.pairing.token.as_bytes()).await?;
            stream.flush().await?;
            stream.read_exact(&mut reply).await?;
            ensure!(reply == [1, 0], "Phone rejected pairing token; pair again");
            Ok(stream)
        })
        .await
        .context("Companion handshake timed out")?
    }
    pub async fn path_status(&self) -> Result<PathStatus> {
        timeout(Duration::from_secs(5), async {
            let mut stream = self.authenticated().await?;
            stream.write_all(&[5, 240, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            stream.flush().await?;
            let length = stream.read_u16().await? as usize;
            ensure!((2..=4096).contains(&length), "Invalid phone status length");
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).await?;
            Ok(serde_json::from_slice(&bytes)?)
        })
        .await
        .context("Phone status timed out")?
    }
    pub async fn negotiate(mut self, require_vpn: bool) -> Result<(Self, PathStatus)> {
        let status = self.path_status().await?;
        status.validate(require_vpn)?;
        self.policy = Some(RoutingPolicy {
            epoch: status.epoch,
            require_vpn,
        });
        Ok((self, status))
    }
    pub async fn check_path(&self) -> Result<PathStatus> {
        let policy = self
            .policy
            .as_ref()
            .context("Routing policy was not negotiated")?;
        let status = self.path_status().await?;
        status.validate(policy.require_vpn)?;
        ensure!(
            status.epoch == policy.epoch,
            "Phone network or VPN changed. Strict sessions require explicit reconnect"
        );
        Ok(status)
    }
    async fn data_stream(&self) -> Result<TlsStream<TcpStream>> {
        let policy = self
            .policy
            .as_ref()
            .context("Routing policy was not negotiated")?;
        let mut stream = self.authenticated().await?;
        stream.write_all(&[5, 241, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        stream.write_u8(u8::from(policy.require_vpn)).await?;
        stream.write_u64(policy.epoch).await?;
        Ok(stream)
    }
    pub async fn phone_exit(&self) -> Result<std::net::IpAddr> {
        timeout(Duration::from_secs(15), async {
            let mut stream = self.data_stream().await?;
            stream.write_all(&[5, 242, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            stream.flush().await?;
            let length = stream.read_u16().await? as usize;
            ensure!((2..=256).contains(&length), "Invalid phone exit response");
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).await?;
            #[derive(Deserialize)]
            struct Exit {
                exit_ip: std::net::IpAddr,
            }
            Ok(serde_json::from_slice::<Exit>(&bytes)?.exit_ip)
        })
        .await
        .context("Independent phone exit probe timed out")?
    }
    pub async fn connect_tcp(&self, host: &str, port: u16) -> Result<TlsStream<TcpStream>> {
        ensure!(
            !host.is_empty() && host.len() <= 253 && port != 0,
            "Invalid destination"
        );
        let mut stream = self.data_stream().await?;
        stream.write_all(&[5, 1, 0, 3, host.len() as u8]).await?;
        stream.write_all(host.as_bytes()).await?;
        stream.write_u16(port).await?;
        stream.flush().await?;
        let response = read_request(&mut stream).await?;
        ensure!(
            response[1] == 0,
            "Phone cannot reach destination through the selected app network"
        );
        Ok(stream)
    }
}
pub async fn read_request<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>> {
    let mut result = vec![0; 4];
    reader.read_exact(&mut result).await?;
    ensure!(result[0] == 5 && result[2] == 0, "Malformed SOCKS header");
    let n = match result[3] {
        1 => 4,
        4 => 16,
        3 => {
            let n = reader.read_u8().await?;
            ensure!(n > 0, "Empty SOCKS hostname");
            result.push(n);
            n as usize
        }
        _ => bail!("Unsupported SOCKS address"),
    };
    let start = result.len();
    result.resize(start + n + 2, 0);
    reader.read_exact(&mut result[start..]).await?;
    Ok(result)
}
pub struct Bridge {
    pub endpoint: SocketAddr,
    pub password: String,
    cancel: CancellationToken,
    task: JoinHandle<()>,
    pub remote: Remote,
}
impl Bridge {
    pub async fn start(remote: Remote) -> Result<Self> {
        use std::io::Read;
        let mut bytes = [0; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
        let password: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let remote = remote.clone();
            let password = password.clone();
            let cancel = cancel.clone();
            async move {
                let mut sessions = JoinSet::new();
                loop {
                    tokio::select! {
                        _=cancel.cancelled()=>break,
                        _=sessions.join_next(),if !sessions.is_empty()=>(),
                        next=listener.accept()=>match next{Ok((local,_)) if sessions.len()<256=>{let remote=remote.clone();let password=password.clone();sessions.spawn(async move{let _=serve_client(local,remote,&password).await;});},Ok(_)=>(),Err(_)=>break}
                    }
                }
                sessions.abort_all();
                while sessions.join_next().await.is_some() {}
            }
        });
        Ok(Self {
            endpoint,
            password,
            cancel,
            task,
            remote,
        })
    }
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
    pub async fn close(mut self) {
        self.cancel.cancel();
        let _ = (&mut self.task).await;
    }
}
async fn serve_client(mut local: TcpStream, remote: Remote, password: &str) -> Result<()> {
    timeout(Duration::from_secs(10), async {
        ensure!(local.read_u8().await? == 5, "Invalid SOCKS version");
        let n = local.read_u8().await?;
        ensure!(n <= 16, "Too many methods");
        let mut methods = vec![0; n as usize];
        local.read_exact(&mut methods).await?;
        ensure!(methods.contains(&2), "Local proxy requires authentication");
        local.write_all(&[5, 2]).await?;
        ensure!(local.read_u8().await? == 1, "Invalid SOCKS auth version");
        let n = local.read_u8().await?;
        let mut user = vec![0; n as usize];
        local.read_exact(&mut user).await?;
        let n = local.read_u8().await?;
        let mut key = vec![0; n as usize];
        local.read_exact(&mut key).await?;
        let valid = user == b"tetherust" && bool::from(key.ct_eq(password.as_bytes()));
        local.write_all(&[1, if valid { 0 } else { 1 }]).await?;
        ensure!(valid, "Local proxy authentication failed");
        Ok::<(), anyhow::Error>(())
    })
    .await
    .context("Local proxy handshake timed out")??;
    let request = timeout(Duration::from_secs(5), read_request(&mut local)).await??;
    ensure!([1, 3].contains(&request[1]), "Unsupported SOCKS command");
    let mut upstream = remote.data_stream().await?;
    upstream.write_all(&request).await?;
    upstream.flush().await?;
    if request[1] == 1 {
        tokio::io::copy_bidirectional(&mut local, &mut upstream).await?;
        return Ok(());
    }
    let response = read_request(&mut upstream).await?;
    ensure!(response[1] == 0, "Phone rejected UDP association");
    let udp = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
    let port = udp.local_addr()?.port();
    let mut reply = vec![5, 0, 0, 1, 127, 0, 0, 1];
    reply.extend_from_slice(&port.to_be_bytes());
    local.write_all(&reply).await?;
    let (mut remote_read, mut remote_write) = tokio::io::split(upstream);
    let peer = Arc::new(tokio::sync::Mutex::new(None::<SocketAddr>));
    let mut tasks = JoinSet::<Result<()>>::new();
    tasks.spawn({
        let udp = udp.clone();
        let peer = peer.clone();
        async move {
            let mut buffer = vec![0; 65535];
            loop {
                let (n, source) = udp.recv_from(&mut buffer).await?;
                if !source.ip().is_loopback() || n < 8 || buffer[..3] != [0, 0, 0] {
                    continue;
                }
                let mut accepted = peer.lock().await;
                if accepted.is_none() {
                    *accepted = Some(source)
                }
                if *accepted != Some(source) {
                    continue;
                }
                drop(accepted);
                remote_write.write_u16(n as u16).await?;
                remote_write.write_all(&buffer[..n]).await?;
                remote_write.flush().await?;
            }
        }
    });
    tasks.spawn({
        let udp = udp.clone();
        let peer = peer.clone();
        async move {
            loop {
                let n = remote_read.read_u16().await? as usize;
                ensure!(n >= 8, "Invalid UDP frame");
                let mut bytes = vec![0; n];
                remote_read.read_exact(&mut bytes).await?;
                ensure!(bytes[..3] == [0, 0, 0], "Invalid UDP response");
                if let Some(peer) = *peer.lock().await {
                    udp.send_to(&bytes, peer).await?;
                }
            }
        }
    });
    tasks.spawn(async move {
        let _ = local.read_u8().await?;
        bail!("UDP control connection closed")
    });
    let result = tasks.join_next().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    if let Some(Ok(Err(error))) = result {
        return Err(error);
    }
    Ok(())
}

pub struct Link {
    pub bridge: Bridge,
    forward: Option<AdbForward>,
    pub path: PathStatus,
    pub strict_anchor: Option<StrictAnchor>,
    active: Option<zbus::zvariant::OwnedObjectPath>,
    tun_network: Option<crate::network::Network>,
    cancel: CancellationToken,
    tunnel: Option<JoinHandle<std::result::Result<usize, tun2proxy::Error>>>,
}
impl Link {
    pub async fn start(
        network: &crate::network::Network,
        data: &Path,
        serial: &str,
        full_system: bool,
        require_vpn: bool,
        anchor: Option<&StrictAnchor>,
        wifi_endpoint: Option<SocketAddr>,
    ) -> Result<(Self, crate::health::Health)> {
        ensure!(
            std::env::var_os("TETHERUST_ADB_DISABLED").is_none(),
            "Phone access disabled for this run"
        );
        let pairing = Pairing::load(data, serial)?;
        let forward = if wifi_endpoint.is_none() {
            Some(AdbForward::create(serial).await?)
        } else {
            None
        };
        let endpoint = if let Some(endpoint) = wifi_endpoint {
            ensure!(
                valid_wifi_endpoint(endpoint),
                "Wi-Fi requires a LAN endpoint"
            );
            endpoint
        } else {
            ([127, 0, 0, 1], forward.as_ref().expect("USB forward").port).into()
        };
        let setup = async {
            let remote = Remote::new(endpoint, pairing)?;
            let (remote, path) = remote.negotiate(require_vpn).await?;
            if let Some(anchor) = anchor {
                ensure!(
                    anchor.epoch == path.epoch,
                    "VPN network changed; click Disconnect then Connect to trust a new path"
                );
            }
            let health = verify_remote(&remote).await?;
            if let Some(anchor) = anchor {
                ensure!(
                    anchor.exit_ip == health.public_ip,
                    "VPN exit changed; explicit reconnect required"
                );
            }
            let bridge = Bridge::start(remote).await?;
            Ok::<_, anyhow::Error>((bridge, health, path))
        }
        .await;
        let (bridge, health, path) = match setup {
            Ok(value) => value,
            Err(error) => {
                if let Some(forward) = forward {
                    let _ = forward.close().await;
                }
                return Err(error);
            }
        };
        let cancel = CancellationToken::new();
        let mut link = Self {
            bridge,
            forward,
            strict_anchor: require_vpn.then(|| StrictAnchor {
                epoch: path.epoch,
                exit_ip: health.public_ip.clone(),
            }),
            path,
            active: None,
            tun_network: None,
            cancel,
            tunnel: None,
        };
        if full_system {
            // Separate bus lifetime: even a timed-out activation or failed cleanup cannot
            // leave a volatile TUN tied to the GUI's long-lived inventory connection.
            match crate::network::Network::open().await {
                Ok(tun_network) => link.tun_network = Some(tun_network),
                Err(error) => {
                    let _ = link.close(network).await;
                    return Err(error);
                }
            }
            match link
                .tun_network
                .as_ref()
                .expect("TUN bus")
                .activate_companion_tun()
                .await
            {
                Ok(active) => link.active = Some(active),
                Err(error) => {
                    if let Some(forward) = link.forward {
                        let _ = forward.close().await;
                    }
                    return Err(error);
                }
            };
            let opened = timeout(Duration::from_secs(12), async {
                loop {
                    if let Ok(device) = network.find(crate::tunnel::INTERFACE).await
                        && device.state == 100
                    {
                        return crate::tunnel::Tun::open();
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            })
            .await;
            let device = match opened {
                Ok(Ok(device)) => device,
                other => {
                    let _ = link.close(network).await;
                    return Err(match other {
                        Ok(Err(error)) => error,
                        _ => anyhow::anyhow!("NetworkManager TUN activation timed out"),
                    });
                }
            };
            let args = crate::tunnel::args(link.bridge.endpoint, &link.bridge.password)?;
            let cancel = link.cancel.clone();
            link.tunnel = Some(tokio::spawn(tun2proxy::run(device, 1500, args, cancel)));
            tokio::time::sleep(Duration::from_millis(50)).await;
            if link.finished() {
                let _ = link.close(network).await;
                bail!("TUN packet engine stopped during startup")
            }
        }
        Ok((link, health))
    }
    pub fn full_system(&self) -> bool {
        self.active.is_some()
    }
    pub fn finished(&self) -> bool {
        self.bridge.is_finished() || self.tunnel.as_ref().is_some_and(|task| task.is_finished())
    }
    pub async fn verify(&self) -> Result<crate::health::Health> {
        ensure!(!self.finished(), "Companion packet engine stopped");
        self.bridge.remote.check_path().await?;
        let health = verify_remote(&self.bridge.remote).await?;
        if let Some(anchor) = &self.strict_anchor {
            ensure!(
                health.public_ip == anchor.exit_ip,
                "VPN exit IP changed; forwarding stopped until explicit reconnect"
            );
        }
        Ok(health)
    }
    pub async fn close(mut self, network: &crate::network::Network) -> Result<()> {
        self.cancel.cancel();
        let result = if let Some(active) = &self.active {
            self.tun_network
                .as_ref()
                .unwrap_or(network)
                .deactivate_companion_tun(active)
                .await
        } else {
            Ok(())
        };
        if let Some(mut task) = self.tunnel.take()
            && timeout(Duration::from_secs(3), &mut task).await.is_err()
        {
            task.abort();
        }
        self.bridge.close().await;
        let forward = if let Some(forward) = self.forward {
            forward.close().await
        } else {
            Ok(())
        };
        drop(self.tun_network.take());
        result?;
        forward?;
        Ok(())
    }
}
pub async fn verify_remote(remote: &Remote) -> Result<crate::health::Health> {
    timeout(Duration::from_secs(30), async {
        let start = std::time::Instant::now();
        let phone_ip = remote.phone_exit().await?;
        let stream = remote.connect_tcp("one.one.one.one", 443).await?;
        let public_ip = crate::health::trace(stream).await?;
        ensure!(
            public_ip.parse::<std::net::IpAddr>()? == phone_ip,
            "Phone-side and Linux forwarding HTTPS exit IPs differ; cannot verify this path"
        );
        Ok(crate::health::Health {
            public_ip,
            latency_ms: start.elapsed().as_millis() as u64,
            dns_ms: 0,
            address_family: "Companion TLS / app network".into(),
            preferred_route: false,
        })
    })
    .await
    .context("Phone effective internet verification timed out")?
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}
