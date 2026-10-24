use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub auto_connect: bool,
    pub auto_reconnect: bool,
    pub full_system: bool,
    pub require_phone_vpn: bool,
    pub kill_switch: bool,
    pub wifi_endpoint: String,
    pub notifications: bool,
    pub preferred_interface: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            auto_connect: false,
            auto_reconnect: true,
            full_system: false,
            require_phone_vpn: false,
            kill_switch: false,
            wifi_endpoint: String::new(),
            notifications: true,
            preferred_interface: String::new(),
        }
    }
}
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(data) => {
                ensure!(data.len() <= 16384, "Configuration is too large");
                Ok(serde_json::from_slice(&data).context("Invalid Tetherust configuration")?)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        if !self.wifi_endpoint.is_empty() {
            ensure!(
                crate::companion::valid_wifi_endpoint(
                    self.wifi_endpoint
                        .parse()
                        .context("Enter a numeric LAN IP:port for Wi-Fi")?
                ),
                "Wi-Fi endpoint must be a private or link-local address"
            );
        }
        ensure!(
            self.preferred_interface.is_empty()
                || (crate::device::valid_interface(&self.preferred_interface)
                    || self
                        .preferred_interface
                        .strip_prefix("phone:")
                        .is_some_and(crate::companion::valid_serial)),
            "Invalid preferred interface"
        );
        crate::storage::atomic_write(path, &serde_json::to_vec_pretty(self)?)
    }
}
pub fn autostart(path: &Path, executable: &Path, enabled: bool) -> Result<()> {
    if !enabled {
        match std::fs::remove_file(path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
        return Ok(());
    }
    let executable = executable
        .to_str()
        .context("Executable path must be UTF-8")?;
    ensure!(
        !executable.contains(['\n', '\r']),
        "Invalid executable path"
    );
    // Desktop Exec quoting differs from shell quoting; escape all reserved characters.
    let escaped = executable
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    crate::storage::atomic_write(path, format!("[Desktop Entry]\nType=Application\nName=Tetherust\nExec=\"{escaped}\" gui\nIcon=tetherust\nTerminal=false\nX-GNOME-Autostart-enabled=true\n").as_bytes())
}
