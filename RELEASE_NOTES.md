# Tetherust 0.3.0 — Android internet sharing for Linux

First public development release: a native Rust/Slint Linux client and Kotlin Android companion for sharing the phone’s effective internet connection. Existing Android VPNs can be used when they route the companion’s traffic through their tunnel.

## Downloads

- **Android:** `Tetherust-Companion-0.3.0.apk` — updated interface and Android Keystore TLS fix.
- **Linux x86_64:** AppImage, DEB, or generic tar.gz. The Linux packages include the matching companion APK and private ADB.
- **Source:** source tar.gz and an Arch `PKGBUILD`. Checksums are in `SHA256SUMS`.

## Start sharing

1. Install the APK and open the companion. Tap **Start sharing**.
2. Connect a USB data cable, enable USB debugging, and authorize your laptop.
3. Open Tetherust on Linux, select the phone, and choose **Pair phone**.
4. Enable **Full-Tunnel** in Settings, save, then **Connect** to route ordinary laptop applications. Without Full-Tunnel the client uses local proxy mode.

Run the Linux GUI as your regular user with NetworkManager and a desktop Polkit agent. If FUSE is unavailable, use `APPIMAGE_EXTRACT_AND_RUN=1 ./Tetherust-0.3.0-x86_64.AppImage`. For oversized XWayland UI, also set `SLINT_SCALE_FACTOR=1`. An older companion using the RSA identity must be paired again after this update.

## Included

Authenticated USB forwarding, pinned TLS, TCP/compatible UDP, phone-side DNS, optional Linux TUN routing, opt-in strict phone-VPN policy and Linux Kill Switch, optional encrypted LAN connection, reconnect supervision, and refined native interfaces. No second Android VPN or imported VPN credentials.

## Validation and compatibility

Physical pairing and matching phone/forwarded HTTPS internet probes passed on an RMX3430; the user reported working sharing. Isolated forwarding, Linux routing/firewall and native GUI tests passed. The new Android interface was checked in an Android 15 emulator, including start/stop, proxy controls, pairing reset cancellation and 150% text sizing.

**Development build:** Linux binaries were built on Arch/Garuda and require glibc 2.43 or newer; the DEB is not a compatibility promise for older Debian/Ubuntu installations. Android requires API 24 or newer. Third-party VPN compatibility, controlled full-system/leak tests on the physical phone, Wi-Fi/hotspot behavior and sustained performance remain unverified. A matching HTTPS exit IP is evidence for the probe, not a universal leak audit. See `TEST_RESULTS.md` and the README for guard installation/recovery and routing limits.
