package app.yuxino.mimi.android.provider

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.io.DataInputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.KeyStore
import java.security.MessageDigest
import java.util.Base64
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager
import kotlin.concurrent.thread

/** Synthetic PCM and loopback WebSocket frames enter the production Rust engine. */
class CustomRecognitionRuntimeJniTest {
    private fun cases() = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
        .bufferedReader().use { it.readText() }).getJSONArray("customRecognitionCompletions")

    private fun configuration(fixture: Loopback) = JSONObject()
        .put("provider", if (fixture.audio3) "customDashScopeASR" else "customOpenAIASR").put("qwenMtModel", "lite")
        .put("credentials", JSONObject().put("kind", "customSpeech").put("endpoint", fixture.endpoint)
            .put("model", "synthetic-model").put("apiKey", "synthetic-only"))
        .put("textCredentials", JSONObject.NULL).put("sourceLanguage", "en").put("targetLanguage", "original")
        .put("translationMode", "turbo").put("networkProxy", choice()).put("textNetworkProxy", choice())

    private fun choice() = JSONObject().put("mode", "direct").put("url", JSONObject.NULL)
    private val network by lazy {
        val factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm())
        factory.init(null as KeyStore?)
        val roots = factory.trustManagers.filterIsInstance<X509TrustManager>().single().acceptedIssuers
        JSONObject().put("speechProxy", choice()).put("textProxy", choice())
            .put("trustRoots", JSONArray(roots.map { Base64.getEncoder().encodeToString(it.encoded) }))
    }

    private fun create(fixture: Loopback) = NativeRuntimeConfiguration.exchange(JSONObject()
        .put("operation", "create").put("configuration", configuration(fixture)).put("network", network)).getLong("handle")

    private fun awaitState(fixture: Loopback, handle: Long, predicate: (JSONObject) -> Boolean): JSONObject {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(8)
        while (System.nanoTime() < deadline) {
            fixture.checkFailure()
            val state = NativeRuntimeConfiguration.command("poll", handle)
            assertTrue(state.toString(), state.isNull("errorCode"))
            if (predicate(state)) return state
            Thread.sleep(10)
        }
        error("Synthetic custom recognition state did not arrive")
    }

    private fun sendTurn(fixture: Loopback, handle: Long, count: Int) {
        val pcm = ByteArray(31 * 960)
        for (index in 0 until 960 step 2) { pcm[index] = 0xb8.toByte(); pcm[index + 1] = 0x0b }
        assertTrue(NativeRuntimeConfiguration.pcmRaw(handle, pcm))
        fixture.awaitCommits(count)
    }

    private fun confirmedSources(state: JSONObject): List<String> {
        val history = state.getJSONObject("snapshot").getJSONArray("history")
        return (0 until history.length()).map { history.getJSONObject(it).getString("source") }
    }

    @Test fun authoritativeCompletionContractsRetirePreviewsAcrossActualJni() {
        val cases = cases()
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            Loopback().use { fixture ->
                val handle = create(fixture)
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    awaitState(fixture, handle) { it.getString("connectionStatus") == "listening" }
                    sendTurn(fixture, handle, 1)
                    fixture.delta("turn-1", case.getString("draft"))
                    awaitState(fixture, handle) { it.getJSONObject("snapshot").getJSONObject("source").getString("text") == case.getString("draft") }
                    if (!case.isNull("completion")) fixture.complete("turn-1", case.getString("completion"))
                    NativeRuntimeConfiguration.command("finish", handle)
                    val finished = awaitState(fixture, handle) { it.getBoolean("finished") }
                    val expected = case.getJSONObject("expected").getJSONArray("confirmedSources")
                    assertEquals(case.getString("id"), (0 until expected.length()).map(expected::getString), confirmedSources(finished))
                    assertEquals(case.getString("id"), expected.length() > 0, finished.getJSONObject("snapshot").getBoolean("displayPairFinal"))
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        }
    }

    @Test fun olderEmptyCompletionKeepsTheNewerMissingFinalStopTailAcrossActualJni() {
        val empty = cases().let { values -> (0 until values.length()).map(values::getJSONObject)
            .single { it.getString("id") == "empty-completion-retires-preview" } }
        Loopback().use { fixture ->
            val handle = create(fixture)
            try {
                NativeRuntimeConfiguration.command("start", handle)
                awaitState(fixture, handle) { it.getString("connectionStatus") == "listening" }
                sendTurn(fixture, handle, 1); sendTurn(fixture, handle, 2)
                val newer = "Synthetic newer stop tail."
                fixture.delta("turn-2", newer)
                awaitState(fixture, handle) { it.getJSONObject("snapshot").getJSONObject("source").getString("text") == newer }
                fixture.complete("turn-1", empty.getString("completion"))
                NativeRuntimeConfiguration.command("finish", handle)
                val finished = awaitState(fixture, handle) { it.getBoolean("finished") }
                assertEquals(listOf(newer), confirmedSources(finished))
                assertTrue(finished.getJSONObject("snapshot").getBoolean("displayPairFinal"))
            } finally { NativeRuntimeConfiguration.command("stop", handle) }
        }
    }

    @Test fun sharedFinishBarrierPreservesLatestTailAndAuthoritativeFinalAcrossActualJni() {
        val contract = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("asrFinishBarrier")
        val ids = setOf("latest-unidentified-draft", "authoritative-unidentified-final")
        val selected = (0 until contract.length()).map(contract::getJSONObject)
            .filter { it.getString("id") in ids }
        assertEquals(ids, selected.map { it.getString("id") }.toSet())
        for (case in selected) {
            Loopback().use { fixture ->
                val handle = create(fixture)
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    awaitState(fixture, handle) { it.getString("connectionStatus") == "listening" }
                    sendTurn(fixture, handle, 1)
                    var previousDraft = ""
                    val events = case.getJSONArray("events")
                    for (index in 0 until events.length()) {
                        val event = events.getJSONObject(index)
                        when (event.getString("kind")) {
                            "sourceDraft" -> {
                                val text = event.getString("text")
                                assertTrue("${case.getString("id")}: cumulative draft", text.startsWith(previousDraft))
                                fixture.delta("turn-1", text.removePrefix(previousDraft))
                                previousDraft = text
                                // Rust queue tests deterministically hold drafts behind the
                                // finish barrier. This JNI case verifies the actual adapter's
                                // latest-tail fallback without depending on thread scheduling.
                                awaitState(fixture, handle) {
                                    it.getJSONObject("snapshot").getJSONObject("source").getString("text") == text
                                }
                            }
                            "sourceFinal" -> {
                                val text = event.getString("text")
                                fixture.complete("turn-1", text)
                                awaitState(fixture, handle) { confirmedSources(it) == listOf(text) }
                            }
                            "sessionFinished" -> NativeRuntimeConfiguration.command("finish", handle)
                            else -> error("Unsupported loopback finish barrier event")
                        }
                    }
                    val finished = awaitState(fixture, handle) { it.getBoolean("finished") }
                    val expected = case.getJSONArray("expected")
                    val retained = (0 until expected.length()).map(expected::getJSONObject)
                        .filter { it.getString("kind") in setOf("sourceDraft", "sourceFinal") }
                        .map { it.getString("text") }
                    assertEquals(case.getString("id"), retained, confirmedSources(finished))
                    assertTrue(case.getString("id"), finished.getJSONObject("snapshot").getBoolean("displayPairFinal"))
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        }
    }

    @Test fun audio3FinishDrainsDelayedTailWithOrWithoutProviderAcknowledgementAcrossActualJni() {
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("audio3FinishDrain")
        assertTrue("Missing Audio3 finish fixtures", cases.length() > 0)
        for (index in 0 until cases.length()) {
            val case = cases.getJSONObject(index)
            Loopback(case).use { fixture ->
                val handle = create(fixture)
                try {
                    NativeRuntimeConfiguration.command("start", handle)
                    awaitState(fixture, handle) {
                        it.getJSONObject("snapshot").getJSONObject("source").getString("text") == case.getString("initialDraft")
                    }
                    NativeRuntimeConfiguration.command("finish", handle)
                    val finished = awaitState(fixture, handle) { it.getBoolean("finished") }
                    assertTrue(case.getString("id"), fixture.finishSeen.await(1, TimeUnit.SECONDS))
                    val expected = case.getJSONArray("confirmedSources")
                    assertEquals(case.getString("id"), (0 until expected.length()).map(expected::getString), confirmedSources(finished))
                    assertTrue(case.getString("id"), fixture.transportClosed.await(3, TimeUnit.SECONDS))
                    fixture.checkFailure()
                } finally { NativeRuntimeConfiguration.command("stop", handle) }
            }
        }
    }

    private class Loopback(private val audio3Finish: JSONObject? = null) : AutoCloseable {
        val audio3 = audio3Finish != null
        private val server = ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))
        val endpoint = "ws://127.0.0.1:${server.localPort}/v1/realtime"
        val finishSeen = CountDownLatch(1)
        val transportClosed = CountDownLatch(1)
        private val ready = CountDownLatch(1)
        private val commits = AtomicInteger()
        private val failure = AtomicReference<Throwable?>()
        private var socket: Socket? = null
        private var output: OutputStream? = null
        @Volatile private var closed = false
        private val worker = thread(name = "mimi-custom-recognition-loopback", isDaemon = true) {
            try {
                server.soTimeout = 5_000
                val connection = server.accept().also { socket = it; it.soTimeout = 5_000 }
                val input = DataInputStream(connection.inputStream)
                val wire = connection.outputStream.also { output = it }
                val request = headers(input)
                check(request.startsWith(if (audio3) "GET /v1/realtime HTTP/1.1\r\n"
                    else "GET /v1/realtime?intent=transcription HTTP/1.1\r\n"))
                val key = request.lineSequence().first { it.startsWith("sec-websocket-key:", true) }.substringAfter(':').trim()
                val accept = Base64.getEncoder().encodeToString(MessageDigest.getInstance("SHA-1")
                    .digest((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").toByteArray(Charsets.US_ASCII)))
                wire.write("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: $accept\r\n\r\n".toByteArray(Charsets.US_ASCII))
                val setup = JSONObject(readText(input, wire))
                if (audio3) {
                    audio3Session(setup, input, wire, requireNotNull(audio3Finish))
                    return@thread
                }
                check(setup.getString("type") == "session.update")
                frame(wire, 1, setup.put("type", "session.updated").toString().toByteArray(Charsets.UTF_8))
                ready.countDown()
                while (true) {
                    val text = readText(input, wire) ?: break
                    val event = JSONObject(text)
                    when (event.getString("type")) {
                        "input_audio_buffer.append" -> Unit
                        "input_audio_buffer.commit" -> {
                            val count = commits.get() + 1
                            send(JSONObject().put("type", "input_audio_buffer.committed").put("item_id", "turn-$count"))
                            commits.incrementAndGet()
                        }
                        else -> error("Unexpected synthetic custom client frame")
                    }
                }
            } catch (error: Throwable) { if (!closed) failure.set(error) }
        }

        private fun audio3Session(setup: JSONObject, input: DataInputStream, wire: OutputStream, case: JSONObject) {
            check(setup.getJSONObject("header").getString("action") == "run-task")
            val taskId = setup.getJSONObject("header").getString("task_id")
            fun event(name: String) = JSONObject().put("header", JSONObject().put("event", name).put("task_id", taskId))
            fun sentence(id: Int, text: String, isFinal: Boolean) = event("result-generated").put("payload", JSONObject()
                .put("output", JSONObject().put("sentence", JSONObject().put("sentence_id", id).put("text", text).put("sentence_end", isFinal))))
            fun sendFrame(event: JSONObject) = frame(wire, 1, event.toString().toByteArray(Charsets.UTF_8))
            sendFrame(event("task-started"))
            ready.countDown()
            sendFrame(sentence(1, case.getString("initialDraft"), false))
            val finish = JSONObject(requireNotNull(readText(input, wire)))
            check(finish.getJSONObject("header").getString("action") == "finish-task")
            check(finish.getJSONObject("header").getString("task_id") == taskId)
            finishSeen.countDown()
            // The recognizer produces its final update only after Stop. Keep
            // this within the production budget; Rust tests control deadlines.
            Thread.sleep(100)
            val sentences = case.getJSONArray("finishSentences")
            for (index in 0 until sentences.length()) {
                val value = sentences.getJSONObject(index)
                sendFrame(sentence(value.getInt("id"), value.getString("text"), value.getBoolean("final")))
            }
            if (case.getBoolean("acknowledge")) sendFrame(event("task-finished"))
            check(readText(input, wire) == null) { "Unexpected command after Audio3 finish" }
            transportClosed.countDown()
        }

        fun checkFailure() { failure.get()?.let { throw AssertionError("Synthetic custom recognition transport failed", it) } }
        fun awaitCommits(count: Int) {
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
            while (System.nanoTime() < deadline) {
                checkFailure(); if (commits.get() >= count) return; Thread.sleep(10)
            }
            error("Synthetic PCM turn was not committed")
        }
        fun delta(id: String, text: String) = send(JSONObject().put("type", "conversation.item.input_audio_transcription.delta").put("item_id", id).put("delta", text))
        fun complete(id: String, text: String) = send(JSONObject().put("type", "conversation.item.input_audio_transcription.completed").put("item_id", id).put("transcript", text))
        private fun send(event: JSONObject) {
            check(ready.await(5, TimeUnit.SECONDS)); checkFailure()
            frame(requireNotNull(output), 1, event.toString().toByteArray(Charsets.UTF_8))
        }
        private fun headers(input: DataInputStream): String {
            val bytes = java.io.ByteArrayOutputStream(); var tail = 0
            while (bytes.size() < 16_384) {
                val byte = input.readUnsignedByte(); bytes.write(byte); tail = (tail shl 8) or byte
                if (tail == 0x0d0a0d0a) return bytes.toString("US-ASCII")
            }
            error("Oversized loopback HTTP header")
        }
        private fun frame(output: OutputStream, opcode: Int, bytes: ByteArray) = synchronized(output) {
            require(bytes.size < 65_536)
            output.write(0x80 or opcode)
            if (bytes.size < 126) output.write(bytes.size) else {
                output.write(126); output.write(bytes.size shr 8); output.write(bytes.size and 255)
            }
            output.write(bytes); output.flush()
        }
        private fun readText(input: DataInputStream, output: OutputStream): String? {
            while (true) {
                val opcode = input.readUnsignedByte() and 15
                val flags = input.readUnsignedByte(); check(flags and 128 != 0)
                val size = when (val short = flags and 127) { 126 -> input.readUnsignedShort(); 127 -> error("Oversized loopback frame"); else -> short }
                check(size <= 65_535)
                val mask = ByteArray(4).also(input::readFully)
                val payload = ByteArray(size).also(input::readFully)
                for (index in payload.indices) payload[index] = (payload[index].toInt() xor mask[index % 4].toInt()).toByte()
                when (opcode) {
                    8 -> return null
                    9 -> frame(output, 10, payload)
                    1 -> return payload.toString(Charsets.UTF_8)
                    2 -> check(audio3) { "Unexpected binary custom recognition frame" }
                    else -> error("Unexpected loopback opcode")
                }
            }
        }
        override fun close() {
            closed = true; socket?.close(); server.close(); worker.join(2_000)
            check(!worker.isAlive) { "Synthetic custom transport did not stop" }
        }
    }
}
