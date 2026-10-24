use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transport {
    Usb,
    Adb,
    Bluetooth,
    Wifi,
    WifiDirect,
    Other,
}
impl std::fmt::Display for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Usb => "USB",
            Self::Adb => "USB companion",
            Self::Bluetooth => "Bluetooth PAN",
            Self::Wifi => "Wi-Fi",
            Self::WifiDirect => "Wi-Fi Direct",
            Self::Other => "Other",
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub interface: String,
    pub path: String,
    pub transport: Transport,
    pub name: String,
    pub driver: String,
    pub state: u32,
    pub managed: bool,
    pub addresses: Vec<String>,
    pub gateway: String,
    pub dns: Vec<String>,
}
impl Device {
    pub fn eligible_usb(&self) -> bool {
        self.transport == Transport::Usb && self.managed
    }
    pub fn description(&self) -> String {
        format!(
            "{} · {} · {} · {}",
            self.name,
            self.interface,
            self.transport,
            match self.state {
                100 => "active",
                30 => "disconnected",
                20 => "unavailable",
                10 => "unmanaged",
                40..=90 => "configuring",
                120 => "failed",
                _ => "inactive",
            }
        )
    }
}
pub fn valid_interface(s: &str) -> bool {
    !s.is_empty()
        && s.len() < libc::IFNAMSIZ
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}
// Do not identify a phone from an interface name or an arbitrary USB Ethernet adapter.
// RNDIS/NCM are tether-capable candidates; user selection + an egress test establishes usability.
pub fn usb_identity(root: &Path, interface: &str) -> Option<(String, String)> {
    if !valid_interface(interface) {
        return None;
    }
    let device = root.join(interface).join("device").canonicalize().ok()?;
    let driver = device
        .join("driver")
        .canonicalize()
        .ok()?
        .file_name()?
        .to_str()?
        .to_owned();
    if !["rndis_host", "cdc_ncm", "cdc_ether", "ipheth"].contains(&driver.as_str()) {
        return None;
    }
    for parent in device.ancestors() {
        if parent.join("idVendor").is_file() && parent.join("idProduct").is_file() {
            let name = std::fs::read_to_string(parent.join("product"))
                .unwrap_or_else(|_| "USB tether-capable device".into());
            return Some((
                name.trim()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(80)
                    .collect(),
                driver,
            ));
        }
    }
    None
}
pub fn counters_at(root: &Path, interface: &str) -> Option<(u64, u64)> {
    if !valid_interface(interface) {
        return None;
    }
    let base = root.join(interface).join("statistics");
    Some((
        std::fs::read_to_string(base.join("rx_bytes"))
            .ok()?
            .trim()
            .parse()
            .ok()?,
        std::fs::read_to_string(base.join("tx_bytes"))
            .ok()?
            .trim()
            .parse()
            .ok()?,
    ))
}
pub fn counters(interface: &str) -> Option<(u64, u64)> {
    counters_at(&PathBuf::from("/sys/class/net"), interface)
}
