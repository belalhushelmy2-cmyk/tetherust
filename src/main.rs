use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, time::Duration};
use tetherust::{
    config::Config,
    device::Transport,
    health,
    network::Network,
    session::{Command, Controller, Snapshot},
    storage::{self, InstanceLock},
};
use tokio::sync::{mpsc, watch};
#[cfg(feature = "desktop")]
mod ui;
#[derive(Parser)]
#[command(
    version,
    about = "Native Linux Android tethering. Never run the GUI as root."
)]
struct Args {
    #[command(subcommand)]
    command: Option<Mode>,
}
#[derive(Subcommand)]
enum Mode {
    /// Open the native Slint dashboard.
    Gui {
        /// Render a read-only dashboard snapshot and exit (disables auto-connect).
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// List NetworkManager links. USB candidates require native tethering enabled.
    Devices {
        #[arg(long)]
        json: bool,
    },
    /// Read-only environment and hardware diagnostics.
    Doctor,
    /// Activate USB, or monitor an already connected Wi-Fi/PAN link. Ctrl+C restores changes.
    Connect {
        #[arg(long)]
        interface: Option<String>,
        /// Explicitly prefer the phone's temporary default route and DNS. Not a kill switch.
        #[arg(long)]
        full_system: bool,
        #[arg(long)]
        no_reconnect: bool,
        #[arg(long)]
        json: bool,
    },
    /// Check DNS and TLS through one interface without changing network settings.
    Verify { interface: String },
    /// Probe an advertised Wi-Fi gateway's standard HTTP CONNECT proxy. No route changes.
    ProbeProxy {
        interface: String,
        #[arg(long, default_value_t = 8000)]
        port: u16,
    },
    /// Android companion operations. Installation/pairing run only when explicitly requested.
    Phone {
        #[command(subcommand)]
        command: PhoneMode,
    },
    /// Show redacted local connection history.
    History,
    /// Release a previously enabled Kill Switch through Polkit (also works after a crash).
    GuardOff,
}
#[derive(Subcommand)]
enum PhoneMode {
    Devices,
    DiscoverWifi,
    Pair {
        serial: String,
    },
    Install {
        serial: String,
        apk: PathBuf,
    },
    Connect {
        serial: String,
        #[arg(long)]
        full_system: bool,
        #[arg(long)]
        require_phone_vpn: bool,
        #[arg(long)]
        kill_switch: bool,
        #[arg(long)]
        wifi_endpoint: Option<std::net::SocketAddr>,
    },
}
async fn phone_command(command: PhoneMode, data: PathBuf) -> Result<()> {
    match command {
        PhoneMode::DiscoverWifi => {
            for endpoint in tetherust::companion::discover_wifi().await? {
                println!("{endpoint} (untrusted until USB-paired certificate verification)");
            }
        }
        PhoneMode::Devices => {
            for phone in tetherust::companion::phones().await? {
                println!(
                    "{} · {} · {}",
                    phone.serial,
                    phone.name,
                    if phone.authorized {
                        "authorized"
                    } else {
                        "authorize USB debugging on Android"
                    }
                );
            }
        }
        PhoneMode::Pair { serial } => {
            tetherust::companion::pair(&data, &serial).await?;
            println!("Phone paired. Start sharing in the Android companion, then connect.");
        }
        PhoneMode::Install { serial, apk } => {
            tetherust::companion::install(&serial, &apk).await?;
            println!("Installed. Open Tetherust Companion on Android and tap Start USB sharing.");
        }
        PhoneMode::Connect {
            serial,
            full_system,
            require_phone_vpn,
            kill_switch,
            wifi_endpoint,
        } => {
            refuse_root()?;
            let _lock = InstanceLock::acquire(&data.join("instance.lock"))?;
            let network = Network::open().await?;
            let config = tetherust::config::Config {
                full_system,
                require_phone_vpn,
                kill_switch,
                wifi_endpoint: wifi_endpoint
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                auto_reconnect: true,
                ..Default::default()
            };
            let (input, receiver) = mpsc::channel(8);
            let (output, mut state) = watch::channel(Snapshot::default());
            input
                .send(Command::Connect(format!("phone:{serial}")))
                .await?;
            let printer = tokio::spawn(async move {
                while state.changed().await.is_ok() {
                    let snapshot = state.borrow_and_update().clone();
                    println!(
                        "{} · {} · Phone VPN: {} · {} · guard={} · exit={}",
                        snapshot.status,
                        snapshot.detail,
                        snapshot.phone_vpn,
                        snapshot.vpn_forwarding,
                        snapshot.guard_active,
                        snapshot
                            .health
                            .map(|h| h.public_ip)
                            .unwrap_or_else(|| "unverified".into())
                    );
                }
            });
            Controller::new(network, config, data)
                .run(receiver, output)
                .await;
            printer.await?;
        }
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    // SAFETY: process initialization, before application file creation.
    unsafe {
        libc::umask(0o077);
    }
    let args = Args::parse();
    let mode = args.command.unwrap_or(Mode::Gui { snapshot: None });
    let data = storage::data_dir()?;
    if let Mode::History = mode {
        match std::fs::read_to_string(data.join("history.jsonl")) {
            Ok(text) => print!("{text}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("No sessions recorded."),
            Err(e) => return Err(e.into()),
        }
        return Ok(());
    }
    if let Mode::Phone { command } = mode {
        return phone_command(command, data).await;
    }
    if matches!(mode, Mode::GuardOff) {
        refuse_root()?;
        tetherust::guard::set(false).await?;
        println!("Kill Switch released");
        return Ok(());
    }
    let network = Network::open().await?;
    match mode {
        Mode::Devices { json } => {
            let devices = network.devices().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&devices)?);
            } else {
                for device in devices {
                    println!("{}", device.description());
                }
            }
        }
        Mode::Doctor => {
            println!(
                "Tetherust {} — read-only diagnostics",
                env!("CARGO_PKG_VERSION")
            );
            println!("NetworkManager: {}", network.version().await?);
            println!("Wayland: {}", std::env::var_os("WAYLAND_DISPLAY").is_some());
            println!(
                "Desktop: {}",
                std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into())
            );
            println!("USB debugging / ADB: not needed for native tethering");
            println!("DNS: NetworkManager automatic phone DNS; no /etc/resolv.conf editing");
            let devices = network.devices().await?;
            let usb = devices
                .iter()
                .filter(|d| d.transport == Transport::Usb)
                .count();
            for d in devices {
                println!(
                    "{} · driver={} · managed={}",
                    d.description(),
                    d.driver,
                    d.managed
                );
            }
            if usb == 0 {
                println!(
                    "No USB tethering interface. Connect a data cable, unlock Android, enable USB tethering. This does not indicate a tested USB connection."
                );
            }
            println!(
                "Bluetooth PAN requires paired, enabled tethering and a desktop-established PAN link."
            );
            println!(
                "Wi-Fi Direct support is device/adapter dependent; Wi-Fi HTTP CONNECT is TCP-only."
            );
            println!(
                "Network changes require NetworkManager Polkit permissions and a working session authentication agent."
            );
        }
        Mode::Verify { interface } => {
            let device = network.find(&interface).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&health::verify(&device).await?)?
            );
        }
        Mode::ProbeProxy { interface, port } => {
            let device = network.find(&interface).await?;
            let endpoint = std::net::SocketAddr::new(
                device
                    .gateway
                    .parse()
                    .context("Selected link has no advertised IP gateway")?,
                port,
            );
            let result = health::verify_proxy(&device, endpoint).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            println!("Verified standard HTTP CONNECT endpoint: http://{endpoint}");
            println!("TCP proxy diagnostic only; no UDP or system-wide routing is enabled.");
        }
        Mode::Connect {
            interface,
            full_system,
            no_reconnect,
            json,
        } => {
            refuse_root()?;
            let _lock = InstanceLock::acquire(&data.join("instance.lock"))?;
            let mut config = Config::load(&data.join("config.json"))?;
            config.full_system = full_system;
            config.auto_reconnect = !no_reconnect;
            config.auto_connect = false;
            let interface = match interface {
                Some(i) => i,
                None => {
                    let devices = network.devices().await?;
                    let candidates: Vec<_> = devices.iter().filter(|d| d.eligible_usb()).collect();
                    if candidates.len() != 1 {
                        bail!(
                            "Expected exactly one USB tethering interface; found {}. Use devices, then connect --interface NAME.",
                            candidates.len()
                        );
                    }
                    candidates[0].interface.clone()
                }
            };
            let (tx, rx) = mpsc::channel(16);
            let (output, mut state) = watch::channel(Snapshot::default());
            tx.send(Command::Connect(interface)).await?;
            let worker = tokio::spawn(Controller::new(network, config, data).run(rx, output));
            let mut previous = String::new();
            while state.changed().await.is_ok() {
                let snapshot = state.borrow_and_update().clone();
                if json {
                    println!("{}", serde_json::to_string(&snapshot)?);
                } else if previous != snapshot.status {
                    println!("{}: {}", snapshot.status, snapshot.detail);
                    previous = snapshot.status;
                }
            }
            worker.await?;
        }
        Mode::Gui { snapshot } => {
            refuse_root()?;
            let _lock = InstanceLock::acquire(&data.join("instance.lock"))?;
            let mut config = Config::load(&data.join("config.json"))?;
            if snapshot.is_some() {
                config.auto_connect = false;
            }
            let (tx, rx) = mpsc::channel(16);
            let (output, state) = watch::channel(Snapshot::default());
            let worker = tokio::spawn(
                Controller::new(network, config.clone(), data.clone()).run(rx, output),
            );
            #[cfg(feature = "desktop")]
            let result = ui::run(tx.clone(), state, config, data, snapshot);
            #[cfg(not(feature = "desktop"))]
            let result: Result<()> = {
                let _ = (state, snapshot);
                Err(anyhow::anyhow!(
                    "This build has no GUI. Build with the desktop feature."
                ))
            };
            let _ = tx.send(Command::Shutdown).await;
            tokio::time::timeout(Duration::from_secs(60), worker).await.context("Network recovery timed out; NetworkManager checkpoint remains the crash-recovery fallback")??;
            result?;
        }
        Mode::History | Mode::GuardOff | Mode::Phone { .. } => unreachable!(),
    }
    Ok(())
}
fn refuse_root() -> Result<()> {
    // SAFETY: geteuid has no arguments or memory accesses.
    if unsafe { libc::geteuid() } == 0 {
        bail!(
            "Run Tetherust as your normal desktop user. NetworkManager uses Polkit for network authorization."
        );
    }
    Ok(())
}
