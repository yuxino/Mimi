package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class ProviderLanguageCatalogTest {
    private val catalogs = requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
        .bufferedReader().use { JSONObject(it.readText()) }.getJSONObject("providerLanguageCatalogs")

    @Test fun volcanoAcceptsProto3EmptyTextWithoutAcceptingMalformedFields() {
        val contract = requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { JSONObject(it.readText()) }
        val cases = contract.getJSONArray("volcanoSubtitleEvents")
        val protocol = VolcanoProtocol(ServiceConfiguration(ServiceProvider.VOLCANO, emptyMap()), "ja", "zh")
        repeat(cases.length()) { index ->
            val case = cases.getJSONObject(index)
            val bytes = case.getString("hex").chunked(2).map { it.toInt(16).toByte() }.toByteArray()
            val result = runCatching { protocol.binary(bytes) }
            if (case.isNull("expected")) {
                assertTrue(case.getString("id"), result.isFailure)
            } else {
                val expected = case.getJSONObject("expected")
                val text = expected.getString("text")
                val event = when (expected.getString("type")) {
                    "sourceDraft" -> ServiceEvent.Source(text, false, "ja")
                    "sourceFinal" -> ServiceEvent.Source(text, true, "ja")
                    "translationDraft" -> ServiceEvent.Translation(text, false)
                    "translationFinal" -> ServiceEvent.Translation(text, true)
                    else -> error("unknown_contract_event")
                }
                assertEquals(case.getString("id"), listOf(event), result.getOrThrow())
            }
        }
    }

    @Test fun volcanoAccumulatesIncrementalPreviewsAndResetsEachSentence() {
        val contract = requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { JSONObject(it.readText()) }
        val cases = contract.getJSONArray("volcanoSubtitleSequences")
        repeat(cases.length()) { index ->
            val case = cases.getJSONObject(index)
            val protocol = VolcanoProtocol(ServiceConfiguration(ServiceProvider.VOLCANO, emptyMap()), "ja", "zh")
            val actual = JSONObject()
            for (key in listOf("sourceDrafts", "translationDrafts", "sourceFinals", "translationFinals")) actual.put(key, org.json.JSONArray())
            val frames = case.getJSONArray("frames")
            repeat(frames.length()) { frameIndex ->
                val bytes = frames.getString(frameIndex).chunked(2).map { it.toInt(16).toByte() }.toByteArray()
                for (event in protocol.binary(bytes)) {
                    when (event) {
                        is ServiceEvent.Source -> actual.getJSONArray(if (event.final) "sourceFinals" else "sourceDrafts").put(event.text)
                        is ServiceEvent.Translation -> actual.getJSONArray(if (event.final) "translationFinals" else "translationDrafts").put(event.text)
                        else -> error("unexpected_contract_event")
                    }
                }
            }
            assertTrue(case.getString("id"), case.getJSONObject("expected").similar(actual))
        }
    }

    @Test fun volcanoIncrementalPreviewsRemainBounded() {
        val protocol = VolcanoProtocol(ServiceConfiguration(ServiceProvider.VOLCANO, emptyMap()), "ja", "zh")
        for (event in listOf(651, 654)) {
            for (character in listOf("a", "日")) {
                val start = if (event == 651) 650 else 653
                protocol.binary(VolcanoWire.number(2, start.toLong()))
                protocol.binary(VolcanoWire.number(2, event.toLong()) + VolcanoWire.string(4, character.repeat(64 * 1024 / character.toByteArray().size)))
                assertThrows(IllegalArgumentException::class.java) {
                    protocol.binary(VolcanoWire.number(2, event.toLong()) + VolcanoWire.string(4, character))
                }
            }
        }
    }

    @Test fun baiduMapsAllDocumentedSpeechLanguagesToItsOwnWireCodes() {
        val expected = catalogs.getJSONObject("baidu").getJSONObject("wireByCode")
        assertEquals(expected.keys().asSequence().toSet(), BAIDU_LANGUAGE_CODES.keys)
        assertEquals(45, BAIDU_LANGUAGE_CODES.size)
        for (code in BAIDU_LANGUAGE_CODES.keys) {
            val config = ServiceConfiguration(ServiceProvider.BAIDU, mapOf("appId" to "test-id", "appKey" to "test-key"))
            val frame = BaiduProtocol(config, code, code).setup() as WireFrame.Text
            val json = JSONObject(frame.value)
            assertEquals(expected.getString(code), json.getString("from"))
            assertEquals(expected.getString(code), json.getString("to"))
        }
        for (code in listOf("auto", "zh_tw", "fa", "invalid")) {
            assertThrows(IllegalArgumentException::class.java) {
                BaiduProtocol(ServiceConfiguration(ServiceProvider.BAIDU, emptyMap()), code, "en").setup()
            }
        }
    }

    @Test fun tencentChecksTheFullDirectionMatrixIncludingMixedBilingualOutput() {
        val expected = catalogs.getJSONObject("tencent").getJSONObject("pairs")
        assertEquals(expected.keys().asSequence().toSet(), TENCENT_LANGUAGE_PAIRS.keys)
        assertFalse("auto" in ServiceProvider.TENCENT.sources)
        val config = ServiceConfiguration(ServiceProvider.TENCENT,
            mapOf("appId" to "1250000000", "secretId" to "test-id", "secretKey" to "test-key"))
        for (source in ServiceProvider.TENCENT.sources) {
            val targets = expected.getJSONArray(source).let { array -> (0 until array.length()).map(array::getString) }
            assertEquals(targets, TENCENT_LANGUAGE_PAIRS[source])
            for (target in ServiceProvider.TENCENT.targets) {
                val allowed = target in targets
                assertEquals(allowed, ServiceProvider.TENCENT.supportsPair(source, target))
                val result = runCatching { TencentProtocol(config, source, target, 100, 1, "test-voice").request() }
                assertEquals("$source -> $target", allowed, result.isSuccess)
                if (allowed) assertEquals(target, result.getOrThrow().url.queryParameter("target"))
            }
        }
        assertEquals("zh_en" to "zh_en", ServiceProvider.TENCENT.normalize("zh_en", "zh_en"))
        assertEquals("ru" to "zh", ServiceProvider.TENCENT.normalize("ru", "ja"))
        assertEquals("zh_en", TencentProtocol(config, "auto", "en").request().url.queryParameter("source"))
    }

    @Test fun volcanoSerializesEveryLegalS2tPairAndRejectsUnsupportedDirections() {
        val catalog = catalogs.getJSONObject("volcanoEngine")
        assertTrue(catalog.getJSONArray("sourceLanguages").similar(org.json.JSONArray(ServiceProvider.VOLCANO.sources)))
        assertTrue(catalog.getJSONArray("targetLanguages").similar(org.json.JSONArray(ServiceProvider.VOLCANO.targets)))
        val pairs = catalog.getJSONObject("targetsBySource")
        val config = ServiceConfiguration(ServiceProvider.VOLCANO, mapOf("apiKey" to "test-key"))
        for (source in VOLCANO_SOURCE_CODES + listOf("auto", "invalid")) {
            val targets = pairs.optJSONArray(source)?.let { array -> (0 until array.length()).map(array::getString) }.orEmpty()
            for (target in VOLCANO_TARGET_CODES + listOf("yue", "wuu", "original", "invalid")) {
                val expected = target in targets
                assertEquals("$source -> $target", expected, ServiceProvider.VOLCANO.supportsPair(source, target))
                assertEquals("$source -> $target", expected, runCatching { VolcanoProtocol(config, source, target).setup() }.isSuccess)
            }
        }
        val setups = catalog.getJSONArray("setups")
        assertEquals(81, setups.length())
        repeat(setups.length()) { index ->
            val case = setups.getJSONObject(index)
            val bytes = (VolcanoProtocol(config, case.getString("source"), case.getString("target"), "catalog-session").setup() as WireFrame.Binary).value
            assertEquals(case.getString("expectedHex"), bytes.joinToString("") { (it.toInt() and 255).toString(16).padStart(2, '0') })
        }
        assertEquals("fr" to "zh", ServiceProvider.VOLCANO.normalize("fr", "ja"))
        assertEquals("zh_en" to "zh_en", ServiceProvider.VOLCANO.normalize("zh_en", "en"))
        assertEquals("wuu" to "en", ServiceProvider.VOLCANO.normalize("wuu", "en"))
    }

    @Test fun alibabaUsesExactModelCatalogAndWireMappingsForEachRoute() {
        val alibaba = catalogs.getJSONObject("androidAlibaba")
        val live = alibaba.getJSONObject("liveTranslate").getJSONArray("wireCodes")
        assertEquals(60, DASHSCOPE_LIVE_LANGUAGE_CODES.size)
        assertEquals((0 until live.length()).map(live::getString), DASHSCOPE_LIVE_LANGUAGE_CODES.map(::dashScopeTargetCode))
        assertEquals("nb", dashScopeSourceCode("no", false))
        assertEquals("no", dashScopeSourceCode("no", true))
        assertEquals("fil", dashScopeSourceCode("tl", true))
        assertNull(dashScopeSourceCode("auto", true))
        assertNull(dashScopeSourceCode("auto", false))
        val asr = alibaba.getJSONObject("asr").getJSONObject("wireByCode")
        assertEquals(asr.keys().asSequence().toSet(), DASHSCOPE_ASR_LANGUAGE_CODES.toSet())
        for (code in DASHSCOPE_ASR_LANGUAGE_CODES) assertEquals(asr.getString(code), dashScopeSourceCode(code, true))
        assertEquals(27, DASHSCOPE_ASR_LANGUAGE_CODES.size)
        assertTrue("nl" in ServiceProvider.DASHSCOPE.sources)
        assertFalse("nl" in ServiceProvider.DASHSCOPE.sourcesForTranslation(TextTranslationProvider.NONE))
        assertTrue("yue" in ServiceProvider.DASHSCOPE.sourcesForTranslation(TextTranslationProvider.NONE))
        assertThrows(IllegalArgumentException::class.java) { dashScopeSourceCode("nl", true) }
        assertThrows(IllegalArgumentException::class.java) { dashScopeTargetCode("invalid") }
    }

    @Test fun independentTranslationIntersectsAsrWithTheSelectedTextProvider() {
        val provider = ServiceProvider.DASHSCOPE
        for ((route, sourceCatalog, targetCatalog) in listOf(
            Triple(TextTranslationProvider.DEEPL, DEEPL_SOURCE_CODES, DEEPL_TARGET_CODES),
            Triple(TextTranslationProvider.DEEPLX, DEEPLX_SOURCE_CODES, DEEPLX_TARGET_CODES),
        )) {
            assertEquals(listOf("auto") + DASHSCOPE_ASR_LANGUAGE_CODES.filter { it in sourceCatalog }, provider.sourcesForTranslation(route))
            assertEquals(targetCatalog, provider.targetsForTranslation(route))
            for (source in provider.sourcesForTranslation(route)) {
                val (from, to) = provider.normalize(source, "fr", route)
                assertTrue(provider.supportsPair(from, to, route))
            }
        }
        assertEquals("fr" to "fr", provider.normalize("fr", "fr", TextTranslationProvider.OPENAI_COMPATIBLE))
        assertEquals("yue" to "unused", provider.normalize("yue", "unused", TextTranslationProvider.NONE))
    }
}
