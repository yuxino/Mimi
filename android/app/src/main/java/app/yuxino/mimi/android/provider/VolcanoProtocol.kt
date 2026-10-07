package app.yuxino.mimi.android.provider

import okhttp3.Request
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.util.UUID

/** The same minimal protobuf field contract as core/protocols/volcano_engine.rs. */
internal object VolcanoWire {
    fun varint(value: Long): ByteArray {
        var n = value; val out = ByteArrayOutputStream()
        do { var b = (n and 127).toInt(); n = n ushr 7; if (n != 0L) b = b or 128; out.write(b) } while(n != 0L)
        return out.toByteArray()
    }
    fun number(field: Int, value: Long) = varint((field.toLong() shl 3)) + varint(value)
    fun bytes(field: Int, value: ByteArray) = varint((field.toLong() shl 3) or 2) + varint(value.size.toLong()) + value
    fun string(field: Int, value: String) = bytes(field, value.toByteArray())
    fun envelope(session: String, event: Int) = bytes(1, string(6, session)) + number(2, event.toLong())
    class Field(val wire: Int, val number: Long = 0, val bytes: ByteArray = byteArrayOf())
    fun fields(data: ByteArray): Map<Int, Field> {
        require(data.size <= 1024 * 1024)
        var cursor = 0
        fun readVarint(): Long {
            var value = 0L
            for (i in 0..9) {
                require(cursor < data.size)
                val b = data[cursor++].toInt() and 255
                if (i == 9) require(b <= 1)
                value = value or ((b and 127).toLong() shl (7 * i))
                if (b and 128 == 0) return value
            }
            error("invalid_varint")
        }
        val result = mutableMapOf<Int, Field>()
        while (cursor < data.size) {
            val key = readVarint(); require(key > 0 && key ushr 3 <= 536870911)
            val id = (key ushr 3).toInt(); val wire = (key and 7).toInt()
            val field = when (wire) {
                0 -> Field(wire, readVarint())
                2 -> {
                    val size = readVarint(); require(size >= 0 && size <= data.size - cursor)
                    val end = cursor + size.toInt()
                    Field(wire, bytes = data.copyOfRange(cursor, end)).also { cursor = end }
                }
                1, 5 -> { val size = if (wire == 1) 8 else 4; require(cursor + size <= data.size); cursor += size; Field(wire) }
                else -> error("unsupported_wire")
            }
            // Every field used in TranslateResponse and ResponseMeta is singular.
            if (id in 1..6) require(id !in result)
            result[id] = field
        }
        return result
    }
    fun string(field: Field): String {
        require(field.wire == 2 && field.bytes.size <= 128 * 1024)
        return Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(field.bytes)).toString()
    }
}

internal class VolcanoProtocol(private val config: ServiceConfiguration, private val source: String, private val target: String,
    private val session: String = UUID.randomUUID().toString()) : ServiceProtocol {
    override val frameBytes = 2560
    override fun request() = Request.Builder().url("wss://openspeech.bytedance.com/api/v4/ast/v2/translate")
        .header("X-Api-Key", config.value("apiKey")).header("X-Api-Resource-Id", "volc.service_type.10053").build()
    override fun setup(): WireFrame {
        require(target in VOLCANO_LANGUAGE_PAIRS[source].orEmpty()) { "unsupported_language" }
        val audio = VolcanoWire.string(4,"wav") + VolcanoWire.string(5,"raw") + VolcanoWire.number(7,16000) + VolcanoWire.number(8,16) + VolcanoWire.number(9,1)
        val translation = VolcanoWire.string(1,"s2t") + VolcanoWire.string(2,volcanoWireLanguage(source)) + VolcanoWire.string(3,volcanoWireLanguage(target))
        return WireFrame.Binary(VolcanoWire.envelope(session,100) + VolcanoWire.bytes(4,audio) + VolcanoWire.bytes(6,translation))
    }
    override fun audio(data: ByteArray): WireFrame {
        require(data.size == frameBytes)
        return WireFrame.Binary(VolcanoWire.envelope(session,200) + VolcanoWire.bytes(4,VolcanoWire.bytes(14,data)))
    }
    override fun finish() = WireFrame.Binary(VolcanoWire.envelope(session,102))
    override fun binary(value: ByteArray): List<ServiceEvent> {
        val fields = VolcanoWire.fields(value)
        val event = requireNotNull(fields[2]); require(event.wire == 0 && event.number in 0..Int.MAX_VALUE.toLong())
        fields[1]?.let { require(it.wire == 2 && it.bytes.size <= 64*1024); VolcanoWire.fields(it.bytes) }
        val text = fields[4]?.let { VolcanoWire.string(it) }
        return when(event.number.toInt()) {
            150 -> listOf(ServiceEvent.Ready)
            152 -> listOf(ServiceEvent.Closed)
            153 -> error("provider_error")
            650, 653 -> emptyList()
            651, 652 -> listOf(ServiceEvent.Source(requireNotNull(text), event.number == 652L, source))
            654, 655 -> listOf(ServiceEvent.Translation(requireNotNull(text), event.number == 655L))
            else -> emptyList()
        }
    }
}
