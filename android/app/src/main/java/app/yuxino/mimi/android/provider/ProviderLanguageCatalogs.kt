package app.yuxino.mimi.android.provider

// Catalogs belong to these exact speech APIs; broader text APIs use separate catalogs.
internal val BAIDU_LANGUAGE_CODES: Map<String, String> = linkedMapOf(
    "zh" to "zh",
    "en" to "en",
    "yue" to "yue",
    "ja" to "jp",
    "ko" to "kor",
    "fr" to "fra",
    "es" to "spa",
    "th" to "th",
    "ar" to "ara",
    "ru" to "ru",
    "pt" to "pt",
    "de" to "de",
    "it" to "it",
    "el" to "el",
    "nl" to "nl",
    "pl" to "pl",
    "bg" to "bul",
    "da" to "dan",
    "fi" to "fin",
    "cs" to "cs",
    "ro" to "rom",
    "sv" to "swe",
    "hu" to "hu",
    "vi" to "vie",
    "id" to "id",
    "ca" to "cat",
    "he" to "heb",
    "hi" to "hi",
    "ms" to "may",
    "no" to "nor",
    "is" to "ice",
    "tl" to "fil",
    "km" to "hkm",
    "hr" to "hrv",
    "lv" to "lav",
    "bn" to "ben",
    "ne" to "nep",
    "af" to "afr",
    "sk" to "sk",
    "si" to "sin",
    "sr" to "srp",
    "sw" to "swa",
    "tr" to "tr",
    "uk" to "ukr",
    "hy" to "arm",
)

internal val TENCENT_LANGUAGE_PAIRS: Map<String, List<String>> = linkedMapOf(
    "zh" to listOf("zh", "en", "ja", "ko", "yue", "id", "th"),
    "en" to listOf("zh", "en", "ja", "ko", "yue", "id", "th"),
    "zh_en" to listOf("zh_en", "zh", "en", "ja", "ko", "yue", "id", "th"),
    "ja" to listOf("zh", "en", "ja", "ko", "yue"),
    "ko" to listOf("zh", "en", "ja", "ko", "yue"),
    "yue" to listOf("zh", "en", "ja", "ko", "yue"),
    "id" to listOf("zh", "en", "id"),
    "th" to listOf("zh", "en", "th"),
    "ru" to listOf("zh", "en", "ru"),
)

internal val DASHSCOPE_LIVE_LANGUAGE_CODES = listOf("zh", "en", "ar", "de", "fr", "es", "pt", "id", "it", "ko", "ru", "th", "vi", "ja", "tr", "hi", "ms", "nl", "ur", "no", "sv", "da", "he", "fi", "pl", "is", "cs", "tl", "fa", "yue", "el", "af", "ast", "be", "bg", "bn", "bs", "ca", "ceb", "et", "gl", "gu", "hr", "hu", "jv", "kk", "kn", "ky", "lv", "mk", "ml", "mr", "pa", "ro", "sk", "sl", "sw", "tg", "az", "uk")
internal val DASHSCOPE_ASR_LANGUAGE_CODES = listOf("zh", "yue", "en", "ja", "de", "ko", "ru", "fr", "pt", "ar", "it", "es", "hi", "id", "th", "tr", "uk", "vi", "cs", "da", "tl", "fi", "is", "ms", "no", "pl", "sv")

internal fun dashScopeSourceCode(code: String, transcriptionOnly: Boolean): String? {
    if (code == "auto") return null
    val catalog = if (transcriptionOnly) DASHSCOPE_ASR_LANGUAGE_CODES else DASHSCOPE_LIVE_LANGUAGE_CODES
    require(code in catalog) { "unsupported_language" }
    return when(code) { "tl" -> "fil"; "no" -> if (transcriptionOnly) "no" else "nb"; else -> code }
}

internal fun dashScopeTargetCode(code: String): String {
    require(code in DASHSCOPE_LIVE_LANGUAGE_CODES) { "unsupported_language" }
    return when(code) { "tl" -> "fil"; "no" -> "nb"; else -> code }
}

internal val VOLCANO_SOURCE_CODES = listOf("zh","en","pt","es","ja","id","de","fr","ru","it","ko","ar","tr","ms","vi","th","nl","ro","pl","cs","yue","wuu","zh_en")
internal val VOLCANO_TARGET_CODES = listOf("zh","en","pt","es","ja","id","de","fr","ru","it","ko","ar","tr","ms","vi","th","nl","ro","pl","cs","zh_en")
internal val VOLCANO_LANGUAGE_PAIRS: Map<String, List<String>> = linkedMapOf(
    "zh" to listOf("zh","en","pt","es","ja","id","de","fr","ru","it","ko","ar","tr","ms","vi","th","nl","ro","pl","cs"),
    "en" to listOf("zh","en","pt","es","ja","id","de","fr","ru","it","ko","ar","tr","ms","vi","th","nl","ro","pl","cs"),
    "pt" to listOf("zh","en"),
    "es" to listOf("zh","en"),
    "ja" to listOf("zh","en"),
    "id" to listOf("zh","en"),
    "de" to listOf("zh","en"),
    "fr" to listOf("zh","en"),
    "ru" to listOf("zh","en"),
    "it" to listOf("zh","en"),
    "ko" to listOf("zh","en"),
    "ar" to listOf("zh","en"),
    "tr" to listOf("zh","en"),
    "ms" to listOf("zh","en"),
    "vi" to listOf("zh","en"),
    "th" to listOf("zh","en"),
    "nl" to listOf("zh","en"),
    "ro" to listOf("zh","en"),
    "pl" to listOf("zh","en"),
    "cs" to listOf("zh","en"),
    "yue" to listOf("zh","en"),
    "wuu" to listOf("zh","en"),
    "zh_en" to listOf("zh_en"),
)
internal fun volcanoWireLanguage(code: String): String = when(code) { "yue" -> "yue-CN"; "wuu" -> "sh-CN"; "zh_en" -> "zhen"; else -> code }
