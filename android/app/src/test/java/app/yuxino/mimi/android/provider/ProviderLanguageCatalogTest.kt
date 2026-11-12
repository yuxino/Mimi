package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class ProviderLanguageCatalogTest {
    @Test fun productionAlibabaUsesPcAudio3InsteadOfTheLegacyAndroidAsrCatalog() {
        val sources = ServiceProvider.DASHSCOPE.sourcesForTranslation(TextTranslationProvider.NONE)
        assertEquals(31, sources.size) // Audio3's 30 languages plus Automatic.
        for (code in listOf("nl", "el", "hu", "ro", "bg", "hr", "sk")) assertTrue(code in sources)
        for (code in listOf("yue", "tr", "uk", "is")) assertFalse(code in sources)
        assertEquals("nl" to "unused", ServiceProvider.DASHSCOPE.normalize("nl", "unused", TextTranslationProvider.NONE))
        assertEquals(25, ServiceProvider.DASHSCOPE.sources.size) // PC Lite intersection plus Automatic.
        assertFalse("no" in ServiceProvider.DASHSCOPE.sources)
    }

    @Test fun oldEmptyAndUnknownLanguagePreferencesNormalizeBeforeStarting() {
        for ((source, target) in listOf("" to "", "unknown" to "unknown", "yue" to "yue")) {
            val normalized = ServiceProvider.DASHSCOPE.normalize(source, target)
            assertTrue(ServiceProvider.DASHSCOPE.supportsPair(normalized.first, normalized.second))
            assertFalse(ServiceProvider.DASHSCOPE.supportsPair(source, target))
        }
    }

    @Test fun independentTranslationIntersectsAsrWithTheSelectedTextProvider() {
        val provider = ServiceProvider.DASHSCOPE
        val catalogs = JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONObject("textLanguageCatalogs")
        for ((route, key) in listOf(
            TextTranslationProvider.DEEPL to "deepL",
            TextTranslationProvider.DEEPLX to "deepLX",
        )) {
            val sourceArray = catalogs.getJSONObject(key).getJSONArray("sourceLanguages")
            val targetArray = catalogs.getJSONObject(key).getJSONArray("targetLanguages")
            val sourceCatalog = (0 until sourceArray.length()).map(sourceArray::getString)
            val targetCatalog = (0 until targetArray.length()).map(targetArray::getString)
            val recognition = provider.sourcesForTranslation(TextTranslationProvider.NONE)
            assertEquals(recognition.filter { it == "auto" || it in sourceCatalog }, provider.sourcesForTranslation(route))
            assertEquals(listOf("original") + targetCatalog, provider.targetsForTranslation(route))
            for (source in provider.sourcesForTranslation(route)) {
                val (from, to) = provider.normalize(source, "fr", route)
                assertTrue(provider.supportsPair(from, to, route))
            }
        }
        assertEquals("fr" to "fr", provider.normalize("fr", "fr", TextTranslationProvider.OPENAI_COMPATIBLE))
        assertFalse(provider.supportsPair("yue", "unused", TextTranslationProvider.NONE))
    }
}
