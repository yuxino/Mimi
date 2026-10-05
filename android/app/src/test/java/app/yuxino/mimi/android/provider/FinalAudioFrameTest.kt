package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.util.Base64

class FinalAudioFrameTest {
    private fun protocols(): List<ServiceProtocol> = listOf(
        GeminiProtocol(ServiceConfiguration(ServiceProvider.GEMINI, emptyMap()), "zh"),
        AzureProtocol(ServiceConfiguration(ServiceProvider.AZURE, emptyMap()), "zh"),
        VolcanoProtocol(ServiceConfiguration(ServiceProvider.VOLCANO, emptyMap()), "en", "zh"),
        TencentProtocol(ServiceConfiguration(ServiceProvider.TENCENT, mapOf("appId" to "1250000000", "secretId" to "AKIDEXAMPLE", "secretKey" to "secret-key")), "en", "zh"),
        BaiduProtocol(ServiceConfiguration(ServiceProvider.BAIDU, emptyMap()), "en", "zh"),
        GrokProtocol(ServiceConfiguration(ServiceProvider.XAI, emptyMap()), "zh", "en"),
    )

    @Test fun everyFixedFrameAdapterPreservesTheTailAndPadsOnlyWithSilence() {
        for (protocol in protocols()) {
            for (size in listOf(2, protocol.frameBytes - 2, protocol.frameBytes)) {
                val tail = ByteArray(size) { (it % 127 + 1).toByte() }
                val actual = pcm(protocol, requireNotNull(protocol.finalAudio(tail)))
                assertEquals(protocol.frameBytes, actual.size)
                assertArrayEquals(tail, actual.copyOf(size))
                assertTrue(actual.drop(size).all { it == 0.toByte() })
            }
            assertNull(protocol.finalAudio(ByteArray(0)))
            assertThrows(IllegalArgumentException::class.java) { protocol.finalAudio(ByteArray(protocol.frameBytes + 1)) }
        }
    }

    private fun pcm(protocol: ServiceProtocol, frame: WireFrame): ByteArray = when (protocol) {
        is TencentProtocol, is BaiduProtocol -> (frame as WireFrame.Binary).value
        is VolcanoProtocol -> VolcanoWire.fields(VolcanoWire.fields((frame as WireFrame.Binary).value).getValue(4).bytes).getValue(14).bytes
        is GeminiProtocol -> Base64.getDecoder().decode(JSONObject((frame as WireFrame.Text).value).getJSONObject("realtimeInput").getJSONObject("audio").getString("data"))
        else -> Base64.getDecoder().decode(JSONObject((frame as WireFrame.Text).value).getString("audio"))
    }
}
