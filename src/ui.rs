use anyhow::Result;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};
use tetherust::{
    config::{self, Config},
    session::{Command, Snapshot},
    storage,
};
use tokio::sync::{mpsc, watch};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};
slint::include_modules!();
fn bytes(value: f64) -> String {
    if value >= 1_000_000_000. {
        format!("{:.1} GB", value / 1_000_000_000.)
    } else if value >= 1_000_000. {
        format!("{:.1} MB", value / 1_000_000.)
    } else if value >= 1_000. {
        format!("{:.1} KB", value / 1_000.)
    } else {
        format!("{value:.0} B")
    }
}
fn autostart_path() -> Result<PathBuf> {
    let dirs =
        directories::BaseDirs::new().ok_or_else(|| anyhow::anyhow!("No user config directory"))?;
    Ok(dirs.config_dir().join("autostart/tetherust.desktop"))
}
fn send(tx: &mpsc::Sender<Command>, command: Command, ui: &AppWindow) {
    if tx.try_send(command).is_err() {
        ui.set_feedback("Backend is busy or has stopped; try again.".into());
    }
}
async fn notify(summary: &'static str) {
    let Ok(connection) = zbus::Connection::session().await else {
        return;
    };
    let Ok(proxy) = zbus::Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    else {
        return;
    };
    let hints = std::collections::HashMap::<String, zbus::zvariant::OwnedValue>::new();
    let _: zbus::Result<u32> = proxy
        .call(
            "Notify",
            &(
                "Tetherust",
                0u32,
                "network-transmit-receive",
                summary,
                "Open Tetherust for connection details.",
                Vec::<String>::new(),
                hints,
                5000i32,
            ),
        )
        .await;
}
fn clipboard(text: &str) -> Result<()> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let mut command = if wayland {
        Command::new("wl-copy")
    } else {
        let mut c = Command::new("xclip");
        c.args(["-selection", "clipboard"]);
        c
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            anyhow::anyhow!(
                "Clipboard tool unavailable ({e}). Install {} or use Export logs.",
                if wayland { "wl-clipboard" } else { "xclip" }
            )
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Clipboard pipe unavailable"))?
        .write_all(text.as_bytes())?;
    if !child.wait()?.success() {
        anyhow::bail!("Desktop clipboard rejected the copy request");
    }
    Ok(())
}
pub fn run(
    tx: mpsc::Sender<Command>,
    state: watch::Receiver<Snapshot>,
    config: Config,
    data: PathBuf,
    snapshot: Option<PathBuf>,
) -> Result<()> {
    let ui = AppWindow::new()?;
    ui.set_auto_connect(config.auto_connect);
    ui.set_auto_reconnect(config.auto_reconnect);
    ui.set_full_system(config.full_system);
    ui.set_require_phone_vpn(config.require_phone_vpn);
    ui.set_kill_switch(config.kill_switch);
    ui.set_wifi_endpoint(config.wifi_endpoint.clone().into());
    ui.set_notifications(config.notifications);
    ui.set_selected(config.preferred_interface.into());
    ui.set_startup(autostart_path()?.exists());
    let notifications = Rc::new(Cell::new(config.notifications));
    let last_snapshot = Rc::new(RefCell::new(Snapshot::default()));
    ui.on_pair_phone({
        let weak = ui.as_weak();
        let tx = tx.clone();
        move || {
            if let Some(ui) = weak.upgrade()
                && let Some(serial) = ui.get_selected().strip_prefix("phone:")
            {
                send(&tx, Command::PairPhone(serial.into()), &ui);
            }
        }
    });
    ui.on_discover_wifi({
        let weak = ui.as_weak();
        let tx = tx.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                send(&tx, Command::DiscoverWifi, &ui);
            }
        }
    });
    ui.on_install_phone({
        let weak = ui.as_weak();
        let tx = tx.clone();
        move || {
            if let Some(ui) = weak.upgrade()
                && let Some(serial) = ui.get_selected().strip_prefix("phone:")
            {
                send(&tx, Command::InstallPhone(serial.into()), &ui);
            }
        }
    });
    ui.on_toggle_connection({
        let weak = ui.as_weak();
        let tx = tx.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                send(
                    &tx,
                    if ui.get_active() {
                        Command::Disconnect
                    } else {
                        Command::Connect(ui.get_selected().to_string())
                    },
                    &ui,
                );
            }
        }
    });
    ui.on_verify({
        let weak = ui.as_weak();
        let tx = tx.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                send(&tx, Command::Verify, &ui);
            }
        }
    });
    ui.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    ui.on_minimize({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                let _ = ui.hide();
            }
        }
    });
    ui.on_save_settings({
        let weak = ui.as_weak();
        let tx = tx.clone();
        let data = data.clone();
        let notifications = notifications.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let config = Config {
                    auto_connect: ui.get_auto_connect(),
                    auto_reconnect: ui.get_auto_reconnect(),
                    full_system: ui.get_full_system(),
                    require_phone_vpn: ui.get_require_phone_vpn(),
                    kill_switch: ui.get_kill_switch(),
                    wifi_endpoint: ui.get_wifi_endpoint().trim().to_string(),
                    notifications: ui.get_notifications(),
                    preferred_interface: ui.get_selected().to_string(),
                };
                let result = (|| -> Result<()> {
                    let exe = std::env::var_os("APPIMAGE")
                        .map(PathBuf::from)
                        .filter(|p| p.is_absolute() && p.is_file())
                        .unwrap_or(std::env::current_exe()?);
                    config.save(&data.join("config.json"))?;
                    config::autostart(&autostart_path()?, &exe, ui.get_startup())?;
                    Ok(())
                })();
                match result {
                    Ok(()) => {
                        notifications.set(config.notifications);
                        send(&tx, Command::Configure(config), &ui);
                        ui.set_feedback(
                            "Settings saved. Reconnect to apply route preferences.".into(),
                        );
                    }
                    Err(e) => {
                        ui.set_feedback(format!("Could not apply all settings: {e:#}").into())
                    }
                }
            }
        }
    });
    ui.on_export_logs({
        let weak = ui.as_weak();
        let last = last_snapshot.clone();
        let data = data.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let path = data.join(format!("tetherust-logs-{stamp}.txt"));
                match storage::export_logs(&path, &last.borrow().logs) {
                    Ok(()) => ui.set_feedback(
                        format!("Exported redacted logs to {}", path.display()).into(),
                    ),
                    Err(e) => ui.set_feedback(format!("Export failed: {e}").into()),
                }
            }
        }
    });
    ui.on_copy_logs({
        let weak = ui.as_weak();
        let last = last_snapshot.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                match clipboard(&last.borrow().logs.join("\n")) {
                    Ok(()) => ui.set_feedback("Copied redacted event logs.".into()),
                    Err(e) => ui.set_feedback(e.to_string().into()),
                }
            }
        }
    });
    // Native StatusNotifierItem / DBusMenu on Wayland. Closing always quits and restores routes.
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("show", "Show Tetherust", true, None),
        &MenuItem::with_id("quit", "Disconnect and Quit", true, None),
    ])?;
    let mut pixels = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let color = if (6..26).contains(&x) && (7..12).contains(&y)
                || (13..19).contains(&x) && (7..26).contains(&y)
            {
                [102, 227, 183, 255]
            } else {
                [19, 27, 38, 255]
            };
            pixels[(y * 32 + x) * 4..(y * 32 + x + 1) * 4].copy_from_slice(&color);
        }
    }
    let tray = if snapshot.is_none() {
        TrayIconBuilder::new()
            .with_id("tetherust")
            .with_tooltip("Tetherust — Android tethering")
            .with_menu(Box::new(menu))
            .with_icon(Icon::from_rgba(pixels, 32, 32)?)
            .build()
            .ok()
    } else {
        None
    };
    let tray_created = tray.is_some();
    let tray_host = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tray_watch = tokio::spawn({
        let host = tray_host.clone();
        async move {
            loop {
                let available = async {
                    let bus = zbus::Connection::session().await.ok()?;
                    let watcher = zbus::Proxy::new(
                        &bus,
                        "org.kde.StatusNotifierWatcher",
                        "/StatusNotifierWatcher",
                        "org.kde.StatusNotifierWatcher",
                    )
                    .await
                    .ok()?;
                    watcher
                        .get_property::<bool>("IsStatusNotifierHostRegistered")
                        .await
                        .ok()
                }
                .await
                .unwrap_or(false);
                host.store(available, std::sync::atomic::Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    });
    ui.set_tray_available(false);
    let weak = ui.as_weak();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let weak = weak.clone();
        let show = event.id.as_ref() == "show";
        let quit = event.id.as_ref() == "quit";
        let _ = slint::invoke_from_event_loop(move || {
            if quit {
                let _ = slint::quit_event_loop();
            } else if show && let Some(ui) = weak.upgrade() {
                let _ = ui.show();
                ui.window().set_minimized(false);
            }
        });
    }));
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let last_status = Rc::new(RefCell::new(String::new()));
    let snapshot_mode = snapshot.is_some();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(250), {
        let last = last_snapshot.clone();
        move || if let Some(ui) = weak.upgrade() {
            if state.has_changed().is_err() { let _ = slint::quit_event_loop(); return; }
            let tray_available = tray_created && tray_host.load(std::sync::atomic::Ordering::Relaxed);
            if ui.get_tray_available() && !tray_available { let _ = ui.show(); }
            ui.set_tray_available(tray_available);
            let current = state.borrow().clone();
            let previous = last.borrow();
            let inventory_changed = previous.devices.len() != current.devices.len() || previous.devices.iter().zip(&current.devices).any(|(a,b)| a.description() != b.description());
            if inventory_changed {
                let rows: Vec<_> = current.devices.iter().filter(|d| d.transport != tetherust::device::Transport::Other).map(|d| {
                    let wifi_companion = d.driver == "wifi-tls";
                    let transport = if wifi_companion { "Wi-Fi TLS".to_string() } else { d.transport.to_string() };
                    let status = if wifi_companion { "paired identity · reachability unchecked" }
                        else if d.transport == tetherust::device::Transport::Adb { if d.state == 30 { "USB authorized · companion mode" } else { "authorize USB debugging on Android" } }
                        else if d.state == 100 { "active" } else { "not active" };
                    DeviceRow { title: d.name.clone().into(), subtitle: format!("{} · {} · {}", transport, d.interface, status).into(), interface: d.interface.clone().into() }
                }).collect();
                if ui.get_selected().is_empty() && let Some(first) = current.devices.iter().find(|d| d.transport == tetherust::device::Transport::Adb).or_else(|| current.devices.iter().find(|d| d.eligible_usb())) { ui.set_selected(first.interface.clone().into()); }
                ui.set_devices(ModelRc::new(VecModel::from(rows)));
            }
            drop(previous);
            ui.set_state(current.status.clone().into()); ui.set_detail(current.detail.clone().into()); ui.set_active(current.active || current.guard_active); ui.set_busy(current.busy);
            ui.set_phone_vpn(current.phone_vpn.clone().into()); ui.set_vpn_forwarding(current.vpn_forwarding.clone().into()); ui.set_guard_active(current.guard_active);
            ui.set_verified(current.health.is_some());
            if current.active { ui.set_selected(current.selected.clone().into()); }
            ui.set_download(format!("{}/s", bytes(current.rx_rate)).into()); ui.set_upload(format!("{}/s", bytes(current.tx_rate)).into());
            ui.set_transferred(bytes((current.received + current.sent) as f64).into());
            ui.set_duration(format!("{:02}:{:02}:{:02}", current.seconds / 3600, current.seconds / 60 % 60, current.seconds % 60).into());
            if let Some(health) = &current.health {
                ui.set_public_ip(health.public_ip.clone().into()); ui.set_latency(format!("{} ms", health.latency_ms).into()); ui.set_dns_time(if health.address_family.starts_with("Companion") { "On phone".into() } else { format!("{} ms", health.dns_ms).into() });
                ui.set_route(if health.preferred_route { "Phone preferred for probe destination; VPN and IPv6 policies may differ." } else { "Interface egress verified; another system route is preferred." }.into());
            } else { ui.set_public_ip("—".into()); ui.set_latency("—".into()); ui.set_dns_time("—".into()); ui.set_route("Not verified".into()); }
            ui.set_log_text(current.logs.join("\n").into());
            ui.set_diagnostic_text(format!("{}\n\n{}", current.detail, current.devices.iter().map(|d| format!("{} · driver={} · DNS servers={} · addresses={}", d.description(), d.driver, d.dns.len(), d.addresses.len())).collect::<Vec<_>>().join("\n")).into());
            if *last_status.borrow() != current.status {
                if notifications.get() && !snapshot_mode && ["Connected", "Degraded", "Device removed", "Recovery needed"].contains(&current.status.as_str()) {
                    let text = match current.status.as_str() { "Connected" => "Phone internet verified", "Degraded" => "Phone internet verification failed", "Device removed" => "Phone disconnected", _ => "Network recovery needs attention" };
                    tokio::spawn(notify(text));
                }
                *last_status.borrow_mut() = current.status.clone();
            }
            *last.borrow_mut() = current;
        }
    });
    let snapshot_timer = slint::Timer::default();
    let render_error = Rc::new(RefCell::new(None));
    if let Some(path) = snapshot {
        let weak = ui.as_weak();
        let error = render_error.clone();
        snapshot_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs(2),
            move || {
                let result = (|| -> Result<()> {
                    let ui = weak
                        .upgrade()
                        .ok_or_else(|| anyhow::anyhow!("Window closed"))?;
                    let pixels = ui.window().take_snapshot()?;
                    image::save_buffer(
                        Path::new(&path),
                        pixels.as_bytes(),
                        pixels.width(),
                        pixels.height(),
                        image::ColorType::Rgba8,
                    )?;
                    Ok(())
                })();
                if let Err(e) = result {
                    *error.borrow_mut() = Some(e.to_string());
                }
                let _ = slint::quit_event_loop();
            },
        );
    }
    ui.run()?;
    tray_watch.abort();
    drop(tray);
    if let Some(error) = render_error.borrow_mut().take() {
        anyhow::bail!("Snapshot failed: {error}");
    }
    Ok(())
}
