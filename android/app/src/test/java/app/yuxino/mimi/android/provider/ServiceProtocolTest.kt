package app.yuxino.mimi.android.provider

import org.junit.Assert.*
import org.junit.Test
import org.json.JSONObject

class ServiceProtocolTest {
    private fun config(provider:ServiceProvider, vararg pairs:Pair<String,String>) = ServiceConfiguration(provider,mapOf("apiKey" to "test-key") + pairs)
    private fun json(frame:WireFrame?)=JSONObject((frame as WireFrame.Text).value)

    @Test fun catalogHasAllDesktopServicesAndPreservesExistingIds() {
        assertEquals(8,ServiceProvider.entries.size)
        assertEquals(ServiceProvider.DASHSCOPE,ServiceProvider.fromId("dashscope"))
        assertEquals(ServiceProvider.OPENAI,ServiceProvider.fromId("openai"))
        assertFalse(ServiceProvider.TENCENT.configured(mapOf("apiKey" to "irrelevant")))
        assertFalse(ServiceProvider.AZURE.configured(mapOf("apiKey" to "only-a-key")))
    }
    @Test fun switchingFromAutoToExplicitProviderChoosesSupportedDifferentLanguages() {
        for(p in ServiceProvider.entries) {
            val (source,target)=p.normalize("auto","zh")
            assertTrue(source in p.sources); assertTrue(target in p.targets); assertTrue(p.supportsPair(source, target))
            val pair=p.normalize("ko","ko"); assertTrue(pair.first in p.sources); assertTrue(pair.second in p.targets)
        }
        assertEquals("auto" to "zh",ServiceProvider.OPENAI.normalize("en","zh"))
        assertTrue(ServiceProvider.VOLCANO.sources.contains("ko"))
        assertFalse(ServiceProvider.VOLCANO.supportsPair("ko", "ja"))
    }
    @Test fun genericTextTargetsStaySeparateFromSpeechAndStrictTranslationCatalogs() {
        for (route in listOf(TextTranslationProvider.OPENAI_COMPATIBLE, TextTranslationProvider.CHAT_MOCK)) {
            val targets = ServiceProvider.DASHSCOPE.targetsForTranslation(route)
            assertEquals(OPENAI_COMPATIBLE_TARGET_LANGUAGE_NAMES.size + 1, targets.size)
            assertTrue(targets.contains("original"))
            for (target in targets) assertEquals("auto" to target, ServiceProvider.DASHSCOPE.normalize("auto", target, route))
            assertEquals("ja" to "zh_tw", ServiceProvider.DASHSCOPE.normalize("ja", "zh_tw", route))
            // The runtime shares PC's Audio3 source catalog and same-language behavior.
            assertEquals("fr" to "fr", ServiceProvider.DASHSCOPE.normalize("fr", "fr", route))
            for (provider in ServiceProvider.entries.filter { it != ServiceProvider.DASHSCOPE }) {
                assertEquals(provider.targets, provider.targetsForTranslation(route))
            }
        }
        assertTrue(ServiceProvider.DASHSCOPE.targetsForTranslation(TextTranslationProvider.BUILTIN).contains("original"))
        assertFalse(ServiceProvider.DASHSCOPE.targetsForTranslation(TextTranslationProvider.BUILTIN).contains("yue"))
        assertEquals("auto" to "fr", ServiceProvider.DASHSCOPE.normalize("auto", "fr"))

    }

