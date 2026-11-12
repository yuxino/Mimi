package app.yuxino.mimi.android.provider

import org.junit.Assert.*
import org.junit.Test
import org.json.JSONObject

class ServiceCatalogTest {
    private fun config(provider:ServiceProvider, vararg pairs:Pair<String,String>) = ServiceConfiguration(provider,mapOf("apiKey" to "test-key") + pairs)

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
        val cases = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("requests")
        val expectedTargets = (0 until cases.length()).map(cases::getJSONObject)
            .filter { it.getString("provider") == "openaiCompatible" && !it.isNull("expected") }
            .map { it.getString("target") }.toSet() + "original"
        for (route in listOf(TextTranslationProvider.OPENAI_COMPATIBLE, TextTranslationProvider.CHAT_MOCK)) {
            val targets = ServiceProvider.DASHSCOPE.targetsForTranslation(route)
            assertEquals(expectedTargets, targets.toSet())
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
}
