package org.tetherust.companion

import java.io.*
import java.net.*

/** A captured Android network, or an explicitly configured local upstream proxy. */
class ForwardPath(
    val connect: (String, Int) -> Socket,
    val datagram: () -> DatagramSocket,
    val resolve: (String) -> InetAddress
)

/** No Android dependencies: shared by the real service and offline protocol tests. */
data class PathStatus(
    val epoch: Long,
    val vpn: Boolean,
    val ipv4Default: Boolean,
    val ipv6Default: Boolean,
    val blocked: Boolean,
    val available: Boolean,
    val mode: String = "default"
) {
    fun check(expected: Long, strict: Boolean) {
        require(expected == epoch) { "Phone network changed; reconnect explicitly" }
        require(available && !blocked) { "Android network unavailable or blocked" }
        if (strict) require(vpn && ipv4Default && mode == "default") { "VPN not applicable to companion, split routes, or proxy-only mode" }
    }
    fun json() = "{\"protocol\":2,\"epoch\":$epoch,\"vpn\":$vpn,\"ipv4_default\":$ipv4Default,\"ipv6_default\":$ipv6Default,\"blocked\":$blocked,\"available\":$available,\"mode\":\"$mode\",\"udp_supported\":${mode == "default"}}"
}

/** Explicit loopback proxy. Destination names are resolved by the proxy, never by direct fallback. */
class LocalProxy(private val kind: String, private val port: Int) {
    init { require(kind == "socks5" || kind == "http"); require(port in 1..65535) }
    fun connect(host: String, targetPort: Int): Socket {
        require(host.isNotEmpty() && host.length <= 253 && host.none { it <= ' ' || it == '/' || it == '@' })
        require(targetPort in 1..65535)
        val socket = Socket(Proxy.NO_PROXY)
        try {
            socket.connect(InetSocketAddress(InetAddress.getByAddress(byteArrayOf(127,0,0,1)), port), 10000)
            socket.soTimeout = 10000
            val input = DataInputStream(socket.inputStream); val output = DataOutputStream(socket.outputStream)
            if (kind == "socks5") {
                output.write(byteArrayOf(5,1,0)); output.flush()
                require(input.readUnsignedByte() == 5 && input.readUnsignedByte() == 0) { "Local SOCKS proxy must accept no-auth localhost clients" }
                output.write(byteArrayOf(5,1,0,3,host.length.toByte())); output.write(host.toByteArray(Charsets.US_ASCII)); output.writeShort(targetPort); output.flush()
                require(input.readUnsignedByte() == 5 && input.readUnsignedByte() == 0 && input.readUnsignedByte() == 0)
                ProxyEngine.readAddress(input)
            } else {
                val target = if (host.contains(':')) "[$host]:$targetPort" else "$host:$targetPort"
                output.write("CONNECT $target HTTP/1.1\r\nHost: $target\r\n\r\n".toByteArray(Charsets.US_ASCII)); output.flush()
                val header = ByteArrayOutputStream()
                while (header.size() < 8192) {
                    val b = input.read(); require(b >= 0); header.write(b)
                    val bytes = header.toByteArray(); val n = bytes.size
                    if (n >= 4 && bytes[n-4] == 13.toByte() && bytes[n-3] == 10.toByte() && bytes[n-2] == 13.toByte() && bytes[n-1] == 10.toByte()) break
                }
                val response = header.toString("US-ASCII")
                require(response.endsWith("\r\n\r\n") && response.lineSequence().first().matches(Regex("HTTP/1\\.[01] 200(?: .*|)"))) { "Local HTTP proxy refused CONNECT" }
            }
            return socket
        } catch (e: Exception) { socket.close(); throw e }
    }
}

/** Independent phone-side HTTPS probe over the same captured path. Does not alter VPN configuration. */
object ExitProbe {
    fun run(path: ForwardPath): String {
        val host = "one.one.one.one"
        val raw = path.connect(host,443)
        try {
            val tls = (javax.net.ssl.SSLSocketFactory.getDefault() as javax.net.ssl.SSLSocketFactory).createSocket(raw,host,443,true) as javax.net.ssl.SSLSocket
            tls.use {
                it.soTimeout=10000
                it.sslParameters = it.sslParameters.apply { endpointIdentificationAlgorithm="HTTPS" }
                it.startHandshake()
                it.outputStream.write("GET /cdn-cgi/trace HTTP/1.0\r\nHost: $host\r\nConnection: close\r\n\r\n".toByteArray(Charsets.US_ASCII)); it.outputStream.flush()
                val bytes=ByteArrayOutputStream(); val buffer=ByteArray(1024)
                while(bytes.size()<=16384) { val n=it.inputStream.read(buffer); if(n<0)break; bytes.write(buffer,0,n) }
                require(bytes.size()<=16384)
                return parse(bytes.toByteArray())
            }
        } finally {raw.close()}
    }
    fun parse(bytes: ByteArray): String {
        require(bytes.size<=16384)
        val response=bytes.toString(Charsets.US_ASCII)
        val separator=response.indexOf("\r\n\r\n"); require(separator>=0)
        val header=response.substring(0,separator)
        require(header.lineSequence().first().matches(Regex("HTTP/1\\.[01] 200(?: .*|)")))
        val fields=header.lineSequence().drop(1).mapNotNull { line-> val at=line.indexOf(':'); if(at<0)null else Pair(line.substring(0,at).lowercase(),line.substring(at+1).trim().lowercase()) }.toMap()
        val bodyBytes=bytes.copyOfRange(separator+4,bytes.size)
        val body=if(fields["transfer-encoding"] == "chunked") {
            val input=DataInputStream(ByteArrayInputStream(bodyBytes));val decoded=ByteArrayOutputStream()
            fun line(): String { val out=ByteArrayOutputStream(); while(out.size()<256) { val b=input.readUnsignedByte(); if(b==13){require(input.readUnsignedByte()==10);return out.toString("US-ASCII")};out.write(b) };throw IOException("Chunk line too large") }
            while(true) { val n=line().substringBefore(';').toInt(16); require(n in 0..16384 && decoded.size()+n<=16384);if(n==0)break;val chunk=ByteArray(n);input.readFully(chunk);decoded.write(chunk);require(input.readUnsignedByte()==13&&input.readUnsignedByte()==10) }
            decoded.toString("US-ASCII")
        } else { require(fields["transfer-encoding"] == null);fields["content-length"]?.let { require(it.toInt()==bodyBytes.size) };bodyBytes.toString(Charsets.US_ASCII) }
        val ip=body.lineSequence().firstOrNull { it.startsWith("ip=") }?.removePrefix("ip=")?.trim() ?: throw IOException("HTTPS probe has no IP")
        require(ip.matches(Regex("[0-9a-fA-F:.]{2,45}")))
        return InetAddress.getByName(ip).hostAddress!!
    }

}