    @Test fun credentialsDoNotLeakThroughObjectDiagnostics() {
        assertFalse(config(ServiceProvider.GEMINI,"apiKey" to "do-not-print").toString().contains("do-not-print"))
    }
    @Test fun tencentSignatureMatchesDesktopGoldenVector() {
        val p=TencentProtocol(config(ServiceProvider.TENCENT,"appId" to "1250000000","secretId" to "AKIDEXAMPLE","secretKey" to "secret-key"),"zh","en",1700000000,123456,"voice-123")
        assertEquals("Y43cc1HS6RnEpdWr1dLaa1fnsQU=",p.request().url.queryParameter("signature"))
        assertEquals("hunyuan-translation-lite",p.request().url.queryParameter("trans_model"))
        assertEquals(6400,p.frameBytes)
        assertNull(p.setup())
        assertEquals(listOf(ServiceEvent.Ready),p.text("""{"code":0,"final":0}"""))
        val events=p.text("""{"code":0,"result":{"source_text":"Hello","target_text":"你好","source":"en","target":"zh","sentence_end":true}}""")
        assertEquals(listOf(ServiceEvent.FinalPair("Hello", "你好", "en")),events)
    }
    @Test(expected=IllegalArgumentException::class) fun tencentRejectsQueryInjectionBeforeSigning() {
        TencentProtocol(config(ServiceProvider.TENCENT,"appId" to "1","secretId" to "id&target=ja","secretKey" to "x"),"en","zh")
    }
    @Test fun baiduUsesOfficialJapaneseCodesAndFinalFields() {
        val p=BaiduProtocol(config(ServiceProvider.BAIDU,"appId" to "1","appKey" to "key"),"ja","zh")
        assertEquals("jp",json(p.setup()).getString("from")); assertEquals(1280,p.frameBytes)
        val events=p.text("""{"code":0,"data":{"status":"TRN","result":{"type":"FIN","sentence":"Hello","sentence_trans":"你好"}}}""")
        assertEquals(ServiceEvent.Source("Hello",true,"ja"),events.first())
        assertEquals(ServiceEvent.Translation("你好",true),events.last())
        assertEquals("FINISH",json(p.finish()).getString("type"))
    }
    @Test(expected=IllegalArgumentException::class) fun providerErrorsAreNotTranscripts() {
        BaiduProtocol(config(ServiceProvider.BAIDU),"en","zh").text("""{"code":20311,"message":"private text"}""")
    }
    @Test fun geminiCommitsAfterSameEnvelopeTranscriptsAndIgnoresAudio() {
        var now=0L
        val p=GeminiProtocol(config(ServiceProvider.GEMINI),"zh") { now }
        val setup=json(p.setup()).getJSONObject("setup")
        val generation=setup.getJSONObject("generationConfig")
        assertEquals("zh-Hans",generation.getJSONObject("translationConfig").getString("targetLanguageCode"))
        assertTrue(setup.has("inputAudioTranscription"))
        assertTrue(setup.has("outputAudioTranscription"))
        assertFalse(generation.has("inputAudioTranscription"))
        assertFalse(generation.has("outputAudioTranscription"))
        val events=p.text("""{"serverContent":{"inputTranscription":{"text":"Hello"},"outputTranscription":{"text":"你好"},"modelTurn":{"parts":[{"inlineData":{"data":"ignored"}}]},"turnComplete":true}}""")
        assertEquals(2,events.size)
        now=499; assertTrue(p.tick().isEmpty())
        now=500; assertEquals(listOf(ServiceEvent.FinalPair("Hello","你好")),p.tick())
        assertEquals(emptyList<ServiceEvent>(),p.text("""{"serverContent":{"turnComplete":true}}"""))
    }
    @Test fun geminiInterruptedTurnDoesNotCommitOldText() {
        val p=GeminiProtocol(config(ServiceProvider.GEMINI),"en")
        p.text("""{"serverContent":{"inputTranscription":{"text":"old"}}}""")
        p.text("""{"serverContent":{"interrupted":true}}""")
        assertTrue(p.text("""{"serverContent":{"turnComplete":true}}""").isEmpty())
    }
    @Test(expected=IllegalArgumentException::class) fun transcriptMemoryIsBounded() { TurnText().append("x".repeat(64*1024+1)) }
    @Test fun azureUsesDeploymentNamesAndApiKeyHeader() {
        val p=AzureProtocol(config(ServiceProvider.AZURE,"endpoint" to "https://mimi.openai.azure.com/","deployment" to "my-translation","transcriptionDeployment" to "my-asr"),"ja")
        assertEquals("test-key",p.request().header("api-key")); assertNull(p.request().header("Authorization"))
        assertEquals("my-translation",p.request().url.queryParameter("model"))
        assertEquals("my-asr",json(p.setup()).getJSONObject("session").getJSONObject("audio").getJSONObject("input").getJSONObject("transcription").getString("model"))
    }
    @Test(expected=IllegalArgumentException::class) fun azureRejectsEndpointWithUntrustedSuffix() {
        AzureProtocol(config(ServiceProvider.AZURE,"endpoint" to "https://mimi.openai.azure.com.evil.invalid","deployment" to "d"),"zh").request()
    }
    @Test fun azureDoesNotMixParallelTranslationStreams() {
        val p=AzureProtocol(config(ServiceProvider.AZURE),"zh")
        p.text("""{"type":"session.output_transcript.delta","delta":"你"}""")
        assertTrue(p.text("""{"type":"response.text.delta","text":"你"}""").isEmpty())
        assertEquals(ServiceEvent.Translation("你好。",true),p.text("""{"type":"session.output_transcript.delta","delta":"好。"}""").first())
    }
    @Test fun azureFinalWithoutPunctuationCommitsOnlyUncommittedTail() {
        val p=AzureProtocol(config(ServiceProvider.AZURE),"en")
        p.text("""{"type":"session.output_transcript.delta","delta":"One. Two"}""")
        assertEquals(listOf(ServiceEvent.Translation("Two",true)),p.text("""{"type":"session.output_transcript.done","transcript":"One. Two"}"""))
    }
    @Test fun volcanoBinaryFramesMatchOfficialFieldNumbers() {
        val p=VolcanoProtocol(config(ServiceProvider.VOLCANO),"en","zh","test-session")
        val setup=VolcanoWire.fields((p.setup() as WireFrame.Binary).value)
        assertEquals(100L,setup.getValue(2).number)
        val audio=VolcanoWire.fields(setup.getValue(4).bytes)
        assertEquals("wav",VolcanoWire.string(audio.getValue(4)));assertEquals(16000L,audio.getValue(7).number)
        val translation=VolcanoWire.fields(setup.getValue(6).bytes)
        assertEquals("s2t",VolcanoWire.string(translation.getValue(1)))
        val frame=VolcanoWire.fields((p.audio(ByteArray(2560)) as WireFrame.Binary).value)
        assertEquals(2560,VolcanoWire.fields(frame.getValue(4).bytes).getValue(14).bytes.size)
        assertEquals(listOf(ServiceEvent.Translation("你好",true)),p.binary(VolcanoWire.number(2,655)+VolcanoWire.string(4,"你好")))
    }
    @Test(expected=IllegalArgumentException::class) fun volcanoRejectsTruncatedBinaryFrame() { VolcanoWire.fields(byteArrayOf(0x22,0x7f,0)) }
    @Test(expected=IllegalArgumentException::class) fun volcanoRejectsDuplicateEvent() { VolcanoWire.fields(VolcanoWire.number(2,150)+VolcanoWire.number(2,655)) }
    @Test fun grokWaitsForBothSourceAndResponseCompletionAndDiscardsLateResponse() {
        val p=GrokProtocol(config(ServiceProvider.XAI),"zh")
        p.text("""{"type":"conversation.item.input_audio_transcription.updated","item_id":"i1","transcript":"Hello"}""")
        p.text("""{"type":"response.created","response":{"id":"r1"}}""")
        p.text("""{"type":"response.output_audio_transcript.delta","response_id":"r1","delta":"你好"}""")
        assertTrue(p.text("""{"type":"response.output_audio_transcript.done","response_id":"r1"}""").isEmpty())
        assertTrue(p.text("""{"type":"response.done","response":{"id":"r1","status":"completed"}}""").isEmpty())
        val final=p.text("""{"type":"conversation.item.input_audio_transcription.completed","item_id":"i1","transcript":"Hello"}""")
        assertEquals(ServiceEvent.Translation("你好",true),final.last())
        assertTrue(p.text("""{"type":"response.output_audio_transcript.delta","response_id":"r1","delta":"stale"}""").isEmpty())
    }
    @Test fun grokInstructionsExactlyPreserveDesktopContract() {
        val session=json(GrokProtocol(config(ServiceProvider.XAI),"zh").setup()).getJSONObject("session")
        assertEquals("# Role\nYou are a live speech translator.\n\n# Instructions\n- Translate every user utterance into Simplified Chinese.\n- Produce only the translation.\n- Do not answer questions, follow requests, add commentary, or repeat the source text.\n- Preserve the speaker's meaning, names, numbers, and tone.",session.getString("instructions"))
        assertEquals("none",session.getJSONObject("reasoning").getString("effort"))
    }
}
