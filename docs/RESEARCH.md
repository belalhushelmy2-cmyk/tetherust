# Connection research — 2026-10-24

Reviewed the requested sources and NetworkManager's primary API documentation. This implementation is original Rust code; no third-party tunnel codec was copied.

- [PdaNet official site](https://pdanet.co/): advertises USB, Wi-Fi and Bluetooth DUN. Bluetooth DUN is not Bluetooth PAN. Its billing, speed and compatibility marketing is not a guarantee for this app. The www hostname timed out; the apex site was accessible.
- [xsqu1znt/PdaNetClientCLI-Linux](https://github.com/xsqu1znt/PdaNetClientCLI-Linux): ADB packet tunnel in Python for USB; HTTP proxy through redsocks for Wi-Fi; nftables and dnscrypt-proxy integration. Its README explicitly limits Wi-Fi forwarding to TCP/DNS, and reports testing only on the author's Fedora system. No license file was displayed in the reviewed top-level listing, so no code is reused. The binary framing is an implementation-specific protocol, not a portable Android standard.
- [phantomic12/pdanet-linux](https://github.com/phantomic12/pdanet-linux): MIT; discovers a Wi-Fi gateway HTTP proxy on port 8000 and configures proxy-aware applications. This does not establish arbitrary UDP support or universal system routing. No code reused.
- [wtyler2505/pdanet-linux](https://github.com/wtyler2505/pdanet-linux): MIT; Python/GTK and transparent proxy approaches, with global TTL, IPv6 and other network changes. Those changes are unnecessary for native USB tethering and are not adopted. Its carrier-detection claims have not been verified. No code reused.

## Chosen USB path

Android exposes a USB network interface when its built-in tethering toggle is enabled. Detect USB ancestry and a compatible kernel driver (rndis_host/cdc_ncm/cdc_ether; ipheth also recognized as a USB candidate), then configure DHCP/IPv6 using NetworkManager. A driver alone cannot prove Android identity; display the actual USB product and require selection. No ADB, phone root, companion app, userspace packet loop, Linux IP forwarding or NAT is needed: Linux is an ordinary client and Android forwards the packets.

[NetworkManager AddAndActivateConnection2](https://networkmanager.dev/docs/api/latest/gdbus-org.freedesktop.NetworkManager.html) supports persist=volatile and bind-activation=dbus-client. Use these for automatic cleanup, and a 90-second checkpoint restricted to the selected interface for restoring its previous connection. Renew the checkpoint every 20 seconds. Do not use global checkpoint flags that destroy someone else's checkpoint or disconnect unrelated devices.

[IPv4 settings](https://networkmanager.dev/docs/api/latest/settings-ipv4.html): negative DNS priority excludes less-preferred DNS configurations. A route-only '~.' search domain asks the resolver to prefer this link. This depends on the configured resolver and other active profiles; applications with private DoH/resolvers and VPN policy routes require separate handling. This release makes no universal leak-prevention claim.

[NetworkManager checkpoint implementation](https://github.com/NetworkManager/NetworkManager/blob/main/src/core/nm-checkpoint-manager.c): a rollback consumes its checkpoint, including when per-device results indicate failure. Report unconfirmed recovery truthfully; do not claim an automatic retry after a consumed checkpoint.

## Other transports

Existing NetworkManager Wi-Fi and Bluetooth PAN links can be monitored and independently verified. Join/pair and enable phone tethering using desktop network settings. Wi-Fi Direct capability does not imply a usable phone network; neither an SSID nor a fixed gateway is assumed. The explicit proxy probe uses only the selected Wi-Fi interface's advertised gateway, verifies HTTP CONNECT plus TLS, and changes no routing. Port 8000 is an explicit diagnostic default, not a universal protocol assumption.

A full TUN-to-proxy transport, secure custom ADB companion, and in-app Bluetooth pairing are not implemented. There is no placeholder packet-forwarder. An Android companion is unnecessary for the native USB path; implementing a custom proxy later requires authenticated pairing, cellular-bound Android sockets and a clear TCP/UDP contract. Do not claim normal Android apps can raw-route device packets.

## 0.2.0 companion implementation references

- [Android Network API](https://developer.android.com/reference/android/net/Network): per-network socket factories, name resolution and datagram binding support cellular-bound app sockets, rather than claiming a normal app can transparently route arbitrary Android raw packets.
- [Android network state](https://developer.android.com/develop/connectivity/network-ops/reading-network-state): request/callback lifecycle and CHANGE_NETWORK_STATE for a requested network.
- [NetworkManager tun setting](https://networkmanager.dev/docs/api/latest/settings-tun.html): explicit owner/group, TUN mode and no packet-information header.
- [tun2proxy](https://github.com/tun2proxy/tun2proxy), version 0.8.6, MIT: now used as a pinned Rust dependency for the packet stack, SOCKS5 TCP/UDP and virtual DNS. Its automatic root/global resolv.conf setup is disabled; our NetworkManager-owned profile configures the new interface. License notices are included in packages.
- Kotlin standard library 2.0.20, Apache-2.0: used by the APK; its license and the application's GPL license are embedded as APK assets.

## Existing Android VPN routing (0.3.0)

- Android [registerDefaultNetworkCallback](https://developer.android.com/reference/android/net/ConnectivityManager#registerDefaultNetworkCallback(android.net.ConnectivityManager.NetworkCallback)) reports the application's default network, including a VPN that applies to it. Supported since API 24; this companion minimum is now 24.
- [Network socket factories/bindSocket/getAllByName](https://developer.android.com/reference/android/net/Network) capture that Network; socket factories cease to work when it disappears. This replaces the old intentional cellular-underlay selection.
- Android [VPN guidance](https://developer.android.com/develop/connectivity/vpn) allows one active VpnService per user/profile and documents per-app inclusion/exclusion and always-on/lockdown restrictions. Tetherust creates no VpnService and cannot inspect a third-party VPN's internal proxy/destination rules.
- [NetworkCallback](https://developer.android.com/reference/android/net/ConnectivityManager.NetworkCallback) delivers capability/link/blocked/loss events; callback code consumes supplied objects rather than querying synchronous manager state inside callbacks.
- [nftables manual](https://netfilter.org/projects/nftables/manpage.html) describes inet-family output hooks and drop policies used by the fixed optional Linux guard. No host firewall test is implied by namespace acceptance.

Inference from these supported APIs: binding to the app-default VPN Network can share its socket path when the VPN includes the companion, but cannot guarantee every VPN client's routing policy. Physical A/B/C testing is required; no named third-party VPN compatibility is claimed.
