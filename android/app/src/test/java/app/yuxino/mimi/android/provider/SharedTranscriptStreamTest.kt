package app.yuxino.mimi.android.provider

import org.junit.Assert.*
import org.junit.Test

class SharedTranscriptStreamTest {
    private class Listener : EngineListener {
        val pairs = mutableListOf<Pair<String, String>>()
        override fun onSessionReady() = Unit
        override fun onSourceDraft(text: String, language: String?) = Unit
        override fun onSourceFinal(text: String, language: String?) = Unit
        override fun onTranslationDraft(text: String) = Unit
        override fun onTranslationFinal(text: String) = Unit
        override fun onFinalPair(source: String, translation: String, language: String?) { pairs += source to translation }
        override fun onError(code: String, message: String) = fail(code)
        override fun onClosed() = Unit
        override fun onLog(message: String) = Unit
    }
    @Test fun differentSentencePunctuationDoesNotPairIndependentFinalsByArrivalOrder() {
        val listener = Listener()
        val stream = SharedTranscriptStream(listener)
        stream.append(true, "First sentence. Second sentence.", null)
        stream.append(false, "第一句、第二句。", null)
        assertTrue(listener.pairs.isEmpty())
        stream.finish()
        assertEquals(listOf("First sentence. Second sentence." to "第一句、第二句。"), listener.pairs)
    }
    @Test fun timingAlignedFinalKeepsBothCompleteTextsAndResetDropsUnmatchedOldTail() {
        val listener = Listener()
        val stream = SharedTranscriptStream(listener)
        stream.append(true, "Hello, world.", 1000)
        stream.append(false, "你好，世界。", 1000)
        assertEquals(listOf("Hello, world." to "你好，世界。"), listener.pairs)
        stream.append(true, "Old incomplete", null)
        stream.reset()
        stream.append(false, "新内容", null)
        stream.finish()
        assertEquals(1, listener.pairs.size)
    }
    @Test fun malformedTimingDoesNotCreateFalseAlignedFinalsInTheTransportAdapter() {
        for (timing in listOf("null", "\"1000\"", "1000.0", "-1")) {
            val listener = Listener()
            val engine = OpenAIRealtimeEngine(listener)
            engine.receiveServerMessage("""{"type":"session.input_transcript.delta","delta":"First. Second.","elapsed_ms":1000}""")
            engine.receiveServerMessage("""{"type":"session.output_transcript.delta","delta":"第一、第二。","elapsed_ms":$timing}""")
            assertTrue("Invalid timing $timing must not confirm mismatched sentence blocks", listener.pairs.isEmpty())
            engine.receiveServerMessage("""{"type":"session.closed"}""")
            assertEquals(listOf("First. Second." to "第一、第二。"), listener.pairs)
            engine.stop()
        }
    }
}
