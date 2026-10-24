use crate::{
    config::Config,
    device::{self, Device},
    health::{self, Health},
    network::{Network, OwnedConnection},
    storage::{self, History},
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Debug)]
pub enum Command {
    Connect(String),
    Disconnect,
    Verify,
    PairPhone(String),
    InstallPhone(String),
    DiscoverWifi,
    Configure(Config),
    Shutdown,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub status: String,
    pub detail: String,
    pub devices: Vec<Device>,
    pub selected: String,
    pub health: Option<Health>,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub received: u64,
    pub sent: u64,
    pub seconds: u64,
    pub busy: bool,
    pub active: bool,
    pub guard_active: bool,
    pub phone_vpn: String,
    pub vpn_forwarding: String,
    pub logs: Vec<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            status: "Disconnected".into(),
            detail:
                "Start sharing in the Android companion, select your phone, then Pair and Connect."
                    .into(),
            devices: Vec::new(),
            selected: String::new(),
            health: None,
            rx_rate: 0.,
            tx_rate: 0.,
            received: 0,
            sent: 0,
            seconds: 0,
            busy: false,
            active: false,
            guard_active: false,
            phone_vpn: "Not checked".into(),
            vpn_forwarding: "Unverified".into(),
            logs: Vec::new(),
        }
    }
}
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64 << attempt.min(5))
}
struct Live {
    interface: String,
    owned: Option<OwnedConnection>,
    companion: Option<crate::companion::Link>,
    start: Instant,
    started_unix: u64,
    counters: Option<(u64, u64)>,
    sample_time: Instant,
    renew_time: Instant,
    verified: bool,
}
pub struct Controller {
    network: Network,
    config: Config,
    state: Snapshot,
    events: VecDeque<String>,
    live: Option<Live>,
    desired: Option<String>,
    attempts: u32,
    retry: Instant,
    next_health: Instant,
    strict_anchor: Option<crate::companion::StrictAnchor>,
    data_dir: PathBuf,
}
impl Controller {
    pub fn new(network: Network, config: Config, data_dir: PathBuf) -> Self {
        Self {
            network,
            config,
            state: Snapshot {
                guard_active: crate::guard::recorded_enabled(),
                ..Snapshot::default()
            },
            events: VecDeque::new(),
            live: None,
            desired: None,
            attempts: 0,
            retry: Instant::now(),
            next_health: Instant::now(),
            strict_anchor: None,
            data_dir,
        }
    }
    fn log(&mut self, event: &str) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if self.events.len() == 200 {
            self.events.pop_front();
        }
        self.events.push_back(format!("{timestamp} {event}"));
        self.state.logs = self.events.iter().cloned().collect();
    }
    fn publish(&self, output: &watch::Sender<Snapshot>) {
        output.send_replace(self.state.clone());
    }
    async fn stop(&mut self) {
        if self.live.is_none() && self.state.status != "Recovery needed" {
            self.state.status = "Disconnected".into();
            self.state.detail =
                "No active session. Select your paired Android companion to share its internet path.".into();
        }
        self.state.health = None;
        self.state.phone_vpn = "Not checked".into();
        self.state.vpn_forwarding = "Unverified".into();
        self.state.active = false;
        self.state.rx_rate = 0.;
        self.state.tx_rate = 0.;
        if let Some(live) = self.live.take() {
            self.state.status = "Disconnecting".into();
            let recovery = if let Some(companion) = live.companion {
                companion.close(&self.network).await
            } else if let Some(owned) = live.owned {
                self.network.release(&owned).await
            } else {
                Ok(())
            };
            match recovery {
                Ok(()) => {
                    self.state.status = "Disconnected".into();
                    self.state.detail =
                        "Session ended. Application-owned network changes restored.".into();
                    self.log("session stopped; network recovery completed");
                }
                Err(error) => {
                    self.state.status = "Recovery needed".into();
                    self.state.detail = format!(
                        "{error:#}. Inspect NetworkManager before reconnecting; recovery could not be confirmed."
                    );
                    self.desired = None;
                    self.config.auto_connect = false;
                    self.log("network recovery failed; inspect NetworkManager");
                }
            }
            let history = History {
                started_unix: live.started_unix,
                seconds: live.start.elapsed().as_secs(),
                received: self.state.received,
                sent: self.state.sent,
                verified: live.verified,
            };
            if storage::append_history(&self.data_dir.join("history.jsonl"), &history).is_err() {
                self.log("could not persist session history");
            }
        }
    }
    async fn connect(
        &mut self,
        interface: &str,
        output: &watch::Sender<Snapshot>,
    ) -> anyhow::Result<()> {
        if let Some(serial) = interface.strip_prefix("phone:") {
            anyhow::ensure!(
                std::env::var_os("TETHERUST_ADB_DISABLED").is_none(),
                "Phone access is disabled for this run; no connection or firewall change was attempted"
            );
            self.state.busy = true;
            self.state.status = "Opening companion".into();
            anyhow::ensure!(
                !self.config.require_phone_vpn
                    || (self.config.full_system && self.config.kill_switch),
                "Require Phone VPN needs Full-Tunnel and Kill Switch enabled"
            );
            anyhow::ensure!(
                !self.config.kill_switch || self.config.full_system,
                "Kill Switch requires Full-Tunnel"
            );
            if self.config.kill_switch {
                crate::guard::available()?;
                // Set conservatively before authorization: a timed-out helper might have applied it.
                self.state.guard_active = true;
                crate::guard::set(true).await?;
            }
            self.state.detail =
                "Authenticating the companion and checking its effective internet path…".into();
            self.publish(output);
            let (link, _) = crate::companion::Link::start(
                &self.network,
                &self.data_dir,
                serial,
                self.config.full_system,
                self.config.require_phone_vpn,
                self.strict_anchor.as_ref(),
                if self.config.wifi_endpoint.is_empty() {
                    None
                } else {
                    Some(self.config.wifi_endpoint.parse()?)
                },
            )
            .await?;
            if self.config.require_phone_vpn {
                self.strict_anchor = link.strict_anchor.clone();
            }
            self.state.phone_vpn = if link.path.vpn {
                "Applies to companion"
            } else {
                "Not applicable / app excluded"
            }
            .into();
            self.state.vpn_forwarding = if self.config.require_phone_vpn {
                "Phone/Linux probe exits match · not a leak audit"
            } else {
                "Unverified (ordinary sharing)"
            }
            .into();
            let now = Instant::now();
            self.live = Some(Live {
                interface: interface.into(),
                owned: None,
                companion: Some(link),
                start: now,
                started_unix: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                counters: device::counters(crate::tunnel::INTERFACE),
                sample_time: now,
                renew_time: now,
                verified: false,
            });
            self.state.selected = interface.into();
            self.state.received = 0;
            self.state.sent = 0;
            self.state.seconds = 0;
            self.state.active = true;
            self.state.busy = false;
            self.state.health = None;
            self.state.status = "Verifying".into();
            self.next_health = now;
            self.log("companion authenticated; checking forwarding");
            return Ok(());
        }
        anyhow::ensure!(
            !self.config.require_phone_vpn && !self.config.kill_switch && !self.state.guard_active,
            "VPN sharing and Kill Switch require the Android companion USB transport"
        );
        let device = self.network.find(interface).await?;
        // Only native USB activation is automatic. Existing Wi-Fi/PAN links can be verified without mutation.
        let owned = if device.eligible_usb() {
            self.state.status = "Configuring USB".into();
            self.state.busy = true;
            self.publish(output);
            Some(
                self.network
                    .activate_usb(&device, self.config.full_system)
                    .await?,
            )
        } else if device.state == 100
            && matches!(
                device.transport,
                device::Transport::Wifi
                    | device::Transport::WifiDirect
                    | device::Transport::Bluetooth
            )
        {
            None
        } else {
            anyhow::bail!(
                "Enable USB tethering, or join a Wi-Fi/PAN connection in your desktop network settings first. This device cannot currently forward traffic."
            );
        };
        let now = Instant::now();
        self.live = Some(Live {
            interface: interface.into(),
            owned,
            companion: None,
            start: now,
            started_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            counters: device::counters(interface),
            sample_time: now,
            renew_time: now,
            verified: false,
        });
        self.state.selected = interface.into();
        self.state.received = 0;
        self.state.sent = 0;
        self.state.seconds = 0;
        self.state.status = "Verifying".into();
        self.state.detail = "Waiting for DHCP and interface-bound DNS + TLS verification…".into();
        self.state.active = true;
        self.state.busy = false;
        self.next_health = Instant::now() + Duration::from_secs(2);
        self.log("session started; internet not yet verified");
        Ok(())
    }
    async fn tick(&mut self, output: &watch::Sender<Snapshot>) {
        match self.network.devices().await {
            Ok(mut devices) => {
                if self.network.physical_inventory
                    && std::env::var_os("TETHERUST_ADB_DISABLED").is_none()
                    && let Ok(phones) = if self.config.wifi_endpoint.is_empty() {
                        crate::companion::phones().await
                    } else {
                        Ok(crate::companion::paired_phones(&self.data_dir))
                    }
                {
                    devices.extend(phones.into_iter().map(|phone| {
                        Device {
                            interface: format!("phone:{}", phone.serial),
                            path: phone.serial,
                            transport: device::Transport::Adb,
                            name: phone.name,
                            driver: if self.config.wifi_endpoint.is_empty() {
                                "adb"
                            } else {
                                "wifi-tls"
                            }
                            .into(),
                            state: if phone.authorized { 30 } else { 20 },
                            managed: true,
                            addresses: vec![],
                            gateway: String::new(),
                            dns: vec![],
                        }
                    }));
                }

                let old: Vec<_> = self
                    .state
                    .devices
                    .iter()
                    .map(|d| d.interface.as_str())
                    .collect();
                let new: Vec<_> = devices.iter().map(|d| d.interface.as_str()).collect();
                if old != new {
                    self.log("network device inventory changed");
                }
                self.state.devices = devices;
            }
            Err(e) => {
                self.state.status = "NetworkManager unavailable".into();
                self.state.detail = e.to_string();
                self.state.health = None;
                if self.live.is_some() {
                    self.stop().await;
                }
                return;
            }
        }
        if self.desired.is_none() && self.config.auto_connect && self.live.is_none() {
            let candidates: Vec<_> = self
                .state
                .devices
                .iter()
                .filter(|d| {
                    (if d.transport == device::Transport::Adb {
                        d.state == 30
                            && d.interface.strip_prefix("phone:").is_some_and(|serial| {
                                crate::companion::Pairing::load(&self.data_dir, serial).is_ok()
                            })
                    } else {
                        d.eligible_usb()
                            && !self.config.require_phone_vpn
                            && !self.config.kill_switch
                    }) && (self.config.preferred_interface.is_empty()
                        || d.interface == self.config.preferred_interface)
                })
                .collect();
            if candidates.len() == 1 {
                self.desired = Some(candidates[0].interface.clone());
            }
        }
        if self.live.is_none()
            && Instant::now() >= self.retry
            && let Some(interface) = self.desired.clone()
        {
            if let Err(e) = self.connect(&interface, output).await {
                self.state.status = "Waiting to reconnect".into();
                self.state.detail = e.to_string();
                self.state.busy = false;
                self.log("connection attempt failed");
                self.retry = Instant::now() + backoff(self.attempts);
                self.attempts = self.attempts.saturating_add(1);
                if !self.config.auto_reconnect {
                    self.desired = None;
                }
            }
            return;
        }
        if self
            .live
            .as_ref()
            .is_some_and(|live| live.companion.is_some())
        {
            self.tick_companion().await;
            return;
        }
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let device = self
            .state
            .devices
            .iter()
            .find(|d| d.interface == live.interface)
            .cloned();
        let Some(device) = device else {
            self.state.status = "Device removed".into();
            self.publish(output);
            self.log("selected device removed");
            self.stop().await;
            if !self.config.auto_reconnect {
                self.desired = None;
            }
            self.retry = Instant::now() + backoff(self.attempts);
            self.attempts = self.attempts.saturating_add(1);
            return;
        };
        self.state.seconds = live.start.elapsed().as_secs();
        if let Some((rx, tx)) = device::counters(&live.interface) {
            if let Some((previous_rx, previous_tx)) = live.counters {
                let elapsed = live.sample_time.elapsed().as_secs_f64().max(0.001);
                let received = rx.saturating_sub(previous_rx);
                let sent = tx.saturating_sub(previous_tx);
                self.state.rx_rate = received as f64 / elapsed;
                self.state.tx_rate = sent as f64 / elapsed;
                self.state.received += received;
                self.state.sent += sent;
            }
            live.counters = Some((rx, tx));
            live.sample_time = Instant::now();
        }
        if live.renew_time.elapsed() >= Duration::from_secs(20) {
            if let Some(owned) = &live.owned
                && let Err(e) = self.network.renew(owned).await
            {
                self.state.detail = format!("Recovery checkpoint expired: {e}");
                self.log("recovery watchdog lost; ending session");
                self.stop().await;
                self.desired = None;
                return;
            }
            live.renew_time = Instant::now();
        }
        if device.state != 100 {
            self.state.health = None;
            self.state.status = if live.start.elapsed() < Duration::from_secs(40) {
                "Waiting for network"
            } else {
                "Degraded"
            }
            .into();
            if live.start.elapsed() >= Duration::from_secs(40) {
                self.log("activation lost or timed out");
                self.stop().await;
                if !self.config.auto_reconnect {
                    self.desired = None;
                }
                self.retry = Instant::now() + backoff(self.attempts);
                self.attempts = self.attempts.saturating_add(1);
            }
            self.next_health = Instant::now() + Duration::from_secs(2);
            return;
        }
        if Instant::now() < self.next_health {
            return;
        }
        self.publish(output);
        match health::verify(&device).await {
            Ok(health) => {
                if self.state.health.is_none() {
                    self.log("interface-bound DNS and TLS internet verification passed");
                }
                self.state.status = "Connected".into();
                self.state.detail = if health.preferred_route { "Phone egress verified. Kernel prefers this interface for the probe destination." } else { "Phone egress verified. Another route is preferred by the system; only interface-bound traffic is verified." }.into();
                self.state.health = Some(health);
                if let Some(live) = &mut self.live {
                    live.verified = true;
                    if live.start.elapsed() > Duration::from_secs(60) {
                        self.attempts = 0;
                    }
                }
            }
            Err(e) => {
                if self.state.health.is_some() {
                    self.log("internet verification lost");
                }
                self.state.health = None;
                self.state.status = "Degraded".into();
                self.state.detail = e.to_string();
                // Do not flap a healthy physical link because a single external endpoint is blocked.
            }
        }
        self.next_health = Instant::now() + Duration::from_secs(15);
    }
    async fn tick_companion(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let Some(link) = &live.companion else {
            return;
        };
        if !self
            .state
            .devices
            .iter()
            .any(|d| d.interface == live.interface && d.state == 30)
            || link.finished()
            || link.bridge.remote.check_path().await.is_err()
        {
            self.log("companion connection or phone network changed");
            self.stop().await;
            if !self.config.auto_reconnect {
                self.desired = None;
            }
            self.retry = Instant::now() + backoff(self.attempts);
            self.attempts = self.attempts.saturating_add(1);
            return;
        }
        self.state.seconds = live.start.elapsed().as_secs();
        if link.full_system()
            && let Some((rx, tx)) = device::counters(crate::tunnel::INTERFACE)
        {
            if let Some((previous_rx, previous_tx)) = live.counters {
                let seconds = live.sample_time.elapsed().as_secs_f64().max(0.001);
                let received = rx.saturating_sub(previous_rx);
                let sent = tx.saturating_sub(previous_tx);
                self.state.rx_rate = received as f64 / seconds;
                self.state.tx_rate = sent as f64 / seconds;
                self.state.received += received;
                self.state.sent += sent;
            }
            live.counters = Some((rx, tx));
            live.sample_time = Instant::now();
        }
        if Instant::now() < self.next_health {
            return;
        }
        let full_system = link.full_system();
        let require_vpn = link.strict_anchor.is_some();
        let result = match link.verify().await {
            Err(error) => Err(error),
            Ok(phone_health) if full_system => {
                async {
                    let device = self.network.find(crate::tunnel::INTERFACE).await?;
                    let health = health::verify(&device).await?;
                    anyhow::ensure!(health.public_ip.parse::<std::net::IpAddr>()? == phone_health.public_ip.parse::<std::net::IpAddr>()?, "TUN request exit differs from the independent phone/forwarding probes");
                    if require_vpn {
                        anyhow::ensure!(health.preferred_route, "Kernel does not prefer the phone TUN for the probe; strict full-system routing was not verified");
                    }
                    Ok::<_,anyhow::Error>(health)
                }.await
            }
            Ok(phone_health) => Ok(phone_health),
        };
        match result {
            Ok(health) => {
                self.state.status = "Connected".into();
                self.state.detail = if full_system && !link.path.udp_supported { "Companion TUN DNS and HTTPS verified through the configured local proxy. TCP only; UDP is unavailable in this proxy mode." } else if full_system { "Companion TUN DNS and HTTPS verified. TCP and UDP use the app-default phone network; VPN routing still needs compatibility/leak testing." }
                    else { "Phone app egress verified. Local proxy mode only; enable Full-Tunnel to route desktop applications." }.into();
                self.state.health = Some(health);
                live.verified = true;
                if live.start.elapsed() >= Duration::from_secs(60) {
                    self.attempts = 0;
                }
            }
            Err(error) => {
                self.state.status = "Degraded".into();
                self.state.detail = format!("{error:#}");
                self.state.health = None;
                if require_vpn {
                    let reason = format!("{error:#}");
                    self.stop().await;
                    self.state.status = "VPN sharing stopped".into();
                    self.state.detail = format!(
                        "{reason}. Kill Switch remains armed until Disconnect; reconnect explicitly after checking the phone VPN."
                    );
                    self.desired = None;
                }
            }
        }
        self.next_health = Instant::now() + Duration::from_secs(15);
    }
    pub async fn run(
        mut self,
        mut input: mpsc::Receiver<Command>,
        output: watch::Sender<Snapshot>,
    ) {
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let terminate = async {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM listener");
            tokio::select! { _ = tokio::signal::ctrl_c() => (), _ = sigterm.recv() => () }
        };
        tokio::pin!(terminate);
        loop {
            tokio::select! {
                _ = &mut terminate => break,
                command = input.recv() => match command {
                    None | Some(Command::Shutdown) => break,
                    Some(Command::Connect(interface)) => {
                        self.stop().await; self.strict_anchor = None; self.desired = Some(interface); self.retry = Instant::now(); self.attempts = 0;
                    }
                    Some(Command::Disconnect) => { self.desired = None; self.strict_anchor = None; self.config.auto_connect = false; self.stop().await; self.release_guard().await; }
                    Some(Command::Verify) => { self.next_health = Instant::now(); }
                    Some(Command::DiscoverWifi) => {
                        self.state.detail = match crate::companion::discover_wifi().await {
                            Ok(endpoints) => format!("Discovered LAN endpoints (untrusted until certificate-pinned): {}. Enter one in Settings; pair the identity through USB first.", endpoints.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")),
                            Err(error) => error.to_string(),
                        };
                    }
                    Some(Command::PairPhone(serial)) => {
                        match crate::companion::pair(&self.data_dir, &serial).await {
                            Ok(()) => { self.state.detail = "Phone paired through authorized ADB. Start sharing in the Android app, then Connect.".into(); self.log("companion pairing saved"); }
                            Err(error) => { self.state.detail = format!("Pairing failed: {error:#}"); self.log("companion pairing failed"); }
                        }
                    }
                    Some(Command::InstallPhone(serial)) => {
                        if let Some(apk) = crate::companion::apk_path() {
                            match crate::companion::install(&serial,&apk).await {
                                Ok(()) => { self.state.detail = "Companion installed. Open it on Android and tap Start USB sharing, then Pair here.".into(); self.log("companion installation completed"); }
                                Err(error) => { self.state.detail = format!("Installation failed: {error:#}"); self.log("companion installation failed"); }
                            }
                        } else { self.state.detail = "Companion APK is not beside this build. Install the APK manually, then Pair phone.".into(); }
                    }
                    Some(Command::Configure(config)) => { self.config = config; self.log("settings updated; route preference applies on next connection"); }
                },
                _ = timer.tick() => self.tick(&output).await,
            }
            self.publish(&output);
        }
        self.stop().await;
        self.release_guard().await;
        self.publish(&output);
    }
    async fn release_guard(&mut self) {
        if self.state.guard_active {
            match crate::guard::set(false).await {
                Ok(()) => self.state.guard_active = false,
                Err(error) => {
                    self.state.status = "Recovery needed".into();
                    self.state.detail = format!(
                        "Kill Switch remains enabled: {error}. Authorize Disconnect again or use the documented recovery command."
                    );
                }
            }
        }
    }
}
