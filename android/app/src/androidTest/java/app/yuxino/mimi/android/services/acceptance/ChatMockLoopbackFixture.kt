package app.yuxino.mimi.android.services.acceptance

import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/** Fixed synthetic HTTP exchange, bound to device loopback and never to external interfaces. */
internal class ChatMockLoopbackFixture : Closeable {
    private class Exchange(val model: String = "", val status: Int = 200, val mayCancel: Boolean = false,
        val deepLX: Boolean = false, val bodyCode: Int = 200) {
        val received = CountDownLatch(1)
        val release = CountDownLatch(1)
        val completed = CountDownLatch(1)
    }

    private val exchanges = listOf(
        Exchange("synthetic-success"),
        Exchange("synthetic-slow", mayCancel = true),
        Exchange("synthetic-auth", status = 401),
        Exchange(deepLX = true),
        Exchange(deepLX = true, bodyCode = 456),
        Exchange(deepLX = true, status = 429),
    )
    private val server = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1")).apply { soTimeout = 10_000 }
    val baseUrl = "http://127.0.0.1:${server.localPort}/v1"
    val deepLXUrl = "http://127.0.0.1:${server.localPort}/translate"
    @Volatile private var failure: Throwable? = null
    @Volatile private var closed = false
    @Volatile private var activeSocket: Socket? = null
    private val worker = thread(name = "chatmock-loopback-fixture", isDaemon = true) {
        try {
            for (exchange in exchanges) {
                try {
                    server.accept().use { socket ->
                        activeSocket = socket
                        socket.soTimeout = 5_000
                        verifyRequest(socket, exchange)
                        exchange.received.countDown()
                        check(exchange.release.await(10, TimeUnit.SECONDS)) { "Fixture response was never released" }
                        try { respond(socket, exchange) }
                        catch (error: IOException) { if (!exchange.mayCancel) throw error }
                    }
                } finally {
                    activeSocket = null
                    exchange.completed.countDown()
                }
            }
        } catch (error: Throwable) {
            if (!closed) failure = error
            exchanges.forEach { it.received.countDown(); it.completed.countDown() }
        }
    }

    fun awaitRequest(index: Int) {
        check(exchanges[index].received.await(8, TimeUnit.SECONDS)) { "Local HTTP request did not reach fixture" }
        assertHealthy()
    }

    fun releaseResponse(index: Int) { exchanges[index].release.countDown() }

    fun awaitResponse(index: Int) {
        check(exchanges[index].completed.await(8, TimeUnit.SECONDS)) { "Local fixture did not finish its response" }
        assertHealthy()
    }

    fun assertHealthy() { failure?.let { throw it } }

    private fun verifyRequest(socket: Socket, exchange: Exchange) {
        val input = BufferedInputStream(socket.getInputStream())
        check(line(input) == "POST ${if (exchange.deepLX) "/translate" else "/v1/chat/completions"} HTTP/1.1") { "Unexpected translation request method or path" }
        val headers = mutableMapOf<String, String>()
        var headerBytes = 0
        while (true) {
            val header = line(input)
            if (header.isEmpty()) break
            headerBytes += header.length
            check(headerBytes <= 16_384) { "Fixture request headers exceeded the limit" }
            val separator = header.indexOf(':')
            check(separator > 0) { "Malformed fixture request header" }
            headers[header.substring(0, separator).lowercase()] = header.substring(separator + 1).trim()
        }
        if (exchange.deepLX) check(headers["authorization"] == "Bearer synthetic-deeplx-local-token") { "DeepLX request used the wrong provider token" }
        else check("authorization" !in headers) { "Keyless fixture received an authorization credential" }
        val size = headers["content-length"]?.toIntOrNull() ?: error("Fixture request has no bounded body length")
        check(size in 1..8_192) { "Fixture request body exceeded the limit" }
        val body = ByteArray(size)
        var count = 0
        while (count < size) {
            val read = input.read(body, count, size - count)
            check(read > 0) { "Fixture request ended early" }
            count += read
        }
        val json = JSONObject(body.toString(Charsets.UTF_8))
        if (exchange.deepLX) {
            check(json.getString("text") == "Hello." && json.getString("source_lang") == "EN" && json.getString("target_lang") == "ZH") {
                "DeepLX check did not use its fixed example and language protocol"
            }
            check(!json.has("model") && !json.has("messages")) { "DeepLX received an OpenAI request" }
            return
        }
        check(json.getString("model") == exchange.model) { "Connection check ignored the draft model" }
        check(!json.getBoolean("stream")) { "Connection check unexpectedly requested streaming" }
        val messages = json.getJSONArray("messages")
        check(messages.length() == 2 && messages.getJSONObject(0).getString("role") == "system") {
            "Connection check did not use the translation request shape"
        }
        check(messages.getJSONObject(1).getString("role") == "user" &&
            messages.getJSONObject(1).getString("content") == "Hello.") { "Connection check sent content other than its fixed example" }
    }

    private fun line(input: BufferedInputStream): String {
        val bytes = ByteArrayOutputStream()
        while (true) {
            val next = input.read()
            check(next >= 0) { "Fixture request headers ended early" }
            if (next == '\n'.code) break
            check(bytes.size() < 8_192) { "Fixture request header line exceeded the limit" }
            bytes.write(next)
        }
        return bytes.toString(Charsets.US_ASCII.name()).removeSuffix("\r")
    }

    private fun respond(socket: Socket, exchange: Exchange) {
        val status = exchange.status
        val body = (if (exchange.deepLX)
            """{"code":${exchange.bodyCode},"data":"Synthetic translation."}"""
        else if (status == 200)
            """{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Synthetic translation."}}]}"""
        else """{"error":{"message":"synthetic-server-detail-must-not-display"}}""").toByteArray(Charsets.UTF_8)
        val reason = when (status) { 200 -> "OK"; 429 -> "Too Many Requests"; else -> "Unauthorized" }
        val headers = "HTTP/1.1 $status $reason\r\nContent-Type: application/json; charset=utf-8\r\n" +
            "Content-Length: ${body.size}\r\nConnection: close\r\n\r\n"
        socket.getOutputStream().apply { write(headers.toByteArray(Charsets.US_ASCII)); write(body); flush() }
    }

    override fun close() {
        closed = true
        exchanges.forEach { it.release.countDown() }
        runCatching { activeSocket?.close() }
        server.close()
        worker.join(2_000)
        check(!worker.isAlive) { "Local HTTP fixture thread did not stop" }
    }
}
