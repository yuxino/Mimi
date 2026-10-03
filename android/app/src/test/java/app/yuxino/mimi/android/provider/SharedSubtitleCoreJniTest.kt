package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

/** Exercises the actual host cdylib and JNI string ABI, with no provider/audio. */
class SharedSubtitleCoreJniTest {
    private fun operation(type: String) = JSONObject().put("type", type)
    private fun create() = SharedSubtitleCore.exchange(
        JSONObject().put("operation", operation("create").put("history_limit", 2))
    )

    @Test fun nativeStateIsExplicitAndIndependentAcrossCallers() {
        val first = create()
        val text = "Synthetic 日本語 😀"
        val request = JSONObject().put("state", first.getString("state")).put(
            "operation", operation("apply").put("event", JSONObject().put("type", "source_draft").put("text", text))
        )
        val updated = SharedSubtitleCore.exchange(request)
        assertEquals(text, updated.getJSONObject("snapshot").getJSONObject("source").getString("text"))
        assertEquals("", create().getJSONObject("snapshot").getJSONObject("source").getString("text"))
        assertTrue(updated.similar(SharedSubtitleCore.exchange(request)))
        assertNotEquals(first.getString("state"), updated.getString("state"))
    }

    @Test fun invalidNativeRequestDoesNotPoisonTheNextExchange() {
        val failure = assertThrows(IllegalArgumentException::class.java) { SharedSubtitleCore.exchangeRaw("{") }
        assertEquals("shared_core_invalid_request", failure.message)
        assertEquals(0, create().getJSONObject("snapshot").getJSONArray("history").length())
    }
}
