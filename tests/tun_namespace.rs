//! Requires tests/network_namespace.sh. Never runs against the host bus/network namespace.
use std::{os::unix::fs::MetadataExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    time::timeout,
};
#[tokio::test]
#[ignore = "requires a private user/network namespace and isolated real NetworkManager; run tests/network_namespace.sh"]
async fn real_networkmanager_owns_tun_routes_and_packet_engine_dns() {
    let original = std::env::var("TETHERUST_PARENT_NETNS").expect("namespace harness required");
    assert_ne!(
        std::fs::metadata("/proc/self/ns/net")
            .unwrap()
            .ino()
            .to_string(),
        original,
        "Never operate on the host network namespace"
    );
    let bus = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap();
    assert!(bus.starts_with("unix:path=/tmp/tetherust-nm-"));
    let owner = tetherust::network::Network::open().await.unwrap();
    let active = owner.activate_companion_tun().await.unwrap();
    let device = timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(device) = owner.find("tetherust0").await
                && device.state == 100
            {
                break device;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert!(device.addresses.contains(&"198.19.255.254".into()));
    assert!(device.dns.contains(&"198.18.0.1".into()));
    assert!(
        tetherust::health::preferred_route("tetherust0", "1.1.1.1".parse().unwrap())
            .await
            .unwrap()
    );
    // The harness drops capabilities for the entire test process before launching it.
    for path in ["/proc/self/status", "/proc/thread-self/status"] {
        let status = std::fs::read_to_string(path).unwrap();
        assert!(
            status
                .lines()
                .any(|line| line == "CapEff:\t0000000000000000")
        );
        assert!(
            status
                .lines()
                .any(|line| line == "CapPrm:\t0000000000000000")
        );
    }
    let tun = tetherust::tunnel::Tun::open().unwrap();
    let cancel = tun2proxy::CancellationToken::new();
    let fixture = std::env::var("TETHERUST_PROXY_FIXTURE").ok().map(|path| {
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap()
    });
    let bridge = if let Some(fixture) = &fixture {
        let pair = tetherust::companion::Pairing {
            version: 1,
            name: "private kernel fixture".into(),
            token: fixture["token"].as_str().unwrap().into(),
            certificate: fixture["certificate"].as_str().unwrap().into(),
        };
        let remote = tetherust::companion::Remote::new(
            ([127, 0, 0, 1], fixture["tls_port"].as_u64().unwrap() as u16).into(),
            pair,
        )
        .unwrap();
        let (remote, _) = remote.negotiate(true).await.unwrap();
        Some(tetherust::companion::Bridge::start(remote).await.unwrap())
    } else {
        None
    };
    let args = if let Some(bridge) = &bridge {
        tetherust::tunnel::args(bridge.endpoint, &bridge.password)
    } else {
        tetherust::tunnel::args("127.0.0.1:9".parse().unwrap(), "ab")
    }
    .unwrap();
    let task = tokio::spawn(tun2proxy::run(tun, 1500, args, cancel.clone()));
    let socket = UdpSocket::bind("0.0.0.0:0").await.unwrap();
    socket.connect("198.18.0.1:53").await.unwrap();
    let mut query = tetherust::health::dns_query([0x55, 0x33], 1)[..12].to_vec();
    query.extend_from_slice(b"\x04echo\x04test\x00\x00\x01\x00\x01");
    socket.send(&query).await.unwrap();
    let mut response = [0; 4096];
    let n = timeout(Duration::from_secs(5), socket.recv(&mut response))
        .await
        .unwrap()
        .unwrap();
    let addresses = tetherust::health::parse_dns(&query, &response[..n]).unwrap();
    assert!(addresses[0].ip().to_string().starts_with("198.18."));
    if let Some(fixture) = &fixture {
        // Ordinary application sockets: no SO_BINDTODEVICE and no proxy configuration.
        let tcp_port = fixture["tcp_port"].as_u64().unwrap() as u16;
        let udp_port = fixture["udp_port"].as_u64().unwrap() as u16;
        let target = addresses[0].ip();
        let mut application = timeout(
            Duration::from_secs(5),
            TcpStream::connect((target, tcp_port)),
        )
        .await
        .unwrap()
        .unwrap();
        let data = "ordinary application → kernel TUN → Rust bridge → pinned TLS Kotlin engine"
            .repeat(400)
            .into_bytes();
        application.write_all(&data).await.unwrap();
        let mut echo = vec![0; data.len()];
        timeout(Duration::from_secs(8), application.read_exact(&mut echo))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(echo, data);
        drop(application);
        let application = UdpSocket::bind("0.0.0.0:0").await.unwrap();
        application.connect((target, udp_port)).await.unwrap();
        for _ in 0..5 {
            application.send(b"kernel UDP path").await.unwrap();
            let mut echo = [0; 128];
            let n = timeout(Duration::from_secs(5), application.recv(&mut echo))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&echo[..n], b"kernel UDP path");
        }
        println!(
            "PASS ordinary TCP/UDP application sockets through real kernel TUN, virtual DNS, Rust bridge and Kotlin TLS gateway; zero capabilities, no phone"
        );
    }
    cancel.cancel();
    timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    if let Some(bridge) = bridge {
        bridge.close().await;
    }
    owner.deactivate_companion_tun(&active).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !tetherust::health::preferred_route("tetherust0", "1.1.1.1".parse().unwrap())
            .await
            .unwrap_or(false)
    );

    // Reconnect must succeed after cleanup; losing the dedicated bus must also remove routes.
    let second = tetherust::network::Network::open().await.unwrap();
    let _active = second.activate_companion_tun().await.unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            if tetherust::health::preferred_route("tetherust0", "1.1.1.1".parse().unwrap())
                .await
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    drop(second);
    timeout(Duration::from_secs(10), async {
        loop {
            if !tetherust::health::preferred_route("tetherust0", "1.1.1.1".parse().unwrap())
                .await
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
}
