package app.yuxino.mimi.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.Locale
import javax.xml.parsers.DocumentBuilderFactory

class InterfaceLanguageCatalogTest {
    private val root = File(checkNotNull(System.getProperty("mimi.repositoryRoot")))
    private val resources = root.resolve("android/app/src/main/res")
    private val directories = mapOf(
        "zh-Hans" to "values-b+zh+Hans", "zh-Hant" to "values-b+zh+Hant",
        "en" to "values", "ja" to "values-ja", "de" to "values-de",
        "ko" to "values-ko", "fr" to "values-fr", "th" to "values-th",
    )
    private fun strings(directory: String): Map<String, String> = buildMap {
        resources.resolve(directory).listFiles()!!.filter { it.name.endsWith("strings.xml") }.forEach { file ->
            val nodes = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file).getElementsByTagName("string")
            for (index in 0 until nodes.length) {
                val node = nodes.item(index)
                val name = node.attributes.getNamedItem("name").nodeValue
                if (node.attributes.getNamedItem("translatable")?.nodeValue != "false") {
                    check(put(name, node.textContent) == null) { "Duplicate $name in $directory" }
                }
            }
        }
    }

    @Test fun pickerAndSystemLanguageCatalogMatchDesktop() {
        val desktop = Regex("UI_LANGUAGES = \\[([^]]+)]").find(root.resolve("src/lib/i18n.ts").readText())!!.groupValues[1]
        val tags = Regex("\"([^\"]+)\"").findAll(desktop).map { match ->
            when (val language = match.groupValues[1]) { "zh" -> "zh-Hans"; "zh-TW" -> "zh-Hant"; else -> language }
        }.toList()
        assertEquals(tags, InterfaceLanguage.tags.drop(1))
        val nodes = DocumentBuilderFactory.newInstance().newDocumentBuilder()
            .parse(resources.resolve("xml/locales_config.xml")).getElementsByTagName("locale")
        assertEquals(tags, (0 until nodes.length).map { nodes.item(it).attributes.getNamedItem("android:name").nodeValue })
    }

    @Test fun everyLanguageHasAllCopyAndPreservesFormatAndProtocolTokens() {
        val base = strings("values")
        val format = Regex("%(?:[0-9]+\\$)?[ds%]")
        val protocol = listOf("/v1/models", "/v1/chat/completions", "stream: false", "choices[0].message.content",
            "http://127.0.0.1:8000/v1", "http://10.0.2.2:8000/v1", "adb reverse tcp:8000 tcp:8000",
            "--reasoning-compat legacy --reasoning-summary none", "qwen3-asr-flash-realtime", "api-free.deepl.com",
            "api.deepl.com", "source_lang", "target_lang", "code: 200")
        directories.forEach { (tag, directory) ->
            val localized = strings(directory)
            assertEquals("$tag missing or unexpected keys", base.keys, localized.keys)
            base.forEach { (key, source) ->
                val value = localized.getValue(key)
                assertTrue("$tag/$key is blank", value.isNotBlank())
                assertEquals("$tag/$key format", format.findAll(source).map { it.value }.sorted().toList(),
                    format.findAll(value).map { it.value }.sorted().toList())
                protocol.filter(source::contains).forEach { token -> assertTrue("$tag/$key lost $token", token in value) }
            }
        }
    }

    @Test fun chineseScriptAndRegionResolutionMatchesDesktop() {
        listOf("zh-Hant", "zh-TW", "zh-HK", "zh-MO", "zh-Hant-CN").forEach {
            assertEquals("zh-Hant", InterfaceLanguage.tags[InterfaceLanguage.indexOf(Locale.forLanguageTag(it))])
        }
        listOf("zh", "zh-CN", "zh-SG", "zh-Hans", "zh-Hans-HK").forEach {
            assertEquals("zh-Hans", InterfaceLanguage.tags[InterfaceLanguage.indexOf(Locale.forLanguageTag(it))])
        }
        assertEquals(0, InterfaceLanguage.indexOf(null))
    }

    @Test fun thaiRegionalLocaleSelectsThaiInterface() {
        listOf("th", "th-TH").forEach {
            assertEquals("th", InterfaceLanguage.tags[InterfaceLanguage.indexOf(Locale.forLanguageTag(it))])
        }
    }
}
