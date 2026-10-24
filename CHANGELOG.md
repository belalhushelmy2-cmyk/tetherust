# Changelog

## 0.3.0 — Existing phone VPN / internet workflow

- Replace forced cellular binding with captured app-default Android networks, network-generation leases and session invalidation; never create a second VpnService.
- Authenticated protocol 2 status and independent phone/Linux HTTPS exit checks. Strict mode requires applicable default VPN routes, pins network/exit, rejects stale paths and refuses unsupported IPv6.
- Optional Polkit-authorized fixed nftables Kill Switch with explicit crash recovery, scrollable settings and honest VPN verification state.
- Explicit loopback HTTP/SOCKS proxy forwarding (TCP only), optional pinned Wi-Fi transport/mDNS and local paired-identity selection.
- P-256 EC Android Keystore identity fixes the physical-device TLS padding failure; existing RSA identities require re-pairing after updating.
- Refined native desktop and Android screens with the existing fonts, aligned menus, clear sharing controls and live traffic/status.
- Physical pairing and matching phone/forwarded internet probes passed on RMX3430; user reported working sharing. Controlled VPN, disconnect and cross-distribution tests remain pending.

## 0.2.0 — Android companion development milestone

- Signed Kotlin Android companion with visible Start/Stop service, cellular-bound TCP/UDP forwarding, Keystore TLS identity and authorized-ADB pairing.
- Desktop ADB phone discovery, explicit install/pair actions, pinned/authenticated USB bridge and owned NetworkManager/tun2proxy system tunnel.
- Offline Kotlin/Rust encrypted interoperability and raw TUN DNS tests. Phone installation/runtime acceptance was not performed after the user withdrew phone-operation authorization.


## 0.1.0 — 2026-10-24 (development)

- Original native Rust/Tokio/Slint Linux tethering manager and dark dashboard.
- NetworkManager D-Bus USB configuration with volatile profiles, Polkit authorization, per-interface checkpoint recovery and instance locking.
- Interface-bound DNS + TLS verification, public IP, actual interface transfer/session statistics, reconnect supervision and clean shutdown.
- Existing Wi-Fi/PAN observation and standard advertised-gateway HTTP CONNECT diagnostic.
- Private settings/session history, redacted log copy/export, native notifications/tray and per-user startup integration.
- Software tests, isolated NetworkManager contract test, private Sway UI harness, Linux packaging recipes and hardware acceptance protocol.
- Hardware forwarding, automatic Wi-Fi Direct/PAN setup, custom ADB companion, system-wide TUN-to-proxy and comprehensive leak protection remain incomplete/unverified.
