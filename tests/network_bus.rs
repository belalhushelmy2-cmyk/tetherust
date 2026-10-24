//! Run explicitly under dbus-run-session; never targets the system bus or real NetworkManager.
use std::{
    collections::HashMap,
    fs,
    os::unix::fs::symlink,
    sync::{Arc, Mutex},
};
use tetherust::{device::Transport, network::Network};
use zbus::{
    Connection,
    zvariant::{OwnedObjectPath, OwnedValue},
};
const ROOT: &str = "/org/freedesktop/NetworkManager";
const DEVICE: &str = "/org/freedesktop/NetworkManager/Devices/1";
const CHECKPOINT: &str = "/org/freedesktop/NetworkManager/Checkpoint/1";
const ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
fn path(s: &str) -> OwnedObjectPath {
    s.try_into().unwrap()
}
type Settings = HashMap<String, HashMap<String, OwnedValue>>;
#[derive(Default)]
struct State {
    calls: Vec<String>,
    activation_fails: bool,
    rollback_code: u32,
    tun_active: bool,
}
#[derive(Clone)]
struct Manager(Arc<Mutex<State>>);
#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    #[zbus(property)]
    fn version(&self) -> &str {
        "1.54.0-test"
    }
    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        vec![path(DEVICE)]
    }
    fn checkpoint_create(
        &self,
        devices: Vec<OwnedObjectPath>,
        timeout: u32,
        flags: u32,
    ) -> OwnedObjectPath {
        assert_eq!(devices, vec![path(DEVICE)]);
        assert_eq!(timeout, 90);
        assert_eq!(flags, 0);
        self.0.lock().unwrap().calls.push("checkpoint".into());
        path(CHECKPOINT)
    }
    fn add_and_activate_connection2(
        &self,
        settings: Settings,
        device: OwnedObjectPath,
        specific: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<(
        OwnedObjectPath,
        OwnedObjectPath,
        HashMap<String, OwnedValue>,
    )> {
        if <&str>::try_from(&settings["connection"]["type"]).unwrap() == "tun" {
            assert_eq!(device, path("/"));
            assert_eq!(specific, path("/"));
            assert_eq!(<&str>::try_from(&options["persist"]).unwrap(), "volatile");
            assert_eq!(
                <&str>::try_from(&options["bind-activation"]).unwrap(),
                "dbus-client"
            );
            assert_eq!(
                <&str>::try_from(&settings["tun"]["owner"]).unwrap(),
                unsafe { libc::getuid() }.to_string()
            );
            assert!(settings["ipv4"].contains_key("route-data"));
            assert!(settings["ipv6"].contains_key("route-data"));
            assert!(settings["ipv4"].contains_key("dns"));
            let mut state = self.0.lock().unwrap();
            state.calls.push("activate-tun".into());
            state.tun_active = true;
            return Ok((
                path("/org/freedesktop/NetworkManager/Settings/2"),
                path(ACTIVE),
                HashMap::new(),
            ));
        }
        assert_eq!(device, path(DEVICE));
        assert_eq!(specific, path("/"));
        assert_eq!(<&str>::try_from(&options["persist"]).unwrap(), "volatile");
        assert_eq!(
            <&str>::try_from(&options["bind-activation"]).unwrap(),
            "dbus-client"
        );
        assert_eq!(
            <&str>::try_from(&settings["connection"]["interface-name"]).unwrap(),
            "usb0"
        );
        assert!(!bool::try_from(&settings["connection"]["autoconnect"]).unwrap());
        assert!(!bool::try_from(&settings["ipv4"]["never-default"]).unwrap());
        let mut state = self.0.lock().unwrap();
        state.calls.push("activate".into());
        if state.activation_fails {
            return Err(zbus::fdo::Error::AccessDenied(
                "simulated Polkit denial".into(),
            ));
        }
        Ok((
            path("/org/freedesktop/NetworkManager/Settings/1"),
            path(ACTIVE),
            HashMap::new(),
        ))
    }
    fn checkpoint_adjust_rollback_timeout(&self, checkpoint: OwnedObjectPath, timeout: u32) {
        assert_eq!(checkpoint, path(CHECKPOINT));
        assert_eq!(timeout, 90);
        self.0.lock().unwrap().calls.push("renew".into());
    }
    fn deactivate_connection(&self, active: OwnedObjectPath) -> zbus::fdo::Result<()> {
        assert_eq!(active, path(ACTIVE));
        {
            let mut state = self.0.lock().unwrap();
            state.calls.push("deactivate".into());
            if state.tun_active {
                state.tun_active = false;
                return Ok(());
            }
        }
        // Simulates removal before explicit disconnect. Rollback must still run.
        Err(zbus::fdo::Error::UnknownObject("unplugged".into()))
    }
    fn checkpoint_rollback(&self, checkpoint: OwnedObjectPath) -> HashMap<String, u32> {
        assert_eq!(checkpoint, path(CHECKPOINT));
        let mut state = self.0.lock().unwrap();
        state.calls.push("rollback".into());
        HashMap::from([(DEVICE.into(), state.rollback_code)])
    }
}
struct Device;
#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    #[zbus(property)]
    fn interface(&self) -> &str {
        "usb0"
    }
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        1
    }
    #[zbus(property)]
    fn driver(&self) -> &str {
        "rndis_host"
    }
    #[zbus(property)]
    fn state(&self) -> u32 {
        30
    }
    #[zbus(property)]
    fn managed(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn ip4_config(&self) -> OwnedObjectPath {
        path("/")
    }
    #[zbus(property)]
    fn ip6_config(&self) -> OwnedObjectPath {
        path("/")
    }
}
#[tokio::test]
#[ignore = "requires a private session bus; run dbus-run-session -- cargo test --test network_bus -- --ignored"]
async fn native_dbus_activation_and_failure_recovery_contract() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let usb = root.join("devices/usb1/1-2");
    let iface = usb.join("1-2:1.0");
    fs::create_dir_all(&iface).unwrap();
    fs::create_dir_all(root.join("net/usb0")).unwrap();
    fs::create_dir_all(root.join("drivers/rndis_host")).unwrap();
    fs::write(usb.join("idVendor"), "18d1").unwrap();
    fs::write(usb.join("idProduct"), "4ee3").unwrap();
    fs::write(usb.join("product"), "Test phone").unwrap();
    symlink(root.join("drivers/rndis_host"), iface.join("driver")).unwrap();
    symlink(&iface, root.join("net/usb0/device")).unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let _server = zbus::connection::Builder::session()
        .unwrap()
        .name("org.freedesktop.NetworkManager")
        .unwrap()
        .serve_at(ROOT, Manager(state.clone()))
        .unwrap()
        .serve_at(DEVICE, Device)
        .unwrap()
        .build()
        .await
        .unwrap();
    let network = Network::on_bus(Connection::session().await.unwrap(), root.join("net"));
    assert_eq!(network.version().await.unwrap(), "1.54.0-test");
    let device = network.find("usb0").await.unwrap();
    assert_eq!(device.transport, Transport::Usb);
    let owned = network.activate_usb(&device, true).await.unwrap();
    network.renew(&owned).await.unwrap();
    network.release(&owned).await.unwrap();
    assert_eq!(
        state.lock().unwrap().calls,
        ["checkpoint", "activate", "renew", "deactivate", "rollback"]
    );
    {
        let mut s = state.lock().unwrap();
        s.calls.clear();
        s.activation_fails = true;
    }
    assert!(network.activate_usb(&device, true).await.is_err());
    assert_eq!(
        state.lock().unwrap().calls,
        ["checkpoint", "activate", "rollback"]
    );
    state.lock().unwrap().rollback_code = 3;
    assert!(network.rollback(&path(CHECKPOINT)).await.is_err());
    assert!(network.find("../../etc").await.is_err());
    // Removal of USB ancestry must prevent activation before a checkpoint is made.
    fs::remove_file(usb.join("idVendor")).unwrap();
    state.lock().unwrap().calls.clear();
    assert!(network.activate_usb(&device, true).await.is_err());
    assert!(state.lock().unwrap().calls.is_empty());

    // A failed initial connect must never claim internet; Disconnect cancels the retry state.
    let (commands, input) = tokio::sync::mpsc::channel(8);
    let (output, mut snapshots) =
        tokio::sync::watch::channel(tetherust::session::Snapshot::default());
    let worker = tokio::spawn(
        tetherust::session::Controller::new(
            network.clone(),
            tetherust::config::Config::default(),
            root.into(),
        )
        .run(input, output),
    );
    commands
        .send(tetherust::session::Command::Connect("missing0".into()))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        while snapshots.changed().await.is_ok() {
            let current = snapshots.borrow_and_update().clone();
            assert!(current.health.is_none());
            assert_ne!(current.status, "Connected");
            if current.status == "Waiting to reconnect" {
                break;
            }
        }
    })
    .await
    .unwrap();
    commands
        .send(tetherust::session::Command::Disconnect)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        while snapshots.changed().await.is_ok() {
            if snapshots.borrow_and_update().status == "Disconnected" {
                break;
            }
        }
    })
    .await
    .unwrap();
    commands
        .send(tetherust::session::Command::Shutdown)
        .await
        .unwrap();
    worker.await.unwrap();
    assert!(state.lock().unwrap().calls.is_empty());

    // A new software TUN uses an explicit user owner, volatile bus lifetime and per-profile DNS/routes.
    let active = network.activate_companion_tun().await.unwrap();
    network.deactivate_companion_tun(&active).await.unwrap();
    assert_eq!(state.lock().unwrap().calls, ["activate-tun", "deactivate"]);
}
