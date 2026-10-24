use crate::device::{Device, Transport, usb_identity, valid_interface};
use anyhow::{Context, Result, bail};
use std::{collections::HashMap, path::PathBuf};
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const DEST: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
type Settings = HashMap<String, HashMap<String, OwnedValue>>;
#[derive(Clone)]
pub struct Network {
    pub connection: Connection,
    sysfs_root: PathBuf,
    pub(crate) physical_inventory: bool,
}
#[derive(Debug)]
pub struct OwnedConnection {
    pub active: OwnedObjectPath,
    pub checkpoint: OwnedObjectPath,
}
fn string(s: &str) -> OwnedValue {
    Value::from(s).try_to_owned().expect("string value")
}
impl Network {
    /// Bind a supplied bus and sysfs root (also used by isolated integration tests).
    pub fn on_bus(connection: Connection, sysfs_root: PathBuf) -> Self {
        Self {
            connection,
            sysfs_root,
            physical_inventory: false,
        }
    }
    pub async fn open() -> Result<Self> {
        let connection = zbus::connection::Builder::system()?
            .method_timeout(std::time::Duration::from_secs(25))
            .build()
            .await
            .context("Cannot reach system D-Bus. NetworkManager is required")?;
        let mut this = Self::on_bus(connection, "/sys/class/net".into());
        this.physical_inventory = true;
        let _: String = this
            .manager()
            .await?
            .get_property("Version")
            .await
            .context("NetworkManager is unavailable")?;
        Ok(this)
    }
    pub async fn manager(&self) -> Result<Proxy<'_>> {
        self.proxy(ROOT, DEST).await
    }
    async fn proxy<'a>(&'a self, path: &'a str, iface: &'a str) -> Result<Proxy<'a>> {
        Ok(Proxy::new(&self.connection, DEST, path, iface).await?)
    }
    pub async fn devices(&self) -> Result<Vec<Device>> {
        let paths: Vec<OwnedObjectPath> = self.manager().await?.call("GetDevices", &()).await?;
        let mut devices = Vec::new();
        for path in paths {
            // Hot-unplug can remove an object between enumeration and property reads.
            if let Ok(device) = self.device(&path).await {
                devices.push(device);
            }
        }
        devices.sort_by_key(|d| (d.transport != Transport::Usb, d.interface.clone()));
        Ok(devices)
    }
    async fn device(&self, path: &OwnedObjectPath) -> Result<Device> {
        let p = self
            .proxy(path.as_str(), "org.freedesktop.NetworkManager.Device")
            .await?;
        let interface: String = p.get_property("Interface").await?;
        let kind: u32 = p.get_property("DeviceType").await?;
        let mut driver: String = p.get_property("Driver").await.unwrap_or_default();
        let mut name = interface.clone();
        let transport =
            if let Some((product, usb_driver)) = usb_identity(&self.sysfs_root, &interface) {
                name = product;
                driver = usb_driver;
                Transport::Usb
            } else {
                match kind {
                    2 => Transport::Wifi,
                    5 => Transport::Bluetooth,
                    30 => Transport::WifiDirect,
                    _ => Transport::Other,
                }
            };
        let mut device = Device {
            interface,
            path: path.to_string(),
            transport,
            name,
            driver,
            state: p.get_property("State").await?,
            managed: p.get_property("Managed").await?,
            addresses: Vec::new(),
            gateway: String::new(),
            dns: Vec::new(),
        };
        for (property, api) in [
            ("Ip4Config", "org.freedesktop.NetworkManager.IP4Config"),
            ("Ip6Config", "org.freedesktop.NetworkManager.IP6Config"),
        ] {
            let config: OwnedObjectPath = p.get_property(property).await?;
            if config.as_str() == "/" {
                continue;
            }
            let p = self.proxy(config.as_str(), api).await?;
            let addresses: Vec<HashMap<String, OwnedValue>> =
                p.get_property("AddressData").await.unwrap_or_default();
            for a in addresses {
                if let Some(v) = a.get("address").and_then(|v| <&str>::try_from(v).ok()) {
                    device.addresses.push(v.into());
                }
            }
            let gateway: String = p.get_property("Gateway").await.unwrap_or_default();
            if device.gateway.is_empty() && !gateway.is_empty() {
                device.gateway = gateway;
            }
            let servers: Vec<HashMap<String, OwnedValue>> =
                p.get_property("NameserverData").await.unwrap_or_default();
            for s in servers {
                if let Some(v) = s.get("address").and_then(|v| <&str>::try_from(v).ok()) {
                    device.dns.push(v.into());
                }
            }
        }
        Ok(device)
    }
    pub async fn find(&self, interface: &str) -> Result<Device> {
        if !valid_interface(interface) {
            bail!("Invalid network interface name");
        }
        self.devices()
            .await?
            .into_iter()
            .find(|d| d.interface == interface)
            .context("Device is no longer available")
    }
    pub async fn activate_usb(
        &self,
        device: &Device,
        full_system: bool,
    ) -> Result<OwnedConnection> {
        // Re-enumerate, rather than trusting an old UI selection or a forged D-Bus path.
        let device = self.find(&device.interface).await?;
        if !device.eligible_usb() {
            bail!(
                "Select a managed RNDIS/NCM/CDC USB tethering interface. Enable USB tethering on the phone first."
            );
        }
        let manager = self.manager().await?;
        let path = OwnedObjectPath::try_from(device.path)?;
        let checkpoint: OwnedObjectPath = manager.call("CheckpointCreate", &(vec![path.clone()], 90u32, 0u32)).await
            .context("Cannot create recovery checkpoint. Check the Polkit agent and NetworkManager permissions")?;
        let settings = usb_settings(&device.interface, full_system);
        let options = HashMap::from([
            ("persist", string("volatile")),
            ("bind-activation", string("dbus-client")),
        ]);
        let result: zbus::Result<(
            OwnedObjectPath,
            OwnedObjectPath,
            HashMap<String, OwnedValue>,
        )> = manager
            .call(
                "AddAndActivateConnection2",
                &(settings, path, OwnedObjectPath::try_from("/")?, options),
            )
            .await;
        match result {
            Ok((_, active, _)) => Ok(OwnedConnection { active, checkpoint }),
            Err(error) => {
                let cleanup = self.rollback(&checkpoint).await;
                match cleanup {
                    Ok(()) => Err(error.into()),
                    Err(recovery) => bail!(
                        "Activation failed: {error}; checkpoint recovery failed: {recovery}. Inspect NetworkManager; a failed rollback result may have consumed the checkpoint."
                    ),
                }
            }
        }
    }
    pub async fn renew(&self, owned: &OwnedConnection) -> Result<()> {
        self.manager()
            .await?
            .call::<_, _, ()>(
                "CheckpointAdjustRollbackTimeout",
                &(&owned.checkpoint, 90u32),
            )
            .await?;
        Ok(())
    }
    pub async fn rollback(&self, checkpoint: &OwnedObjectPath) -> Result<()> {
        let results: HashMap<String, u32> = self
            .manager()
            .await?
            .call("CheckpointRollback", &(checkpoint,))
            .await?;
        if results.values().any(|value| *value != 0) {
            bail!("NetworkManager could not restore a device (rollback results: {results:?})");
        }
        Ok(())
    }
    pub async fn release(&self, owned: &OwnedConnection) -> Result<()> {
        // Rollback is required even if deactivation fails after hot unplug.
        let _: zbus::Result<()> = self
            .manager()
            .await?
            .call("DeactivateConnection", &(&owned.active,))
            .await;
        self.rollback(&owned.checkpoint).await
    }
    /// A brand-new, user-owned TUN, with no persistent profile or unrelated interface edits.
    pub async fn activate_companion_tun(&self) -> Result<OwnedObjectPath> {
        let devices = self.devices().await?;
        if devices
            .iter()
            .any(|d| d.interface == crate::tunnel::INTERFACE)
        {
            bail!("tetherust0 already exists; refusing to replace it");
        }
        if devices
            .iter()
            .flat_map(|d| &d.addresses)
            .any(|ip| ip.starts_with("198.18.") || ip.starts_with("198.19."))
        {
            bail!("Virtual DNS address pool conflicts with an existing interface");
        }
        let manager = self.manager().await?;
        let settings = companion_tun_settings();
        let options = HashMap::from([
            ("persist", string("volatile")),
            ("bind-activation", string("dbus-client")),
        ]);
        let (_,active,_): (OwnedObjectPath,OwnedObjectPath,HashMap<String,OwnedValue>) = manager.call("AddAndActivateConnection2", &(settings,OwnedObjectPath::try_from("/")?,OwnedObjectPath::try_from("/")?,options)).await.context("Cannot create the temporary user-owned TUN. Check NetworkManager Polkit authorization")?;
        Ok(active)
    }
    pub async fn deactivate_companion_tun(&self, active: &OwnedObjectPath) -> Result<()> {
        self.manager()
            .await?
            .call::<_, _, ()>("DeactivateConnection", &(active,))
            .await?;
        Ok(())
    }
    pub async fn version(&self) -> Result<String> {
        Ok(self.manager().await?.get_property("Version").await?)
    }
}
pub fn usb_settings(interface: &str, full_system: bool) -> Settings {
    let mut settings = HashMap::from([
        (
            "connection".into(),
            HashMap::from([
                ("id".into(), string("Tetherust USB (temporary)")),
                ("type".into(), string("802-3-ethernet")),
                ("interface-name".into(), string(interface)),
                ("autoconnect".into(), OwnedValue::from(false)),
            ]),
        ),
        ("802-3-ethernet".into(), HashMap::new()),
    ]);
    for family in ["ipv4", "ipv6"] {
        let mut ip = HashMap::from([
            ("method".into(), string("auto")),
            ("may-fail".into(), OwnedValue::from(true)),
            ("never-default".into(), OwnedValue::from(false)),
            (
                "dns-priority".into(),
                OwnedValue::from(if full_system { -100i32 } else { 60000i32 }),
            ),
            (
                "route-metric".into(),
                OwnedValue::from(if full_system { 20i64 } else { 60000i64 }),
            ),
        ]);
        if full_system {
            ip.insert(
                "dns-search".into(),
                Value::from(vec!["~."]).try_to_owned().expect("DNS domain"),
            );
        }
        settings.insert(family.into(), ip);
    }
    settings
}

