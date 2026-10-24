package org.tetherust.companion
import java.io.File
import java.net.*
import java.security.KeyStore
import java.util.Base64
import javax.net.ssl.*

// Host-only fixture. No Android runtime, ADB, real phone or external internet is used.
object JvmServer {
 @JvmStatic fun main(args: Array<String>) {
    ProxyChecks.run()
    val store=KeyStore.getInstance("PKCS12").apply { File(args[0]).inputStream().use { load(it,"test-password".toCharArray()) } }
    val manager=KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(store,"test-password".toCharArray()) }
    val tls=SSLContext.getInstance("TLS").apply { init(manager.keyManagers,null,null) }
    val loop=InetAddress.getByName("127.0.0.1")
    val tcp=ServerSocket(0,16,loop); val udp=DatagramSocket(0,loop)
    Thread { while (!tcp.isClosed) { try { val client=tcp.accept(); Thread { client.use { val input=it.inputStream;val output=it.outputStream;val buffer=ByteArray(32768);while(true){val n=input.read(buffer);if(n<0)break;output.write(buffer,0,n)} } }.apply {isDaemon=true;start()} }catch(_:Exception){break} } }.apply {isDaemon=true;start()}
    Thread { val buffer=ByteArray(65507);while(!udp.isClosed){try{val packet=DatagramPacket(buffer,buffer.size);udp.receive(packet);udp.send(packet)}catch(_:Exception){break}} }.apply {isDaemon=true;start()}
    val token="0123456789abcdef".repeat(4)
    val state=File(args[1]+".state").apply {writeText("vpn")}
    fun status(): PathStatus { val mode=state.readText().trim(); return PathStatus(if(mode=="changed") 2 else 1,mode!="excluded",mode!="split",true,mode=="blocked",true) }
    val path=ForwardPath(connect={host,port->require(host=="echo.test"&&port==tcp.localPort);Socket(loop,port)},datagram={DatagramSocket(0,loop)},resolve={host->require(host=="echo.test"||host=="127.0.0.1");loop})
    val engine=ProxyEngine(tls,token,pathStatus={status()},openPath={epoch,strict->status().check(epoch,strict);path},destinationAllowed={it.isLoopbackAddress},exitProbe={"203.0.113.123"})
    engine.start(0)
    Thread { var previous="vpn"; while(true) { Thread.sleep(20); val current=state.readText(); if (current!=previous) {previous=current;engine.invalidateClients()} } }.apply {isDaemon=true;start()}
    val cert=Base64.getEncoder().encodeToString(store.getCertificate("test").encoded)
    File(args[1]).writeText("{\"version\":1,\"name\":\"Host test fixture\",\"token\":\"$token\",\"certificate\":\"$cert\",\"tls_port\":${engine.boundPort},\"tcp_port\":${tcp.localPort},\"udp_port\":${udp.localPort}}")
    try { System.`in`.read() } finally {engine.close();tcp.close();udp.close()}
 }
}
