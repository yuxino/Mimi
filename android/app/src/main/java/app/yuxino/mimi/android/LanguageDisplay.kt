package app.yuxino.mimi.android

import android.content.Context
import java.util.Locale

/** Shared by setup and the live overlay; unknown codes must never be labeled Auto. */
fun languageDisplayName(context: Context, code: String): String {
    val resource = languageNameResource(code)
    return resource?.let(context::getString)
        ?: explicitLanguageDisplayName(code, context.resources.configuration.locales[0])
}

internal fun languageNameResource(code: String): Int? = when (code) {
        "auto" -> R.string.lang_auto
        "zh" -> R.string.lang_zh
        "en" -> R.string.lang_en
        "ja" -> R.string.lang_ja
        "ko" -> R.string.lang_ko
        "wuu" -> R.string.lang_shanghainese
        "zh_en" -> R.string.lang_chinese_english
        else -> null
}

internal fun explicitLanguageDisplayName(code: String, locale: Locale): String {
    val tag = when(code) { "zh_tw" -> "zh-Hant"; "tl" -> "fil"; else -> code }
    return Locale.forLanguageTag(tag).getDisplayName(locale).ifBlank { code }
}
