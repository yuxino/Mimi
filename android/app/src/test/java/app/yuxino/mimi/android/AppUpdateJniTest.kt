package app.yuxino.mimi.android

import app.yuxino.mimi.android.provider.NativeRuntimeConfiguration
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.io.InputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketTimeoutException
import java.nio.file.Files
import java.security.KeyStore
import java.util.Base64
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import kotlin.concurrent.thread

/** Fixed production destination through an explicitly trusted loopback TLS proxy; no Internet. */
class AppUpdateJniTest {
    private fun request(operation: String, handle: Long? = null) = JSONObject().put("operation", operation).apply {
        handle?.let { put("handle", it) }
    }

    @Test fun actualJniRejectsArbitraryDestinationsCredentialsAndInvalidTickets() {
        assertEquals("https://api.github.com/repos/yuxino/Mimi/releases/latest",
            NativeRuntimeConfiguration.exchange(request("app_update_endpoint")).getString("endpoint"))
        for (input in listOf(
            request("app_update_endpoint").put("endpoint", "https://untrusted.invalid"),
            request("app_update_start").put("configuration", JSONObject().put("apiKey", "synthetic")),
            request("app_update_start").put("network", JSONObject().put("proxy", JSONObject().put("mode", "direct"))
                .put("trustRoots", JSONArray())),
            request("app_update_poll", Long.MAX_VALUE),
        )) {
            assertEquals("app_update_invalid_request", assertThrows(IllegalArgumentException::class.java) {
                NativeRuntimeConfiguration.exchange(input)
            }.message)
        }
    }

    @Test fun actualJniCompletesOneFixedRequestAndConsumesItsResponse() {
        TlsProxy(hold = false).use { proxy ->
            val handle = NativeRuntimeConfiguration.exchange(request("app_update_start").put("network", proxy.network)).getLong("handle")
            try {
                assertTrue(proxy.requestReceived.await(5, TimeUnit.SECONDS))
                val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
                var finished: JSONObject? = null
                while (System.nanoTime() < deadline) {
                    val result = NativeRuntimeConfiguration.exchange(request("app_update_poll", handle))
                    if (result.getBoolean("finished")) { finished = result; break }
                    Thread.sleep(10)
                }
                assertEquals("{}", checkNotNull(finished).getString("response"))
                assertThrows(IllegalArgumentException::class.java) { NativeRuntimeConfiguration.exchange(request("app_update_poll", handle)) }
                proxy.checkFailure()
            } finally { NativeRuntimeConfiguration.exchange(request("app_update_cancel", handle)) }
        }
    }

    @Test fun actualJniCancellationAbortsTheHeldTransportAndRemovesItsTicket() {
        TlsProxy(hold = true).use { proxy ->
            val handle = NativeRuntimeConfiguration.exchange(request("app_update_start").put("network", proxy.network)).getLong("handle")
            try {
                assertTrue(proxy.requestReceived.await(5, TimeUnit.SECONDS))
                assertFalse(NativeRuntimeConfiguration.exchange(request("app_update_poll", handle)).getBoolean("finished"))
                NativeRuntimeConfiguration.exchange(request("app_update_cancel", handle))
                assertTrue("Cancelled transport remained open", proxy.closed.await(3, TimeUnit.SECONDS))
                assertThrows(IllegalArgumentException::class.java) { NativeRuntimeConfiguration.exchange(request("app_update_poll", handle)) }
                proxy.checkFailure()
            } finally { NativeRuntimeConfiguration.exchange(request("app_update_cancel", handle)) }
        }
    }

    private class TlsProxy(private val hold: Boolean) : AutoCloseable {
        private val directory = Files.createTempDirectory("mimi-update-jni-").toFile()
        private val server = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))
        private val failure = AtomicReference<Throwable?>()
        private val socket = AtomicReference<Socket?>()
        val requestReceived = CountDownLatch(1)
        val closed = CountDownLatch(1)
        val network: JSONObject
        private val worker: Thread

        init {
            val keytool = java.io.File(System.getProperty("java.home"),
                "bin/keytool${if (System.getProperty("os.name").orEmpty().startsWith("Windows")) ".exe" else ""}")
            val keyStoreFile = java.io.File(directory, "fixture.p12")
            val process = ProcessBuilder(keytool.absolutePath, "-genkeypair", "-alias", "fixture", "-keyalg", "EC", "-groupname", "secp256r1",
                "-dname", "CN=api.github.com", "-ext", "SAN=dns:api.github.com", "-ext", "EKU=serverAuth", "-validity", "2",
                "-storetype", "PKCS12", "-keystore", keyStoreFile.absolutePath, "-storepass", "synthetic-fixture", "-noprompt")
                .redirectErrorStream(true).redirectOutput(java.io.File(directory, "keytool.log")).start()
            check(process.waitFor(15, TimeUnit.SECONDS) && process.exitValue() == 0)
            val store = KeyStore.getInstance("PKCS12").apply { keyStoreFile.inputStream().use { load(it, "synthetic-fixture".toCharArray()) } }
            val managers = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(store, "synthetic-fixture".toCharArray()) }
            val tls = SSLContext.getInstance("TLS").apply { init(managers.keyManagers, null, null) }
            network = JSONObject().put("proxy", JSONObject().put("mode", "custom").put("url", "http://127.0.0.1:${server.localPort}"))
                .put("trustRoots", JSONArray(listOf(Base64.getEncoder().encodeToString(store.getCertificate("fixture").encoded))))
            worker = thread(name = "mimi-update-jni-loopback", isDaemon = true) {
                try {
                    server.soTimeout = 5_000
                    server.accept().use { raw ->
                        socket.set(raw)
                        raw.soTimeout = 5_000
                        check(headers(raw.getInputStream()).startsWith("CONNECT api.github.com:443 HTTP/1.1\r\n"))
                        raw.getOutputStream().write("HTTP/1.1 200 Connection Established\r\n\r\n".toByteArray())
                        raw.getOutputStream().flush()
                        (tls.socketFactory.createSocket(raw, "api.github.com", 443, true) as SSLSocket).use { secure ->
                            secure.useClientMode = false
                            secure.soTimeout = 5_000
                            secure.startHandshake()
                            val header = headers(secure.inputStream)
                            check(header.startsWith("GET /repos/yuxino/Mimi/releases/latest HTTP/1.1\r\n"))
                            check(!header.lowercase().contains("authorization:"))
                            requestReceived.countDown()
                            if (hold) {
                                try { check(secure.inputStream.read() == -1) }
                                catch (error: SocketTimeoutException) { throw error }
                                catch (_: java.io.IOException) { /* Aborting TLS may close without close_notify. */ }
                            } else {
                                secure.outputStream.write("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".toByteArray())
                                secure.outputStream.flush()
                            }
                        }
                    }
                } catch (error: Throwable) { failure.set(error) }
                finally { closed.countDown() }
            }
        }

        fun checkFailure() { failure.get()?.let { throw AssertionError("Loopback update request failed", it) } }
        override fun close() {
            server.close()
            socket.get()?.close()
            worker.join(5_000)
            check(!worker.isAlive)
            directory.deleteRecursively()
            checkFailure()
        }

        private fun headers(input: InputStream): String {
            val buffer = StringBuilder()
            while (buffer.length < 8_192 && !buffer.endsWith("\r\n\r\n")) {
                val next = input.read()
                check(next >= 0)
                buffer.append(next.toChar())
            }
            check(buffer.endsWith("\r\n\r\n"))
            return buffer.toString()
        }
    }
}
