package app.yuxino.mimi.android

import android.content.Context
import java.util.Locale

/** Shared by setup and the live overlay; unknown codes must never be labeled Auto. */
fun languageDisplayName(context: Context, code: String): String {
    val resource = when (code) {
        "auto" -> R.string.lang_auto
        "zh" -> R.string.lang_zh
        "en" -> R.string.lang_en
        "ja" -> R.string.lang_ja
        "ko" -> R.string.lang_ko
        else -> null
    }
    return resource?.let(context::getString)
        ?: explicitLanguageDisplayName(code, context.resources.configuration.locales[0])
}

internal fun explicitLanguageDisplayName(code: String, locale: Locale): String {
    if (code == "wuu") return when (locale.language) { "zh" -> "上海话"; "ja" -> "上海語"; else -> "Shanghainese" }
    if (code == "zh_en") return when (locale.language) {
        "zh" -> "中文与英语（混合）"
        "ja" -> "中国語と英語（混在）"
        else -> "Chinese and English (mixed)"
    }
    val tag = when(code) { "zh_tw" -> "zh-Hant"; "tl" -> "fil"; else -> code }
    return Locale.forLanguageTag(tag).getDisplayName(locale).ifBlank { code }
}
