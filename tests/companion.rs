use std::{net::SocketAddr, time::Duration};
use tetherust::companion::{self, Bridge, Pairing, Remote};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    time::timeout,
};
#[test]
fn adb_inventory_includes_authorized_and_blocked_phones_without_shell_commands() {
    let devices=companion::parse_devices(b"List of devices attached\nABC123 device usb:1-2 model:motorola_edge_60_fusion\nXYZ unauthorized usb:1-3\n../bad device\n");
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].name, "motorola edge 60 fusion");
    assert!(devices[0].authorized);
    assert!(!devices[1].authorized);
    for bad in ["", "../x", "a/b", "x;id", "serial\n", "x$y"] {
        assert!(!companion::valid_serial(bad));
    }
}
#[test]
fn pairing_rejects_malformed_secrets_and_is_private() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(Pairing::load(tmp.path(), "../bad").is_err());
    let pair = Pairing {
        version: 1,
        name: "Phone".into(),
        token: "00".repeat(32),
        certificate: "invalid base64".into(),
    };
    assert!(pair.validate().is_err());
}
#[test]
fn companion_tun_is_owned_by_this_user_and_has_scoped_routes() {
    let settings = tetherust::network::companion_tun_settings();
    assert_eq!(
        <&str>::try_from(&settings["connection"]["interface-name"]).unwrap(),
        "tetherust0"
    );
    assert!(!bool::try_from(&settings["connection"]["autoconnect"]).unwrap());
    assert_eq!(
        <&str>::try_from(&settings["tun"]["owner"]).unwrap(),
        unsafe { libc::getuid() }.to_string()
    );
    assert_eq!(
        <&str>::try_from(&settings["tun"]["group"]).unwrap(),
        unsafe { libc::getgid() }.to_string()
    );
    for family in ["ipv4", "ipv6"] {
        assert!(settings[family].contains_key("route-data"));
        assert_eq!(
            i32::try_from(&settings[family]["dns-priority"]).unwrap(),
            -100
        );
    }
    let args =
        tetherust::tunnel::args("127.0.0.1:1234".parse().unwrap(), &"ab".repeat(32)).unwrap();
    assert!(!args.setup);
    assert!(args.ipv6_enabled);
    assert!(matches!(args.dns, tun2proxy::ArgDns::Virtual));
    assert!(tetherust::tunnel::args("192.0.2.1:1234".parse().unwrap(), "ab").is_err());
}
async fn local_socks(
    endpoint: SocketAddr,
    password: &str,
    command: u8,
    host: &str,
    port: u16,
) -> anyhow::Result<(TcpStream, Vec<u8>)> {
    let mut stream = TcpStream::connect(endpoint).await?;
    stream.write_all(&[5, 1, 2]).await?;
    let mut reply = [0; 2];
    stream.read_exact(&mut reply).await?;
    assert_eq!(reply, [5, 2]);
    stream.write_all(&[1, 9]).await?;
    stream.write_all(b"tetherust").await?;
    stream.write_u8(password.len() as u8).await?;
    stream.write_all(password.as_bytes()).await?;
    stream.read_exact(&mut reply).await?;
    anyhow::ensure!(reply == [1, 0], "Authentication refused");
    stream
        .write_all(&[5, command, 0, 3, host.len() as u8])
        .await?;
    stream.write_all(host.as_bytes()).await?;
    stream.write_u16(port).await?;
    let reply = companion::read_request(&mut stream).await?;
    anyhow::ensure!(reply[1] == 0, "SOCKS failed");
    Ok((stream, reply))
}
#[tokio::test]
#[ignore = "run tests/companion_host.py to create an isolated Kotlin TLS fixture; never uses a phone"]
async fn kotlin_and_rust_forward_tcp_udp_with_pinned_tls_and_authentication() {
    timeout(Duration::from_secs(30), async {
        let path = std::env::var("TETHERUST_PROXY_FIXTURE").unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let pairing = Pairing {
            version: 1,
            name: "fixture".into(),
            token: json["token"].as_str().unwrap().into(),
            certificate: json["certificate"].as_str().unwrap().into(),
        };
        let tls_port = json["tls_port"].as_u64().unwrap() as u16;
        let tcp_port = json["tcp_port"].as_u64().unwrap() as u16;
        let udp_port = json["udp_port"].as_u64().unwrap() as u16;
        let remote = Remote::new(([127, 0, 0, 1], tls_port).into(), pairing.clone()).unwrap();
        let (remote, path_status) = remote.negotiate(true).await.unwrap();
        assert!(path_status.vpn && path_status.ipv4_default);
        assert_eq!(
            remote.phone_exit().await.unwrap().to_string(),
            "203.0.113.123"
        );
        let state_path = format!("{path}.state");
        for mode in ["excluded", "split", "blocked"] {
            std::fs::write(&state_path, mode).unwrap();
            tokio::time::sleep(Duration::from_millis(80)).await;
            assert!(
                Remote::new(remote.endpoint, pairing.clone())
                    .unwrap()
                    .negotiate(true)
                    .await
                    .is_err()
            );
        }
        std::fs::write(&state_path, "vpn").unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        let marker = "TLS through the actual Kotlin forwarding engine مرحبا"
            .repeat(4000)
            .into_bytes();
        let mut direct = remote.connect_tcp("echo.test", tcp_port).await.unwrap();
        direct.write_all(&marker).await.unwrap();
        let mut received = vec![0; marker.len()];
        direct.read_exact(&mut received).await.unwrap();
        assert_eq!(received, marker);
        drop(direct);
        let mut wrong = pairing.clone();
        wrong.token = "ff".repeat(32);
        assert!(
            Remote::new(remote.endpoint, wrong)
                .unwrap()
                .negotiate(true)
                .await
                .is_err()
        );
        let mut wrong = pairing.clone();
        use base64::Engine;
        let mut cert = base64::engine::general_purpose::STANDARD
            .decode(&wrong.certificate)
            .unwrap();
        cert[30] ^= 1;
        wrong.certificate = base64::engine::general_purpose::STANDARD.encode(cert);
        assert!(
            Remote::new(remote.endpoint, wrong)
                .unwrap()
                .negotiate(true)
                .await
                .is_err()
        );
        let bridge = Bridge::start(remote).await.unwrap();
        assert!(
            local_socks(bridge.endpoint, "wrong", 1, "echo.test", tcp_port)
                .await
                .is_err()
        );
        let (mut tcp, _) = local_socks(bridge.endpoint, &bridge.password, 1, "echo.test", tcp_port)
            .await
            .unwrap();
        tcp.write_all(b"SOCKS TCP echo").await.unwrap();
        let mut data = [0; 14];
        tcp.read_exact(&mut data).await.unwrap();
        assert_eq!(&data, b"SOCKS TCP echo");
        drop(tcp);
        let (control, reply) = local_socks(bridge.endpoint, &bridge.password, 3, "0.0.0.0", 0)
            .await
            .unwrap();
        let udp_target = SocketAddr::from((
            [127, 0, 0, 1],
            u16::from_be_bytes([reply[reply.len() - 2], reply[reply.len() - 1]]),
        ));
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut frame = vec![0, 0, 0, 3, 9];
        frame.extend_from_slice(b"echo.test");
        frame.extend_from_slice(&udp_port.to_be_bytes());
        frame.extend_from_slice(b"UDP over pinned TLS");
        for _ in 0..20 {
            udp.send_to(&frame, udp_target).await.unwrap();
            let mut data = [0; 4096];
            let (n, _) = udp.recv_from(&mut data).await.unwrap();
            assert_eq!(&data[..3], &[0, 0, 0]);
            assert!(data[..n].ends_with(b"UDP over pinned TLS"));
        }
        let mut fragmented = frame.clone();
        fragmented[2] = 1;
        udp.send_to(&fragmented, udp_target).await.unwrap();
        let mut data = [0; 4096];
        assert!(
            timeout(Duration::from_millis(150), udp.recv_from(&mut data))
                .await
                .is_err()
        );
        drop(control);
        // A network transition closes existing streams as well as rejecting stale new ones.
        let mut held=bridge.remote.connect_tcp("echo.test",tcp_port).await.unwrap();
        held.write_all(b"x").await.unwrap(); let mut one=[0]; held.read_exact(&mut one).await.unwrap();
        std::fs::write(&state_path, "changed").unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!matches!(timeout(Duration::from_secs(2), held.read(&mut one)).await.unwrap(), Ok(n) if n > 0));
        assert!(bridge.remote.check_path().await.is_err());
        assert!(
            bridge
                .remote
                .connect_tcp("echo.test", tcp_port)
                .await
                .is_err()
        );
        bridge.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn packet_engine_answers_dns_from_raw_tun_packets_without_network_changes() {
    let query = tetherust::health::dns_query([0x12, 0x34], 1);
    let length = (28 + query.len()) as u16;
    let mut packet = vec![
        0x45,
        0,
        (length >> 8) as u8,
        length as u8,
        0,
        1,
        0,
        0,
        64,
        17,
        0,
        0,
        10,
        0,
        0,
        2,
        198,
        18,
        0,
        1,
    ];
    let sum: u32 = packet
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
        .sum();
    let mut sum = (sum & 0xffff) + (sum >> 16);
    sum = (sum & 0xffff) + (sum >> 16);
    let checksum = !(sum as u16);
    packet[10..12].copy_from_slice(&checksum.to_be_bytes());
    packet.extend_from_slice(&52001u16.to_be_bytes());
    packet.extend_from_slice(&53u16.to_be_bytes());
    packet.extend_from_slice(&((query.len() + 8) as u16).to_be_bytes());
    packet.extend_from_slice(&[0, 0]);
    packet.extend_from_slice(&query);
    let (device, mut host) = tokio::io::duplex(65536);
    let cancel = tun2proxy::CancellationToken::new();
    let args = tetherust::tunnel::args("127.0.0.1:9".parse().unwrap(), "ab").unwrap();
    let task = tokio::spawn(tun2proxy::run(device, 1500, args, cancel.clone()));
    host.write_all(&packet).await.unwrap();
    let mut header = [0; 20];
    timeout(Duration::from_secs(5), host.read_exact(&mut header))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(header[9], 17);
    let length = u16::from_be_bytes([header[2], header[3]]) as usize;
    assert!(length > 28);
    let mut rest = vec![0; length - 20];
    host.read_exact(&mut rest).await.unwrap();
    assert_eq!(&rest[..2], &53u16.to_be_bytes());
    let resolved = tetherust::health::parse_dns(&query, &rest[8..]).unwrap();
    assert!(resolved[0].ip().to_string().starts_with("198.18."));
    cancel.cancel();
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[test]
fn strict_policy_rejects_exclusion_split_routes_blocked_network_and_old_protocol() {
    let base = companion::PathStatus {
        protocol: 2,
        epoch: 7,
        vpn: true,
        ipv4_default: true,
        ipv6_default: false,
        blocked: false,
        available: true,
        mode: "default".into(),
        udp_supported: true,
    };
    assert!(base.validate(true).is_ok());
    for field in [
        "vpn",
        "routes",
        "blocked",
        "unavailable",
        "proxy",
        "protocol",
    ] {
        let mut bad = base.clone();
        match field {
            "vpn" => bad.vpn = false,
            "routes" => bad.ipv4_default = false,
            "blocked" => bad.blocked = true,
            "unavailable" => bad.available = false,
            "proxy" => bad.mode = "http".into(),
            _ => bad.protocol = 1,
        }
        assert!(bad.validate(true).is_err(), "{field}");
    }
    let mut direct = base;
    direct.vpn = false;
    direct.ipv4_default = false;
    assert!(direct.validate(false).is_ok());
}
#[test]
fn wifi_discovery_is_untrusted_and_only_accepts_numeric_lan_endpoints() {
    let endpoints=companion::parse_discovery(b"=;eth0;IPv4;Phone;_tetherust._tcp;local;phone.local;192.168.1.2;38473;\n=;eth0;IPv4;Spoof;_tetherust._tcp;local;evil;8.8.8.8;443;\n=;eth0;IPv6;Phone;_tetherust._tcp;local;phone.local;fd23::2;38473;\n");
    assert_eq!(endpoints.len(), 2);
    assert!(!companion::valid_wifi_endpoint(
        "127.0.0.1:38473".parse().unwrap()
    ));
    assert!(!companion::valid_wifi_endpoint(
        "192.168.1.2:0".parse().unwrap()
    ));
    let tmp = tempfile::tempdir().unwrap();
    let config = tetherust::config::Config {
        wifi_endpoint: "8.8.8.8:443".into(),
        ..Default::default()
    };
    assert!(config.save(&tmp.path().join("config.json")).is_err());
}