fn owned<T: Into<Value<'static>>>(value: T) -> OwnedValue {
    value.into().try_to_owned().expect("owned D-Bus setting")
}
pub fn companion_tun_settings() -> Settings {
    // SAFETY: UID/GID reads have no pointer arguments or side effects.
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };
    let mut settings = HashMap::from([
        (
            "connection".into(),
            HashMap::from([
                ("id".into(), string("Tetherust companion (temporary)")),
                ("type".into(), string("tun")),
                ("interface-name".into(), string(crate::tunnel::INTERFACE)),
                ("autoconnect".into(), OwnedValue::from(false)),
            ]),
        ),
        (
            "tun".into(),
            HashMap::from([
                ("mode".into(), OwnedValue::from(1u32)),
                ("owner".into(), string(&uid.to_string())),
                ("group".into(), string(&gid.to_string())),
                ("pi".into(), OwnedValue::from(false)),
            ]),
        ),
    ]);
    for (family, address, prefix, halves) in [
        ("ipv4", "198.19.255.254", 15u32, ["0.0.0.0", "128.0.0.0"]),
        ("ipv6", "fd41:7465:7468::1", 128, ["::", "8000::"]),
    ] {
        let addresses = vec![HashMap::from([
            ("address".to_string(), string(address)),
            ("prefix".into(), OwnedValue::from(prefix)),
        ])];
        let routes: Vec<_> = halves
            .into_iter()
            .map(|dest| {
                HashMap::from([
                    ("dest".to_string(), string(dest)),
                    ("prefix".into(), OwnedValue::from(1u32)),
                    ("metric".into(), OwnedValue::from(20u32)),
                ])
            })
            .collect();
        let mut ip = HashMap::from([
            ("method".into(), string("manual")),
            ("address-data".into(), owned(addresses)),
            ("route-data".into(), owned(routes)),
            ("dns-priority".into(), OwnedValue::from(-100i32)),
            ("dns-search".into(), owned(vec!["~."])),
            ("may-fail".into(), OwnedValue::from(family == "ipv6")),
        ]);
        if family == "ipv4" {
            ip.insert(
                "dns".into(),
                owned(vec![u32::from_ne_bytes([198, 18, 0, 1])]),
            );
        }
        settings.insert(family.into(), ip);
    }
    settings
}
