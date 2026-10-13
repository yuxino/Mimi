package app.yuxino.mimi.android.provider

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.URI
import java.nio.file.Files
import java.security.KeyStore
import java.util.Base64
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import kotlin.concurrent.thread

/** Production JNI text probes send all requests through a private loopback proxy. */
class NativeTranslationThinkingTest {
    @Test fun sharedThinkingOptionsReachActualJniHttpRequests() {
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("thinkingRequestOptions")
        val hosts = (0 until cases.length()).map { URI(cases.getJSONObject(it).getString("endpoint")).host }.distinct()
        Loopback(hosts).use { fixture ->
            for (index in 0 until cases.length()) {
                val case = cases.getJSONObject(index)
                val id = case.getString("id")
                val configuration = buildTextProbeConfiguration(TranslationConfiguration(
                    endpoint = case.getString("endpoint"), model = case.getString("model"),
                    apiKey = "synthetic-text-key", allowLocalHttp = true,
                ), "en", "zh", "lite")
                val handle = NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "create_text_probe")
                    .put("configuration", configuration).put("network", fixture.network)).getLong("handle")
                try {
                    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
                    var result: JSONObject? = null
                    while (System.nanoTime() < deadline) {
                        fixture.checkFailure()
                        val state = NativeRuntimeConfiguration.command("text_probe_poll", handle)
                        if (state.optBoolean("finished")) { result = state.getJSONObject("result"); break }
                        Thread.sleep(10)
                    }
                    assertNotNull("$id probe did not finish", result)
                    assertEquals(id, "available", result!!.getString("service"))
                    val request = requireNotNull(fixture.request.getAndSet(null)) { "$id request not captured" }
                    assertEquals(id, case.getString("model"), request.getString("model"))
                    assertFalse(id, request.getBoolean("stream"))
                    assertEquals(id, 2, request.getJSONArray("messages").length())
                    request.remove("model"); request.remove("stream"); request.remove("messages")
                    val expected = case.getJSONObject("expected")
                    assertEquals(id, expected.keySet(), request.keySet())
                    for (key in expected.keySet()) assertEquals(id, expected.get(key).toString(), request.get(key).toString())
                    assertFalse(id, result.toString().contains("synthetic-text-key"))
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("text_probe_cancel", handle) }
            }
        }
    }

    private class Loopback(hosts: List<String>) : AutoCloseable {
        private val directory = Files.createTempDirectory("mimi-thinking-jni-").toFile()
        private val server = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))
        private val failure = AtomicReference<Throwable?>()
        val request = AtomicReference<JSONObject?>()
        private var socket: Socket? = null
        @Volatile private var closed = false
        val network: JSONObject
        private val worker: Thread

        init {
            val keytool = java.io.File(System.getProperty("java.home"), "bin/keytool${if (System.getProperty("os.name").orEmpty().startsWith("Windows")) ".exe" else ""}")
            val storeFile = java.io.File(directory, "fixture.p12")
            val process = ProcessBuilder(keytool.absolutePath, "-genkeypair", "-alias", "fixture", "-keyalg", "EC", "-groupname", "secp256r1",
                "-dname", "CN=synthetic-translation-fixture", "-ext", "SAN=" + hosts.filter { it != "127.0.0.1" }.joinToString(",") { "dns:$it" },
                "-ext", "EKU=serverAuth", "-validity", "2", "-storetype", "PKCS12", "-keystore", storeFile.absolutePath,
                "-storepass", "mimi-fixture", "-keypass", "mimi-fixture", "-noprompt")
                .redirectErrorStream(true).redirectOutput(java.io.File(directory, "keytool.log")).start()
            check(process.waitFor(20, TimeUnit.SECONDS)) { process.destroyForcibly(); "Synthetic TLS identity timed out" }
            check(process.exitValue() == 0) { "Synthetic TLS identity failed" }
            val password = "mimi-fixture".toCharArray()
            val store = KeyStore.getInstance("PKCS12").apply { storeFile.inputStream().use { load(it, password) } }
            val managers = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(store, password) }
            val tls = SSLContext.getInstance("TLS").apply { init(managers.keyManagers, null, null) }
            network = JSONObject().put("speechProxy", JSONObject().put("mode", "direct").put("url", JSONObject.NULL))
                .put("textProxy", JSONObject().put("mode", "custom").put("url", "http://127.0.0.1:${server.localPort}"))
                .put("trustRoots", JSONArray(listOf(Base64.getEncoder().encodeToString(store.getCertificate("fixture").encoded))))
            worker = thread(name = "mimi-thinking-loopback", isDaemon = true) {
                try {
                    while (!closed) {
                        val tcp = server.accept().also { socket = it; it.soTimeout = 5_000 }
                        val first = headers(DataInputStream(tcp.getInputStream()))
                        val secure: Socket
                        val header: String
                        if (first.startsWith("CONNECT ")) {
                            val host = first.substringAfter("CONNECT ").substringBefore(':')
                            check(host in hosts)
                            tcp.getOutputStream().write("HTTP/1.1 200 Connection Established\r\n\r\n".toByteArray(Charsets.US_ASCII))
                            secure = (tls.socketFactory.createSocket(tcp, host, 443, true) as SSLSocket).apply { useClientMode = false; startHandshake() }
                            socket = secure
                            header = headers(DataInputStream(secure.inputStream))
                        } else { secure = tcp; header = first }
                        secure.use {
                            check(header.startsWith("POST "))
                            val length = header.lineSequence().first { it.startsWith("content-length:", true) }.substringAfter(':').trim().toInt()
                            check(length in 1..65_536)
                            val body = ByteArray(length).also { DataInputStream(secure.inputStream).readFully(it) }
                            request.set(JSONObject(body.toString(Charsets.UTF_8)))
                            val response = "{\"choices\":[{\"message\":{\"content\":\"Synthetic translation.\"},\"finish_reason\":\"stop\"}]}"
                            secure.getOutputStream().write(("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ${response.length}\r\nConnection: close\r\n\r\n$response").toByteArray(Charsets.US_ASCII))
                        }
                        socket = null
                    }
                } catch (error: Throwable) { if (!closed) failure.set(error) }
            }
        }

        fun checkFailure() { failure.get()?.let { throw AssertionError("Synthetic text transport failed", it) } }
        private fun headers(input: DataInputStream): String {
            val bytes = ByteArrayOutputStream()
            var tail = 0
            while (bytes.size() < 32_768) {
                val byte = input.readUnsignedByte(); bytes.write(byte); tail = (tail shl 8) or byte
                if (tail == 0x0d0a0d0a) return bytes.toString("US-ASCII")
            }
            error("Oversized loopback HTTP header")
        }
        override fun close() {
            closed = true; socket?.close(); server.close(); worker.join(2_000)
            check(!worker.isAlive) { "Synthetic transport did not stop" }
            directory.deleteRecursively()
        }
    }
}
