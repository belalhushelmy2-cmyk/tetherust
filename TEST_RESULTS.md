# Current checkpoint — 0.3.0 phone VPN / effective internet (2026-10-24)

The earlier prohibition on phone operations remains in force. This milestone performed **no phone query, install, launch, pairing read, forward, Wi-Fi discovery, or real phone network probe**. The user's live firewall/routes were not modified. Builds and tests use laptop-local fixtures and private namespaces. The 0.2.0 cellular-only description below is historical and is superseded by app-default Network routing.

- Default Rust suite: **18 passed**, with three separately opt-in cases ignored in that command. Added strict path rejection for missing/app-excluded VPN, split routes, blocked/unavailable network, local proxy mode and old protocol; untrusted Wi-Fi discovery/endpoint validation; existing pairing, TUN and DNS checks remain.
- Actual Kotlin/Rust engine interoperability: **1 opt-in test passed**. Includes pinned TLS/token/local-proxy authentication, TCP and repeated UDP, independent phone-exit command, strict rejection, stale generation rejection and closing an existing TCP stream on a simulated network change. JVM startup also checks actual local HTTP CONNECT and SOCKS5 proxy connections with remote hostnames and the HTTPS probe response parser (plain/chunked/rejected status). These are synthetic Android network states, not proof of Android VPN behavior.
- The old namespace harness did not enforce capability dropping despite the historical report below. This milestone corrects that: setpriv removes effective/permitted/bounding/inheritable/ambient capabilities for the entire client process before launch, locks NOROOT and no_new_privs, and assertions inspect both process and current-thread capability masks immediately before TUN open.
- Private D-Bus NetworkManager/state contract: **1 opt-in test passed**. Real NM/kernel TUN namespace acceptance: **1 opt-in test passed**, including zero-capability TUN use, actual virtual DNS, **ordinary TCP/UDP application sockets through the real kernel TUN → Rust bridge → pinned TLS Kotlin gateway**, routes, cleanup/reconnect and dedicated D-Bus lifetime cleanup. The gateway uses local echo endpoints and simulated Android VPN status, not a physical phone.
- Expanded firewall acceptance: **passed in a private user/network/mount namespace**. The actual root-entry helper applies fixed nftables rules only inside that namespace. Verifies blocked existing/new TCP, UDP, DNS, IPv4 and IPv6 direct output; allowed loopback/LAN/TUN output; failure after the TUN-named path disappears; explicit recovery and preservation of an unrelated firewall table; readable crash-recovery marker. An initial echo fixture replied from its primary address to a connected UDP client expecting the alternate address; the fixture now binds each test address explicitly. Only the corrected complete run is counted as a pass.
- Formatting/Clippy pass. Private Sway GUI acceptance passes all four pages, persisted Full-Tunnel/Require Phone VPN/Kill Switch controls without connecting, reconnect/notification preferences, autostart, redacted log copy/export, duplicate-session rejection and shutdown. Final AppImage packaged GUI acceptance also passes with phone access disabled; its three-second disconnected sample observed 34.42 MB RSS and 0.67% of one CPU core (not a sustained benchmark). Tar installation/uninstallation with a special-character prefix, DEB helper/notices/payload identity, AppImage runtime/extraction, and all artifact checksums pass. The root helper was copied only into test/install payloads, never into a privileged host location.
- Signed non-debuggable **0.3.0 APK** built offline with min API 24 / target 35. Android v2 and v3 signatures verify; v1 is not needed for the minimum supported Android version. Android compiler deprecation warnings concern conditional notification/status-bar APIs. No Android service, Keystore, callback or NSD runtime was exercised on hardware.
- Strict enforcement pins an observed probe exit, not a user-supplied VPN server identity. The phone must report a VPN applicable to the companion and default IPv4 VPN routes; IPv6 is refused without default IPv6 VPN routes. The phone, forwarded Linux and TUN HTTPS probe exits must agree. Internal VPN splitting, delayed exit changes between probes, destination-specific policies, LAN exceptions and arbitrary application DNS still require physical audit. No named VPN application is certified compatible.
- Local explicit HTTP/SOCKS proxy mode is TCP-only and has no direct fallback. It cannot satisfy strict VPN state. Wi-Fi listener/discovery/paired transport compiles and uses the same TLS pin/token policy; actual LAN/hotspot/Wi-Fi Direct/lockdown compatibility remains untested.

