package org.tetherust.companion

import java.io.*
import java.net.*
import java.security.MessageDigest
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Semaphore
import java.util.concurrent.atomic.AtomicLong
import javax.net.ssl.*

/** Socket forwarding, not raw Android packet routing. TLS + RFC1929 authentication precedes every stream.
 * cmd=3 is an explicitly negotiated private UDP-over-TLS extension, never exposed as ordinary SOCKS UDP.
 * The Linux bridge translates it to standard localhost SOCKS5 UDP associations.
 */
class ProxyEngine(
    private val tls: SSLContext,
    private val token: String,
    private val pathStatus: () -> PathStatus,
    private val openPath: (Long, Boolean) -> ForwardPath,
    private val exitProbe: (ForwardPath) -> String = { ExitProbe.run(it) },
    private val onError: (String) -> Unit = {},
    private val onClientCount: (Int) -> Unit = {},
    private val destinationAllowed: (InetAddress) -> Boolean = { !it.isLoopbackAddress && !it.isAnyLocalAddress && !it.isMulticastAddress },
    val received: AtomicLong = AtomicLong(),
    val sent: AtomicLong = AtomicLong()
) : Closeable {
    @Volatile private var running = false
    private var server: SSLServerSocket? = null
    private val clients = java.util.Collections.newSetFromMap(ConcurrentHashMap<Closeable, Boolean>())
    private val permits = Semaphore(64)
    private val authenticatedStreams = java.util.concurrent.atomic.AtomicInteger()
    val active: Int get() = clients.size
    val boundPort: Int get() = server?.localPort ?: 0
    fun start(port: Int = 38473, wifi: Boolean = false) {
        check(!running)
        val listener = tls.serverSocketFactory.createServerSocket() as SSLServerSocket
        listener.reuseAddress = true
        listener.enabledProtocols = arrayOf("TLSv1.2")
        listener.bind(InetSocketAddress(InetAddress.getByName(if (wifi) "0.0.0.0" else "127.0.0.1"), port), 32)
        server = listener; running = true
        worker("accept") {
            while (running) {
                val client = try { listener.accept() as SSLSocket } catch (e: IOException) { if (running) onError("USB listener stopped"); break }
                if (!permits.tryAcquire()) { client.close(); continue }
                clients.add(client)
                worker("client") {
                    try { client.soTimeout = 10000; client.startHandshake(); handle(client) }
                    catch (_: Exception) { /* No destinations, tokens or private network data in status/logs. */ }
                    finally { clients.remove(client); try { client.close() } catch (_: Exception) {}; permits.release() }
                }
            }
        }
    }
    private fun worker(name: String, task: () -> Unit) = Thread(null, Runnable(task), "tetherust-$name", 256 * 1024L).apply { isDaemon = true; start() }
    private fun handle(client: SSLSocket) {
        val input = DataInputStream(client.inputStream); val output = DataOutputStream(client.outputStream)
        require(input.readUnsignedByte() == 5)
        val methods = ByteArray(input.readUnsignedByte()); input.readFully(methods)
        if (!methods.contains(2.toByte())) { output.write(byteArrayOf(5, -1)); output.flush(); return }
        output.write(byteArrayOf(5, 2)); output.flush()
        require(input.readUnsignedByte() == 1)
        val user = ByteArray(input.readUnsignedByte()); input.readFully(user)
        val password = ByteArray(input.readUnsignedByte()); input.readFully(password)
        val authenticated = MessageDigest.isEqual(user, "tetherust".toByteArray()) && MessageDigest.isEqual(password, token.toByteArray())
        output.write(byteArrayOf(1, if (authenticated) 0 else 1)); output.flush()
        if (!authenticated) return
        synchronized(authenticatedStreams) { onClientCount(authenticatedStreams.incrementAndGet()) }
        try {
        require(input.readUnsignedByte() == 5)
        var command = input.readUnsignedByte(); require(input.readUnsignedByte() == 0)
        var destination = readAddress(input)
        if (command == 240) {
            val bytes = pathStatus().json().toByteArray(Charsets.UTF_8)
            output.writeShort(bytes.size); output.write(bytes); output.flush(); return
        }
        require(command == 241) { "Update the Linux client: routing policy negotiation required" }
        val strict = input.readUnsignedByte(); require(strict in 0..1)
        val epoch = input.readLong()
        val path = openPath(epoch, strict == 1)
        require(input.readUnsignedByte() == 5)
        command = input.readUnsignedByte(); require(input.readUnsignedByte() == 0)
        destination = readAddress(input)
        when (command) {
            242 -> {
                val ip = exitProbe(path)
                require(ip.matches(Regex("[0-9a-fA-F:.]{2,45}")))
                val bytes = "{\"exit_ip\":\"$ip\"}".toByteArray(Charsets.UTF_8)
                output.writeShort(bytes.size); output.write(bytes); output.flush()
            }
            1 -> {
                val upstream = try { path.connect(destination.first, destination.second) } catch (_: Exception) { reply(output, 4); return }
                clients.add(upstream)
                try {
                    upstream.tcpNoDelay = true; client.tcpNoDelay = true
                    client.soTimeout = 600000; upstream.soTimeout = 600000
                    reply(output, 0)
                    val reverse = worker("download") {
                        try { pump(upstream.inputStream, output, received) }
                        catch (_: Exception) {}
                        finally { try { client.close() } catch (_: Exception) {} }
                    }
                    try { pump(input, upstream.outputStream, sent); try { upstream.shutdownOutput() } catch (_: Exception) {}; reverse.join() }
                    finally { upstream.close(); reverse.join(2000) }
                } finally { clients.remove(upstream) }
            }
            // This is an authenticated custom stream extension. Only our Linux bridge sends it.
            3 -> udp(client, input, output, path)
            else -> reply(output, 7)
        }
        } finally { synchronized(authenticatedStreams) { onClientCount(authenticatedStreams.decrementAndGet()) } }
    }
    private fun pump(input: InputStream, output: OutputStream, count: AtomicLong) {
        val buffer = ByteArray(32768)
        while (running) { val n = input.read(buffer); if (n < 0) break; output.write(buffer, 0, n); count.addAndGet(n.toLong()) }
    }
    private fun udp(client: SSLSocket, input: DataInputStream, output: DataOutputStream, path: ForwardPath) {
        val socket = path.datagram(); socket.soTimeout = 1000
        clients.add(socket)
        val contacted = ConcurrentHashMap<InetSocketAddress, Long>()
        try {
            reply(output, 0); client.soTimeout = 60000
            val reverse = worker("udp-download") {
                val buffer = ByteArray(65507)
                while (running && !socket.isClosed) {
                    try {
                        val packet = DatagramPacket(buffer, buffer.size); socket.receive(packet)
                        val source = InetSocketAddress(packet.address, packet.port)
                        val last = contacted[source] ?: continue
                        if (System.currentTimeMillis() - last > 60000) { contacted.remove(source); continue }
                        val body = ByteArrayOutputStream(); val data = DataOutputStream(body)
                        data.writeShort(0); data.writeByte(0); writeAddress(data, packet.address, packet.port); data.write(buffer, 0, packet.length)
                        val bytes = body.toByteArray()
                        if (bytes.size > 65535) continue
                        synchronized(output) { output.writeShort(bytes.size); output.write(bytes); output.flush() }
                        received.addAndGet(packet.length.toLong())
                    } catch (_: SocketTimeoutException) {} catch (_: Exception) { break }
                }
            }
            try {
                while (running) {
                    val length = input.readUnsignedShort(); require(length in 8..65535)
                    val frame = ByteArray(length); input.readFully(frame)
                    val payload = DataInputStream(ByteArrayInputStream(frame)); require(payload.readUnsignedShort() == 0 && payload.readUnsignedByte() == 0)
                    val destination = readAddress(payload); val address = path.resolve(destination.first)
                    require(destinationAllowed(address))
                    val endpoint = InetSocketAddress(address, destination.second)
                    if (contacted.size >= 128 && !contacted.containsKey(endpoint)) continue
                    contacted[endpoint] = System.currentTimeMillis()
                    val bytes = payload.readBytes(); require(bytes.size <= 65507)
                    socket.send(DatagramPacket(bytes, bytes.size, endpoint)); sent.addAndGet(bytes.size.toLong())
                }
            } finally { socket.close(); reverse.join(2000) }
        } finally { socket.close(); clients.remove(socket) }
    }
    fun invalidateClients() {
        clients.forEach { try { it.close() } catch (_: Exception) {} }
    }
    override fun close() {
        running = false; try { server?.close() } catch (_: Exception) {}
        clients.forEach { try { it.close() } catch (_: Exception) {} }; clients.clear()
    }
    companion object {
        fun readAddress(input: DataInputStream): Pair<String, Int> {
            val host = when (input.readUnsignedByte()) {
                1 -> ByteArray(4).also { input.readFully(it) }.let { InetAddress.getByAddress(it).hostAddress!! }
                4 -> ByteArray(16).also { input.readFully(it) }.let { InetAddress.getByAddress(it).hostAddress!! }
                3 -> { val n = input.readUnsignedByte(); require(n in 1..253); ByteArray(n).also { input.readFully(it) }.toString(Charsets.US_ASCII).also { require(it.all { c -> c.isLetterOrDigit() || c == '.' || c == '-' }) } }
                else -> throw IOException("Unsupported address")
            }
            return Pair(host, input.readUnsignedShort())
        }
        fun writeAddress(output: DataOutputStream, address: InetAddress, port: Int) { val bytes = address.address; output.writeByte(if (bytes.size == 4) 1 else 4); output.write(bytes); output.writeShort(port) }
        private fun reply(output: DataOutputStream, status: Int) { output.write(byteArrayOf(5, status.toByte(), 0, 1, 127, 0, 0, 1, 0, 0)); output.flush() }
    }
}
