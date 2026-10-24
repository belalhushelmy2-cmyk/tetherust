use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    time::Duration,
};
use tetherust::{
    config::{Config, autostart},
    device::{counters_at, usb_identity, valid_interface},
    health::{dns_query, parse_dns},
    network::usb_settings,
    session::backoff,
    storage::{self, History, InstanceLock},
};

#[test]
fn usb_detection_requires_supported_driver_and_usb_ancestry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let usb = root.join("devices/usb1/1-2");
    let interface = usb.join("1-2:1.0");
    let net = root.join("net/usb0");
    fs::create_dir_all(&interface).unwrap();
    fs::create_dir_all(&net).unwrap();
    fs::create_dir_all(root.join("drivers/rndis_host")).unwrap();
    symlink(root.join("drivers/rndis_host"), interface.join("driver")).unwrap();
    symlink(&interface, net.join("device")).unwrap();
    assert!(usb_identity(&root.join("net"), "usb0").is_none());
    fs::write(usb.join("idVendor"), "18d1").unwrap();
    fs::write(usb.join("idProduct"), "4ee3").unwrap();
    fs::write(usb.join("product"), "Android\nPhone\n").unwrap();
    assert_eq!(
        usb_identity(&root.join("net"), "usb0").unwrap(),
        ("AndroidPhone".into(), "rndis_host".into())
    );
    fs::remove_file(interface.join("driver")).unwrap();
    fs::create_dir_all(root.join("drivers/r8152")).unwrap();
    symlink(root.join("drivers/r8152"), interface.join("driver")).unwrap();
    assert!(usb_identity(&root.join("net"), "usb0").is_none());
}
#[test]
fn interface_validation_prevents_path_and_control_characters() {
    for value in [
        "../wlan0",
        "",
        "../../etc",
        "/dev/null",
        "wlan0\n",
        "a b",
        ";rm",
        "abcdefghijklmnop",
        ".",
        "..",
    ] {
        assert!(!valid_interface(value), "{value}");
    }
    for value in ["usb0", "enp0s20u1", "p2p-wlp3s0-0", "bt-pan.1"] {
        assert!(valid_interface(value));
    }
}
#[test]
fn counter_reads_reject_invalid_names_and_parse_u64() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("usb0/statistics")).unwrap();
    fs::write(tmp.path().join("usb0/statistics/rx_bytes"), "4294967297\n").unwrap();
    fs::write(tmp.path().join("usb0/statistics/tx_bytes"), "12\n").unwrap();
    assert_eq!(counters_at(tmp.path(), "usb0"), Some((4294967297, 12)));
    assert_eq!(counters_at(tmp.path(), "../usb0"), None);
}
fn dns_response(query: &[u8]) -> Vec<u8> {
    let mut packet = query.to_vec();
    packet[2] = 0x81;
    packet[3] = 0x80;
    packet[7] = 1;
    packet.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 1, 1, 1]);
    packet
}
#[test]
fn dns_verification_rejects_spoofed_truncated_and_mismatched_answers() {
    let query = dns_query([0x12, 0x34], 1);
    let response = dns_response(&query);
    assert_eq!(
        parse_dns(&query, &response).unwrap()[0].to_string(),
        "1.1.1.1:443"
    );
    for len in 0..response.len() {
        assert!(
            parse_dns(&query, &response[..len]).is_err(),
            "truncation {len}"
        );
    }
    for (at, value) in [(0, 0), (2, 0x83), (3, 0x83), (5, 2), (13, b'x')] {
        let mut bad = response.clone();
        bad[at] = value;
        assert!(parse_dns(&query, &bad).is_err(), "offset {at}");
    }
    let mut loopback = response;
    let n = loopback.len();
    loopback[n - 4..].copy_from_slice(&[127, 0, 0, 1]);
    assert!(parse_dns(&query, &loopback).is_err());
}
#[test]
fn malformed_dns_packets_never_panic() {
    let query = dns_query([1, 2], 1);
    for length in 0..4096 {
        let bytes = vec![length as u8; length];
        let _ = parse_dns(&query, &bytes);
    }
}
#[test]
fn route_and_dns_changes_require_explicit_full_system_preference() {
    for full in [false, true] {
        let settings = usb_settings("usb0", full);
        let conn = &settings["connection"];
        assert!(!bool::try_from(&conn["autoconnect"]).unwrap());
        for family in ["ipv4", "ipv6"] {
            let ip = &settings[family];
            assert!(!bool::try_from(&ip["never-default"]).unwrap());
            assert_eq!(
                i32::try_from(&ip["dns-priority"]).unwrap(),
                if full { -100 } else { 60000 }
            );
            assert_eq!(
                i64::try_from(&ip["route-metric"]).unwrap(),
                if full { 20 } else { 60000 }
            );
            assert_eq!(ip.contains_key("dns-search"), full);
            assert!(!ip.contains_key("mtu"));
        }
    }
}
#[test]
fn config_defaults_safe_roundtrip_and_invalid_json_is_not_silenced() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.json");
    let mut cfg = Config::load(&path).unwrap();
    assert!(!cfg.auto_connect && !cfg.full_system);
    cfg.auto_reconnect = false;
    cfg.save(&path).unwrap();
    assert!(!Config::load(&path).unwrap().auto_reconnect);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::write(&path, "{\"unknown\": true}").unwrap();
    assert!(Config::load(&path).is_err());
}
#[test]
fn autostart_quotes_exec_and_uninstall_only_removes_owned_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("autostart/tetherust.desktop");
    autostart(&path, std::path::Path::new("/tmp/a b/%foo$bar"), true).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("Exec=\"/tmp/a b/%%foo\\$bar\" gui"));
    assert!(autostart(&path, std::path::Path::new("/tmp/evil\nExec=bad"), true).is_err());
    autostart(&path, std::path::Path::new("/unused"), false).unwrap();
    assert!(!path.exists());
}
#[test]
fn duplicate_instances_rejected_and_lock_recovers_after_drop() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("instance.lock");
    let lock = InstanceLock::acquire(&path).unwrap();
    assert!(InstanceLock::acquire(&path).is_err());
    drop(lock);
    assert!(InstanceLock::acquire(&path).is_ok());
}
#[test]
fn log_exports_are_private_and_never_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("logs.txt");
    storage::export_logs(&path, &["internet verification passed".into()]).unwrap();
    assert!(storage::export_logs(&path, &[]).is_err());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
#[test]
fn history_is_bounded_and_omits_device_identifiers() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("history.jsonl");
    for i in 0..110 {
        storage::append_history(
            &path,
            &History {
                started_unix: i,
                seconds: 5,
                received: 20,
                sent: 10,
                verified: false,
            },
        )
        .unwrap();
    }
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 100);
    assert!(!text.contains("interface"));
    assert!(text.lines().next().unwrap().contains("\"started_unix\":10"));
}
#[test]
fn reconnect_backoff_is_bounded_even_at_u32_max() {
    assert_eq!(backoff(0), Duration::from_secs(1));
    assert_eq!(backoff(4), Duration::from_secs(16));
    assert_eq!(backoff(5), Duration::from_secs(32));
    assert_eq!(backoff(u32::MAX), Duration::from_secs(32));
}
