package org.tetherust.companion
import java.io.*
import java.net.*
import java.util.concurrent.atomic.AtomicReference

object ProxyChecks {
 fun run() {
    val plain="HTTP/1.0 200 OK\r\n\r\nip=203.0.113.5\n".toByteArray()
    check(ExitProbe.parse(plain)=="203.0.113.5")
    val body="ip=203.0.113.5\n"
    val chunks="HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n${body.length.toString(16)}\r\n$body\r\n0\r\n\r\n"
    check(ExitProbe.parse(chunks.toByteArray())=="203.0.113.5")
    check(runCatching {ExitProbe.parse("HTTP/1.0 500 Error\r\n\r\nip=203.0.113.5\n".toByteArray())}.isFailure)
    for(kind in arrayOf("socks5","http")) {
        val listener=ServerSocket(0,1,InetAddress.getByName("127.0.0.1"));val failure=AtomicReference<Throwable?>()
        val thread=Thread {
            try {listener.accept().use { client->
                client.soTimeout=3000
                val input=DataInputStream(client.inputStream);val output=DataOutputStream(client.outputStream)
                if(kind=="socks5") {
                    check(input.readUnsignedByte()==5&&input.readUnsignedByte()==1&&input.readUnsignedByte()==0)
                    output.write(byteArrayOf(5,0));output.flush()
                    check(input.readUnsignedByte()==5&&input.readUnsignedByte()==1&&input.readUnsignedByte()==0)
                    val address=ProxyEngine.readAddress(input);check(address==Pair("echo.test",443))
                    output.write(byteArrayOf(5,0,0,1,127,0,0,1,0,0));output.flush()
                } else {
                    val request=ByteArrayOutputStream()
                    while(!request.toString("US-ASCII").endsWith("\r\n\r\n")){check(request.size()<1024);request.write(input.readUnsignedByte())}
                    check(request.toString("US-ASCII").startsWith("CONNECT echo.test:443 HTTP/1.1\r\n"))
                    output.write("HTTP/1.1 200 Connection established\r\n\r\n".toByteArray());output.flush()
                }
                val bytes=ByteArray(4);input.readFully(bytes);output.write(bytes);output.flush()
            }} catch(e:Throwable){failure.set(e)} finally {listener.close()}
        }.apply{isDaemon=true;start()}
        LocalProxy(kind,listener.localPort).connect("echo.test",443).use {socket->socket.outputStream.write("echo".toByteArray());val reply=ByteArray(4);DataInputStream(socket.inputStream).readFully(reply);check(reply.toString(Charsets.US_ASCII)=="echo")}
        thread.join(4000);check(!thread.isAlive);failure.get()?.let {throw it}
    }
 }
}
