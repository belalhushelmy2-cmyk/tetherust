package org.tetherust.companion

import android.app.*
import android.content.*
import android.net.*
import android.net.nsd.*
import android.os.*
import java.net.*
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicLong

object SharingState {
    @Volatile var running = false
    @Volatile var status = "Stopped"
    @Volatile var vpn = "VPN forwarding unverified"
    val received = AtomicLong(); val sent = AtomicLong()
    @Volatile var connections = 0
}
class SharingService : Service() {
    private lateinit var manager: ConnectivityManager
    private val lock = Any()
    private var network: Network? = null
    private var capabilities: NetworkCapabilities? = null
    private var properties: LinkProperties? = null
    private var blocked = false
    private var epoch = SecureRandom().nextLong().and(Long.MAX_VALUE).coerceAtLeast(1)
    private var fingerprint = ""
    private var engine: ProxyEngine? = null
    private var registered = false
    private var advertisement: NsdManager.RegistrationListener? = null
    private var mode = "default"
    private var proxy: LocalProxy? = null
    @Volatile private var failure: String? = null
    private fun publishPath() {
        val next = "$network|${capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_VPN)}|${capabilities?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)}|$blocked|${properties?.routes}|${properties?.dnsServers}|${properties?.httpProxy}"
        if (next != fingerprint) { fingerprint = next; epoch++; engine?.invalidateClients() }
        val status = pathStatus()
        SharingState.vpn = when {
            mode != "default" -> "Explicit local proxy · TCP only · VPN exit unverified"
            status.vpn && status.ipv4Default -> "VPN applies to companion · exit unverified"
            status.vpn -> "Split VPN routes · strict sharing unavailable"
            else -> "No VPN applies to this app; it may be excluded"
        }
        SharingState.status = when {
            status.blocked -> "Android blocks this app’s network access"
            !status.available -> "Waiting for Android’s app-default network"
            SharingState.connections > 0 -> "Laptop linked · ${if (mode == "default") "app-default network" else "local proxy"}"
            else -> "Ready · waiting for laptop"
        }
    }
    private fun defaultRoutes(bytes: Int): Boolean {
        val routes = properties?.routes?.filter { it.type == RouteInfo.RTN_UNICAST && it.destination.address.address.size == bytes } ?: return false
        return routes.any { it.destination.prefixLength == 0 } ||
            (routes.any { it.destination.prefixLength == 1 && it.destination.address.address[0].toInt().and(255) == 0 } &&
             routes.any { it.destination.prefixLength == 1 && it.destination.address.address[0].toInt().and(255) == 128 })
    }
    private fun pathStatus(): PathStatus = synchronized(lock) {
        PathStatus(epoch, capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true,
            defaultRoutes(4), defaultRoutes(16), blocked, network != null && capabilities != null, mode)
    }
    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(value: Network) = synchronized(lock) { network = value; capabilities = null; properties = null; blocked = false; publishPath() }
        override fun onCapabilitiesChanged(value: Network, caps: NetworkCapabilities) = synchronized(lock) { if (network == value) { capabilities = caps; publishPath() } }
        override fun onLinkPropertiesChanged(value: Network, links: LinkProperties) = synchronized(lock) { if (network == value) { properties = links; publishPath() } }
        override fun onBlockedStatusChanged(value: Network, isBlocked: Boolean) = synchronized(lock) { if (network == value) { blocked = isBlocked; publishPath() } }
        override fun onLost(value: Network) = synchronized(lock) { if (network == value) { network = null; capabilities = null; properties = null; publishPath() } }
    }
    /** Capture the app's current network. Never request a cellular underlay or use protect(). */
    private fun openPath(expected: Long, strict: Boolean): ForwardPath = synchronized(lock) {
        pathStatus().check(expected, strict)
        val selected = network ?: throw SocketException("No app network")
        val ipv6Allowed = !strict || defaultRoutes(16)
        fun permitted(address: InetAddress): Boolean = !address.isAnyLocalAddress && !address.isLoopbackAddress && !address.isMulticastAddress && (ipv6Allowed || address.address.size == 4)
        fun guard() = synchronized(lock) { pathStatus().check(expected, strict) }
        val upstream = proxy
        if (upstream != null) return@synchronized ForwardPath(
            connect = { host, port -> guard(); val socket = upstream.connect(host, port); try { guard(); socket } catch (e: Exception) { socket.close(); throw e } },
            datagram = { throw SocketException("Local proxy mode supports TCP only") },
            resolve = { throw SocketException("Proxy resolves hostnames remotely") })
        ForwardPath(
            connect = { host, port ->
                guard()
                val addresses = selected.getAllByName(host).filter(::permitted)
                var result: Socket? = null
                for (address in addresses) {
                    guard()
                    val socket = selected.socketFactory.createSocket()
                    try { socket.connect(InetSocketAddress(address, port), 10000); guard(); result = socket; break }
                    catch (_: Exception) { socket.close() }
                }
                result ?: throw SocketException("App-default network destination unavailable")
            },
            datagram = {
                guard()
                val socket = DatagramSocket(null)
                try { selected.bindSocket(socket); socket.bind(InetSocketAddress(0)); guard(); socket }
                catch (e: Exception) { socket.close(); throw e }
            },
            resolve = { host -> guard(); selected.getAllByName(host).firstOrNull(::permitted) ?: throw SocketException("Destination family unavailable through selected network") })
    }
    override fun onBind(intent: Intent?) = null
    override fun onCreate() {
        super.onCreate(); manager = getSystemService(ConnectivityManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("sharing", "USB internet sharing", NotificationManager.IMPORTANCE_LOW))
    }
    override fun onStartCommand(intent: Intent?, flags: Int, id: Int): Int {
        if (intent?.action == "STOP") { stopSelf(); return START_NOT_STICKY }
        if (SharingState.running) return START_NOT_STICKY
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 1, Intent(this, SharingService::class.java).setAction("STOP"), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val builder = if (Build.VERSION.SDK_INT >= 26) Notification.Builder(this,"sharing") else Notification.Builder(this)
        startForeground(1, builder.setSmallIcon(android.R.drawable.stat_sys_upload_done).setContentTitle("Tetherust USB sharing")
            .setContentText("Uses this app’s Android network; VPN compatibility is conditional").setContentIntent(open).setOngoing(true).addAction(android.R.drawable.ic_menu_close_clear_cancel,"Stop sharing",stop).build())
        SharingState.status = "Waiting for app-default network"; SharingState.running = true
        try {
            val preferences = getSharedPreferences("routing", MODE_PRIVATE)
            mode = preferences.getString("mode", "default") ?: "default"
            if (mode != "default") proxy = LocalProxy(mode, preferences.getInt("proxy_port", 1080))
            // API 24+: callback explicitly includes the VPN if it applies to this app.
            manager.registerDefaultNetworkCallback(callback); registered = true
            engine = ProxyEngine(Identity.tls(this), Identity.token(this), ::pathStatus, ::openPath,
                onError = { failure = it; SharingState.status = it; stopSelf() },
                onClientCount = { count -> SharingState.connections = count; synchronized(lock) { if (SharingState.running) publishPath() } }, received = SharingState.received, sent = SharingState.sent).also { it.start(wifi = preferences.getBoolean("wifi", false)) }
            if (preferences.getBoolean("wifi", false)) {
                val listener = object : NsdManager.RegistrationListener {
                    override fun onServiceRegistered(info: NsdServiceInfo) {}
                    override fun onRegistrationFailed(info: NsdServiceInfo, code: Int) { SharingState.status = "Wi-Fi discovery unavailable; enter this phone’s LAN IP:38473 manually" }
                    override fun onServiceUnregistered(info: NsdServiceInfo) {}
                    override fun onUnregistrationFailed(info: NsdServiceInfo, code: Int) {}
                }
                advertisement = listener
                val info = NsdServiceInfo().apply { serviceName="Tetherust"; serviceType="_tetherust._tcp."; port=38473; setAttribute("protocol","2") }
                getSystemService(NsdManager::class.java).registerService(info, NsdManager.PROTOCOL_DNS_SD, listener)
            }
        } catch (_: Exception) { failure = "Sharing could not start. Check app network access and local proxy settings."; SharingState.status = failure!!; stopSelf() }
        return START_NOT_STICKY
    }
    override fun onDestroy() {
        SharingState.running = false
        synchronized(lock) { network = null; capabilities = null; properties = null; epoch++ }
        advertisement?.let { try { getSystemService(NsdManager::class.java).unregisterService(it) } catch (_: Exception) {} }; advertisement = null
        engine?.close(); engine = null
        if (registered) try { manager.unregisterNetworkCallback(callback) } catch (_: Exception) {}
        SharingState.connections = 0; SharingState.status = failure ?: "Stopped"; SharingState.vpn = "VPN forwarding unverified"
        super.onDestroy()
    }
}
