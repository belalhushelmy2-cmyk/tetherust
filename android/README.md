# Tetherust Companion — Android

Installable artifact: `../dist/Tetherust-Companion-0.3.0.apk`. It is a signed, non-debuggable Android APK, minimum Android 7 (API 24), targeting API 35. It has not been installed on or runtime-tested against the user's phone in this session. The user explicitly withdrew permission for further phone operations; all subsequent work/tests are laptop-only.

## Manual setup

1. Transfer the APK to the phone yourself and install it through Android's package installer. Allow that install source if Android asks.
2. Open **Tetherust Companion**, keep your existing working internet/VPN active, select App default, and tap **Start USB sharing**. A persistent notification provides a Stop action.
3. Connect a USB data cable. Enable USB debugging and authorize only your trusted laptop. Native Android USB tethering is not required for this mode.
4. Quit the older Linux window and open the matching **0.3.0 Linux client**. Choose the phone marked **USB companion** in Devices, then **Pair phone**. The desktop's Install button is an optional, explicitly requested ADB install operation; it is never automatic.
5. In Settings, enable **Full-Tunnel**, Save, then Connect. Leaving it unchecked selects local proxy-only mode, which does not route all laptop applications.
6. The desktop verifies DNS and HTTPS through its TUN before reporting system forwarding. Existing LAN routes stay available. VPN/policy routes and independent resolver configurations require separate auditing; this is not a guaranteed leak-blocking VPN. ICMP/raw IP protocols are not implemented by this socket proxy; TCP and UDP are.

The app captures Android's app-default Network and uses its socket factory, DNS resolver and UDP binding, including an applicable VPN. It never requests or binds to a cellular underlay. Network changes invalidate forwarding sessions; strict requests require an applicable VPN with default IPv4 routes. It refuses IPv6 under strict policy when default IPv6 VPN routes are absent. It does not intercept the phone's existing traffic and therefore does not need root or a phone VpnService. Internet/network-state permissions and an Android connected-device foreground service are used; notification permission is requested where supported. No contacts, files/media, location, accessibility, SMS or device-administrator permission is requested. Sharing starts only through the visible user control, not at boot.

## Pairing and security

The phone creates a P-256 EC identity in Android Keystore and a random 256-bit pairing token in private app preferences (Android backup disabled). Its TLS listener binds only to 127.0.0.1 by default. Explicit Wi-Fi sharing opens an authenticated LAN listener and advertises an untrusted mDNS service; identity must still be paired through trusted USB. An ADB forward exposes it only on the laptop's loopback interface. Every session uses TLS 1.2 with exact certificate pinning and handshake-signature verification, followed by authenticated SOCKS negotiation. There is no accept-all certificate verifier. A separate random credential protects the laptop's local SOCKS bridge and stays in process memory, outside command-line arguments.

Pairing material can be read only by Android's shell UID (2000) or the app's own UID. This intentionally trusts hosts the user has already authorized for Android debugging, including authorized wireless-debugging hosts. Other installed apps are rejected by the provider. It does not defend against root or a malicious authorized debugging host. Pair is an explicit desktop action; changed certificates fail until the user pairs again. Forget paired laptops on the phone stops sharing and rotates the identity/token. Desktop pairing files are private (0600) and are never exported in event logs.

TCP uses ordinary authenticated SOCKS CONNECT inside TLS. UDP uses a private, bounded length-framed SOCKS-datagram stream inside TLS; the Linux bridge translates this to standard local SOCKS5 UDP association. UDP datagrams are accepted back only from destinations contacted by the association. Fragmented SOCKS datagrams are rejected. Concurrency, packet sizes and destination tracking are bounded. Phone sockets bind to the captured app-default Network; stale generations are rejected. A phone-side HTTPS probe and a separately forwarded Linux HTTPS probe must report matching exit IPs; that is evidence only for those requests, not a universal VPN/leak proof. See the main README for strict mode, firewall recovery, Wi-Fi and local HTTP/SOCKS proxy limitations.

## Build without contacting a device

Requires JDK 17+, an installed Android SDK platform 35/build-tools 35.0.0, Kotlin compiler jars and zip. Gradle 8.11.1's lib directory contains a suitable Kotlin 2.0.20 compiler/stdlib. No Gradle daemon or Android device is required.

```sh
ANDROID_HOME=/path/to/sdk KOTLIN_COMPILER_HOME=/path/to/gradle-8.11.1/lib ./build.sh
```

This script only compiles, DEXes, aligns and signs the APK and verifies its signature. It never calls ADB. The stable signing key/password are private under the user's Tetherust data directory, outside the repository. Keep them to sign compatible future updates. App licenses are embedded as APK assets.

## Offline interoperability test

```sh
TETHERUST_ADB_DISABLED=1 KOTLIN_COMPILER_HOME=/path/to/gradle/lib \
  python3 ../tests/companion_host.py
```

Uses the exact Kotlin ProxyEngine on a local JVM, a temporary self-signed TLS fixture, localhost echo servers and the Rust bridge. It tests TCP data, UDP data, certificate/token/local-proxy authentication rejection and malformed UDP rejection. It neither invokes ADB nor demonstrates real phone/VPN forwarding. Android Keystore/foreground-service behavior, real app-default VPN binding, USB interruptions and device battery behavior still require manual device acceptance when authorized.

### Android TLS key compatibility fix

The original RSA identity failed on a physical RMX3430 with Android Keystore “Incompatible padding mode” during Conscrypt signing. New installs and updates use a new P-256 EC alias authorized for raw (DIGEST_NONE) TLS signatures as well as SHA-256/384/512. Existing RSA identity authorizations are immutable, so the old alias is retained until an explicit pairing reset. Updating requires pairing again; certificate pinning stays mandatory. The existing pairing token is preserved.

### Companion interface

The native Android screen uses the existing system font, density-aware spacing, dark mint-accented sharing/traffic cards, and inset handling for system bars and the keyboard. Sharing state and traffic counters come from the running service. Internet source, local proxy port and optional Wi-Fi settings are disabled while sharing runs; proxy fields appear only for proxy modes. Setup instructions expand on demand. Resetting pairing still requires its confirmation dialog. The interface never claims verified VPN routing from the service-running state.