| Required phone condition | Phone exit / Linux exit / DNS / routes | VPN loss + USB unplug/replug |
| --- | --- | --- |
| A. No phone VPN | Not executed: phone operations prohibited | Not executed |
| B. Full-device VPN | Not executed: phone operations prohibited | Not executed |
| C. Split or proxy-based VPN | Not executed: phone operations prohibited | Not executed |

No real-phone forwarding, universal VPN sharing, sustained throughput, battery improvement, host Polkit acceptance or broad distribution compatibility is claimed. Local packages retain the current Garuda/glibc 2.43 development baseline. CI has not been run. See README for the explicit root-owned helper installation and recovery commands; no privileged helper was installed on the host during this milestone.

## Earlier 0.2.0 checkpoint (historical)

# Current checkpoint — 0.2.0 Android companion (2026-10-24)

The user explicitly prohibited further operations on the connected phone. Before that instruction, read-only ADB reported an Edge 60 Fusion / API 36. After it, no phone installation, launch, pairing retrieval, forward, file transfer or additional phone probe was performed. All following tests are on the laptop with ADB disabled, synthetic/local peers, or isolated network namespaces. Actual Android service/Keystore/cellular/USB acceptance is **not** claimed.

- Signed non-debuggable APK built from Kotlin and Android SDK platform/build-tools 35.0.0. APK signature verification passes v1, v2 and v3. Source targets API 35, minimum API 23. Compatibility conditionals intentionally retain a few deprecated Android calls for older platforms.
- Default Rust suite: **16 passed** (12 previous core tests, 4 companion/model/raw-packet tests). The private-bus, host-JVM and namespace tests are deliberately opt-in and run separately.
- Host Kotlin/Rust interoperability: **1 passed**, exercising the actual pure Kotlin ProxyEngine with pinned TLS, authenticated TCP/UDP bridging, repeated UDP replies, rejection of invalid phone tokens, changed certificates, invalid local credentials and fragmented UDP datagrams. Uses localhost echo endpoints and a temporary test certificate, not Android/ADB.
- Isolated D-Bus NetworkManager contract: **1 passed**, extended to validate the real serialized TUN profile, owner/group, volatile/bus-client options, profile DNS/routes and deactivation in addition to the previous USB/retry cases.
- Real NetworkManager + kernel TUN in isolated user/network/mount/PID/UTS/IPC namespaces: **1 passed**. An explicit guard rejects the host namespace/bus. The client drops all Linux capabilities before opening the owner-assigned TUN. Verified profile addresses/DNS, kernel preferred route, real UDP DNS through the kernel TUN and tun2proxy, explicit cleanup/reconnect, and automatic route removal after the dedicated D-Bus client disappears. Resolver mode is disabled in the test daemon so it does not edit the host resolver; this is not a real phone/system-wide resolver acceptance test.
- Formatting and Clippy pass. Private Sway UI regression passes with all ADB/phone access disabled. New phone-specific Install/Pair controls compile but were not exercised on a phone.
- `android/build.sh`, `tests/companion_host.py` and the namespace harness do not access a phone. In production, only explicit Install/Pair/Connect actions perform those corresponding phone operations; discovery reads the local ADB inventory.
- Final desktop packages passed tar installation/uninstallation under a private prefix containing spaces, dollar and percent characters; DEB payload/notices checks; AppImage CLI and private Sway GUI acceptance; and all artifact SHA256 checks. The generated PKGBUILD source checksum also passes. Packaged disconnected idle observation: 33.96 MB RSS and 1.00% of one CPU core over three seconds, not a throughput benchmark. These are local Garuda development builds with a glibc 2.43 baseline; cross-distribution acceptance and CI remain unverified.
- Still pending: Android UI/service/Keystore execution, real cellular egress, USB reconnect, phone suspend/battery behavior, phone IPv4/IPv6/UDP/TCP throughput, and live VPN/DNS/IPv6 leak auditing.

## Earlier 0.1.0 checkpoint (historical)

# Test results — 2026-10-24

Evidence from this development session. No Android phone, USB tethering interface, Bluetooth PAN peer or compatible phone Wi-Fi Direct connection was available. No claim of real phone forwarding or general product completion is made.

## Environment

