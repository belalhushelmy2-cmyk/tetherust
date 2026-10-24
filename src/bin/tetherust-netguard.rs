//! Minimal root entry point: two fixed operations, no GUI, supplied rules, paths or shell execution.
use anyhow::{Result, ensure};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn main() -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "Use Polkit to authorize this helper"
    );
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        arguments.len() == 1 && ["enable", "disable"].contains(&arguments[0].as_str()),
        "Expected enable or disable"
    );
    let _uid: u32 = std::env::var("PKEXEC_UID")?.parse()?;
    // Serialise simultaneous operations. The lock is in a root-owned runtime directory.
    let dir = "/run/tetherust-netguard";
    match std::fs::create_dir(dir) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let meta = std::fs::symlink_metadata(dir)?;
    ensure!(
        meta.is_dir() && meta.uid() == 0 && meta.mode() & 0o022 == 0,
        "Unsafe guard runtime directory"
    );
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(format!("{dir}/lock"))?;
    use std::os::fd::AsRawFd;
    ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } == 0,
        "Guard lock unavailable"
    );
    let exists = Command::new("/usr/bin/nft")
        .args(["list", "table", "inet", "tetherust_guard"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();
    let enable = arguments[0] == "enable";
    if enable == exists {
        if enable {
            std::fs::write(format!("{dir}/active"), "enabled\n")?;
            std::fs::set_permissions(
                format!("{dir}/active"),
                std::fs::Permissions::from_mode(0o644),
            )?;
        } else {
            let _ = std::fs::remove_file(format!("{dir}/active"));
        }
        return Ok(());
    }
    let rules = if enable {
        tetherust::guard::RULES
    } else {
        "delete table inet tetherust_guard\n"
    };
    let mut child = Command::new("/usr/bin/nft")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(rules.as_bytes())?;
    ensure!(
        child.wait()?.success(),
        "Fixed nftables policy could not be applied"
    );
    if enable {
        std::fs::write(format!("{dir}/active"), "enabled\n")?;
        std::fs::set_permissions(
            format!("{dir}/active"),
            std::fs::Permissions::from_mode(0o644),
        )?;
    } else {
        let _ = std::fs::remove_file(format!("{dir}/active"));
    }
    Ok(())
}
