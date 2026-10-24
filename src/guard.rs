//! Fixed nftables policy. Never activated by discovery, startup, tests, or module initialization.
use anyhow::{Context, Result, ensure};
use std::{os::unix::fs::MetadataExt, path::PathBuf, time::Duration};
pub const RULES: &str = r#"table inet tetherust_guard {
 chain output {
  type filter hook output priority 10; policy drop;
  oifname "lo" accept
  oifname "tetherust0" accept
  udp dport { 53, 853 } drop
  tcp dport { 53, 853 } drop
  ip daddr { 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16 } accept
  ip6 daddr { fc00::/7, fe80::/10 } accept
  meta l4proto ipv6-icmp icmpv6 type { nd-router-solicit, nd-neighbor-solicit, nd-neighbor-advert } accept
 }
}
"#;
fn helper() -> Result<PathBuf> {
    for path in [
        "/usr/lib/tetherust/tetherust-netguard",
        "/usr/local/lib/tetherust/tetherust-netguard",
    ] {
        let path = PathBuf::from(path);
        if let Ok(metadata) = path.metadata()
            && metadata.is_file()
            && metadata.uid() == 0
            && metadata.mode() & 0o022 == 0
        {
            let canonical = path.canonicalize()?;
            let trusted = canonical.ancestors().all(|ancestor| {
                ancestor
                    .metadata()
                    .is_ok_and(|m| m.uid() == 0 && m.mode() & 0o022 == 0)
            });
            if trusted {
                return Ok(canonical);
            }
        }
    }
    anyhow::bail!(
        "Install the root-owned tetherust-netguard helper and nftables first. Strict VPN sharing requires Full-Tunnel and Kill Switch; the GUI stays unprivileged"
    )
}
pub fn available() -> Result<()> {
    helper().map(|_| ())
}
pub async fn set(enabled: bool) -> Result<()> {
    let executable = helper()?;
    let mut command = tokio::process::Command::new("/usr/bin/pkexec");
    let mut child = command
        .arg(executable)
        .arg(if enabled { "enable" } else { "disable" })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Polkit helper unavailable")?;
    let status = tokio::time::timeout(Duration::from_secs(50), child.wait())
        .await
        .context("Kill switch authorization timed out; inspect recovery instructions")??;
    ensure!(
        status.success(),
        "Kill switch authorization or nftables operation failed; no connection was started. If previously enabled, use Disconnect or the documented recovery command"
    );
    Ok(())
}

pub fn recorded_enabled() -> bool {
    std::fs::read_to_string("/run/tetherust-netguard/active")
        .is_ok_and(|value| value.trim() == "enabled")
}
