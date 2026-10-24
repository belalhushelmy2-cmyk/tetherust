use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

pub fn data_dir() -> Result<PathBuf> {
    let base = directories::ProjectDirs::from("org", "tetherust", "tetherust")
        .context("No user home directory")?;
    let path = base.data_local_dir().to_path_buf();
    std::fs::create_dir_all(&path)?;
    Ok(path)
}
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().context("Path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(data)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
pub struct InstanceLock {
    _file: File,
}
impl InstanceLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        // SAFETY: descriptor is live; flock is released automatically on close/crash.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("Another Tetherust session is running, or its lock cannot be acquired");
        }
        Ok(Self { _file: file })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct History {
    pub started_unix: u64,
    pub seconds: u64,
    pub received: u64,
    pub sent: u64,
    pub verified: bool,
}
pub fn append_history(path: &Path, history: &History) -> Result<()> {
    // A bounded local JSON-lines journal. No IPs, phone names, serials or pairing secrets.
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<&str> = old.lines().rev().take(99).collect();
    lines.reverse();
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&serde_json::to_string(history)?);
    text.push('\n');
    atomic_write(path, text.as_bytes())
}
pub fn export_logs(path: &Path, lines: &[String]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .context("Log export requires a new filename")?;
    // Session log messages use fixed event labels, never raw backend error strings.
    for line in lines {
        writeln!(file, "{line}")?;
    }
    file.sync_all()?;
    Ok(())
}
