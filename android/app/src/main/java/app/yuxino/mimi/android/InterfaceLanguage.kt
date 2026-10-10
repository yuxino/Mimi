package app.yuxino.mimi.android

import android.content.Context
import android.content.res.Configuration
import android.os.Build
import android.os.LocaleList
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Spinner
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat
import java.util.concurrent.CopyOnWriteArraySet
import java.util.Locale

/** AppCompat owns persistence before API 33; Android owns it from API 33 onward. */
internal object InterfaceLanguage {
    val tags = listOf("", "zh-Hans", "zh-Hant", "en", "ja", "de", "ko", "fr", "th")
    private val labelIds = listOf(
        R.string.settings_language_system, R.string.settings_language_chinese,
        R.string.settings_language_traditional, R.string.lang_en, R.string.lang_ja,
        R.string.settings_language_german, R.string.lang_ko, R.string.settings_language_french,
        R.string.settings_language_thai,
    )
    private val listeners = CopyOnWriteArraySet<() -> Unit>()

    fun bind(activity: AppCompatActivity, picker: Spinner) {
        picker.adapter = ArrayAdapter(activity, R.layout.mimi_spinner_item,
            labelIds.map(activity::getString)).apply {
            setDropDownViewResource(R.layout.mimi_spinner_dropdown)
        }
        var selected = indexOf(AppCompatDelegate.getApplicationLocales()[0])
        picker.setSelection(selected)
        picker.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                if (position == selected) return
                try {
                    AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(tags[position]))
                    selected = position
                    listeners.forEach { it() }
                } catch (_: RuntimeException) {
                    picker.setSelection(selected)
                    Toast.makeText(activity, R.string.settings_language_failed, Toast.LENGTH_LONG).show()
                }
            }
            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
    }

    fun addListener(listener: () -> Unit) { listeners.add(listener) }
    fun removeListener(listener: () -> Unit) { listeners.remove(listener) }

    /** Match desktop's script/region rules for Chinese, including Hong Kong and Macau. */
    fun indexOf(locale: Locale?): Int {
        if (locale == null) return 0
        val tag = if (locale.language == "zh") {
            if (locale.script == "Hant" || (locale.script != "Hans" && locale.country in setOf("TW", "HK", "MO")))
                "zh-Hant" else "zh-Hans"
        } else locale.language
        return tags.indexOf(tag).coerceAtLeast(0)
    }

    /** Pre-33 AppCompat localizes activities, so service UI needs a localized context too. */
    fun context(context: Context): Context {
        if (Build.VERSION.SDK_INT >= 33) return context
        val locales = AppCompatDelegate.getApplicationLocales()
        if (locales.isEmpty) return context
        return context.createConfigurationContext(Configuration(context.resources.configuration).apply {
            setLocales(LocaleList.forLanguageTags(locales.toLanguageTags()))
        })
    }
}