- Garuda/Arch Linux x86_64, kernel 7.3.0-2-garuda; Rust 1.99.1; NetworkManager 1.58.1; Slint 1.18.1.
- Ordinary UID 1000, effective capabilities zero. Private headless Sway with pixman renderer used for GUI acceptance, not the user's live Sway configuration.
- Read-only real NetworkManager enumeration found Wi-Fi, Wi-Fi P2P capability, Ethernet, loopback and an existing TUN interface; **zero supported USB candidates**.
- Read-only Wi-Fi verification correctly failed closed under the current host network policy (DNS socket operation returned EPERM); it did not borrow connectivity from the existing TUN interface. This is not a successful internet/phone test.

## Passed software checks

- `cargo build --release --locked` (desktop); executable reports `tetherust 0.1.0`.
- `cargo check --locked --no-default-features` (CLI-only).
- `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`.
- `cargo test --locked --all-targets`: **12 passed**; the separate private-bus test is deliberately ignored in this command and was run explicitly below.
- `dbus-run-session -- cargo test --locked --test network_bus -- --ignored`: **1 passed**. Fake NetworkManager on a private session bus verifies device/IP object enumeration, USB ancestry, temporary profile options (volatile / dbus-client / no autoconnect), selected-device checkpoint scope, renewal, deactivation error followed by rollback, activation authorization-denial rollback, nonzero recovery result handling, invalid interface rejection and rejection when USB ancestry disappears. It also verifies that a failed initial connection never reports Connected and Disconnect cancels the waiting/retry state. These are contract tests, not kernel routing or physical phone acceptance.
- Core tests cover USB driver/ancestry classification without name guessing, path/control-character rejection, 64-bit kernel counters, DNS transaction/question/status checks, every truncated packet boundary, malformed packet handling, safe route/DNS profile settings, private atomic JSON, invalid settings rejection, startup quoting/removal, lock contention/release, non-overwriting private log export, bounded history and reconnect backoff saturation.
- `tests/native_sway.py` against the release binary: actual Wayland window rendering and all four navigation pages, settings checkbox persistence, per-user startup desktop entry, redacted log export/copy through private compositor clipboard, advanced diagnostics view, duplicate-session rejection and clean SIGTERM exit. No connect button was pressed on a live network. Screenshots saved in `dist/`.
- The first GUI signal test exposed a worker-stops/window-remains-open bug; fixed by ending the event loop when the watch channel closes, then the release GUI test passed. The deprecated Sway cursor IPC did not deliver test clicks on a pointer-less headless seat; the harness now uses a real Wayland virtual-pointer protocol helper. Initial failed attempts are not counted as acceptance passes.
- Release disconnected idle observation after UI actions: **32.83 MB RSS, 0.67% of one CPU core over a 3-second sample**. A small host-specific observation, not a sustained performance benchmark or tethering throughput result.
- Packaging shell syntax and desktop entry validation passed (desktop validator reports an informational dual-category hint).

## Packaging coverage

Local artifacts use this host's recent glibc ABI, recorded in BUILD_INFO.txt and the DEB dependency. These are development artifacts for the current host, not verified Debian/Ubuntu-compatible releases. The separate CI recipe builds on Ubuntu 22.04 for an older baseline; CI has not been run in this session.

- Built AppImage (~13 MB), DEB (~11 MB), generic tar.gz (~11 MB), matching source archive, SHA256SUMS and a PKGBUILD with the exact generated source checksum. Rust dependency notices are included in binary packages. Build tools were downloaded at pinned versions and verified against SHA256 before use.
- `tests/packages.py` passed: tar extraction, install/uninstall under a private prefix containing spaces, dollar and percent characters; desktop entry validation; installed CLI execution; Debian ar/control/data layout, explicit glibc baseline dependency, executable identity and notices; AppImage extract-and-run CLI; extracted AppRun/private-library GUI through the same private Sway interaction suite; all artifact checksums.
- The bundled GUI sample observed 32.79 MB RSS and 1.00% of one CPU core over 3 seconds. This is not hardware tethering performance evidence.
- No privileged package installation, cross-distribution runtime acceptance, public publishing or CI execution was performed.

## Explicitly untested or incomplete

- Actual Android USB detection, DHCP, forwarding, TCP/UDP internet, IPv4/IPv6, disconnect/replug and port changes.
- Real Polkit prompts/denial, restoration of an existing USB profile, physical unplug rollback, crash watchdog, NetworkManager restart and suspend/resume on a phone.
- Wi-Fi Direct joining, standard phone proxy compatibility, arbitrary UDP through a proxy, Bluetooth pairing/PAN establishment.
- Full-system route/DNS correctness in a physical topology, DNS/IPv6 leak capture/audit, VPN coexistence.
- Sustained USB/Wi-Fi throughput, latency/jitter/loss under load, reconnection time and battery/resource consumption during forwarding.
- KDE/GNOME/XFCE, independent X11 runtime, other distributions, DEB/Arch privileged installation/uninstallation and CI execution.
- Custom Android/ADB transport, automatic Wi-Fi Direct/PAN setup, TUN-to-proxy full-system routing, custom DNS editor and strict leak prevention are not implemented.

Follow `docs/HARDWARE_ACCEPTANCE.md` before promoting this development build to a practical daily-use release.

## Final package checkpoint

The final rebuilt AppImage passed the same private Sway interaction suite after the tray-host and retry-cancellation fixes. Final 3-second packaged idle sample: 32.75 MB RSS, 0.67% of one CPU core. Tar installer/uninstaller, DEB payload/notices, AppImage CLI and SHA256SUMS checks passed again. `makepkg --verifysource --noconfirm` from `dist/` validated the generated PKGBUILD source checksum; an initial invocation from the project root correctly failed because PKGBUILD is in dist, and is not counted as a pass. No Arch package installation was performed.

## Dashboard visual refinement (2026-10-24)

Retained the existing font family and the user's Fontconfig substitution; no font files or font preferences were installed/changed. Added consistent vector icons, centered navigation glyphs, a clearer primary phone action, equal-width traffic cards, compact status pills and a combined connection-details panel. Private Sway GUI regression passes; an additional private preview uses the existing JetBrains Mono rule at the user's tiled-window width to verify wrapping/alignment. Clippy and the optimized desktop build pass. Phone access remains disabled for preview and acceptance runs.

## Physical-device TLS diagnosis and local fix (2026-10-24)

With user-authorized USB detection, pairing and connection attempts, RMX3430 was authorized, pairing read succeeded, and SharingService was confirmed foreground/running. TLS failed before authentication; read-only process-filtered logcat showed Conscrypt raw RSA signing rejected by Android Keystore with incompatible padding (-11). Replaced new identity generation with P-256 EC, DIGEST_NONE plus SHA digests, and a new alias to migrate immutable key authorizations. Existing identity/token not reset on the phone.

Android APK compilation and v2/v3 signature verification passed. Offline Kotlin/Rust interoperability with a P-256 EC fixture passed (pinned TLS, authentication rejection, TCP/UDP forwarding and routing-policy checks). This fixture does not exercise Android Keystore. Corrected APK is built locally; installation and physical handshake/VPN/system-routing acceptance remain pending user approval. No phone settings changes or APK installation performed. Linux packages still contain the previous APK until repackaged.

## Physical connection follow-up and Android interface (2026-10-24)

Supersedes pending physical handshake status above: the phone subsequently exposed a P-256 EC identity, confirming the updated companion was installed. Its certificate differed from the laptop's old pairing. Re-pairing and connecting succeeded; the desktop showed Connected and independent phone/forwarded HTTPS exit verification passed. This was local proxy mode; phone VPN was not applicable/app excluded. Full-Tunnel was selected afterward, and the user reported the connection works; this is not a controlled full-system or VPN leak acceptance test. No APK installation was executed by this agent on the physical phone.

Redesigned the native companion screen with density-aware dark/mint cards, equal-width traffic metrics, service-derived status, a prominent sharing action, contextual proxy controls, active-session settings locking, expandable setup instructions, system bar/keyboard insets and existing font family. No forwarding/protocol changes in this UI update. Production APK builds and v2/v3 signature verification pass. Android 15 isolated emulator preview verified rendering, start/stop and service-driven status, disabled source/Wi-Fi controls while active, and SOCKS proxy field visibility. The preview-only build omitted FLAG_SECURE for screenshots; the production APK retains it. Earlier automated navigation checks required scroll/timing adjustments; incomplete reset/large-text checks are not counted as passes. The UI update has not been installed on the physical phone. Existing Linux packages still bundle the older companion.

Final production APK verification: installed only on isolated emulator-5582, opened setup help, opened/reset confirmation and cancelled it, then verified the sharing action remained reachable at 150% system text size and successfully started/stopped the service. Restored emulator text scale. All passed. Earlier automation mismatches were scroll timing and Android's uppercase dialog labels; the corrected checks passed. APK v2/v3 signatures reverified after the interruption.
