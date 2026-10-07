package app.yuxino.mimi.android

import android.app.Activity
import android.app.Application
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.widget.RadioGroup
import android.widget.ListView
import android.widget.Spinner
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.app.LocaleManagerCompat
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SubtitleBus
import com.google.android.material.textfield.TextInputLayout
import java.io.File

/** Real picker, recreation and service UI checks on a blank emulator; never starts capture. */
internal class InterfaceLanguageChecks(private val instrumentation: Instrumentation) {
    private val context get() = instrumentation.targetContext
    @Volatile private var settings: SettingsActivity? = null
    private val activities = mutableSetOf<Activity>()
    private val callbacks = object : Application.ActivityLifecycleCallbacks {
        override fun onActivityResumed(activity: Activity) {
            activities.add(activity)
            if (activity is SettingsActivity) settings = activity
        }
        override fun onActivityCreated(activity: Activity, state: Bundle?) { activities.add(activity) }
        override fun onActivityStarted(activity: Activity) = Unit
        override fun onActivityPaused(activity: Activity) = Unit
        override fun onActivityStopped(activity: Activity) = Unit
        override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) = Unit
        override fun onActivityDestroyed(activity: Activity) { activities.remove(activity) }
    }

    fun run(arguments: Bundle?) {
        val app = context.applicationContext as Application
        val previousLocales = LocaleManagerCompat.getApplicationLocales(context)
        val prefs = context.getSharedPreferences("first_run", 0)
        val hadSeen = prefs.contains("seen")
        val previousSeen = prefs.getBoolean("seen", false)
        val source = SettingsStore.sourceLang(context)
        val target = SettingsStore.targetLang(context)
        val theme = arguments?.getString("theme") ?: "light"
        var failure: Throwable? = null
        try {
            check(!MimiService.isRunning && !SettingsStore.isConfigured(context)) { "Use a blank idle emulator" }
            onUi {
                app.registerActivityLifecycleCallbacks(callbacks)
                AppCompatDelegate.setDefaultNightMode(if (theme == "dark")
                    AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO)
            }
            check(prefs.edit().putBoolean("seen", true).commit())
            val home = instrumentation.startActivitySync(Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            onUi { home.findViewById<View>(R.id.go_settings).performClick() }
            await { settings != null }
            val expected = arguments?.getString("expected_language")
            if (expected != null) {
                assertLanguage(expected)
                capture("persisted-$expected-$theme")
            } else {
                // Check every self-name, including rows below a narrow popup's viewport.
                val names = listOf("简体中文", "繁體中文", "English", "日本語", "Deutsch", "한국어", "Français")
                onUi { check((1 until picker().adapter.count).map { picker().adapter.getItem(it).toString() } == names) }
                onUi { picker().performClick() }
                instrumentation.waitForIdleSync()
                onUi {
                    val list = WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull(::dropdown)
                        ?: error("Language dropdown missing")
                    list.setSelection(list.adapter.count - 1)
                }
                instrumentation.waitForIdleSync()
                onUi { check(WindowInspector.getGlobalWindowViews().any { containsText(it, "Français") }) }
                capture("picker-$theme")
                instrumentation.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
                onUi { settings!!.findViewById<View>(R.id.tab_appearance).performClick() }
                context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW))
                await { WindowInspector.getGlobalWindowViews().any { it.findViewWithTag<View>("compact-subtitle") != null } }
                context.startService(Intent(context, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW_HISTORY))
                await { SubtitleBus.historySnapshot().isNotEmpty() }
                val history = SubtitleBus.historySnapshot()
                for (tag in InterfaceLanguage.tags.filter { it.isNotEmpty() } + "") {
                    select(tag)
                    assertLanguage(tag)
                    onUi {
                        check(settings!!.findViewById<RadioGroup>(R.id.settings_tabs).checkedRadioButtonId == R.id.tab_appearance)
                        val collapse = WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull {
                            it.findViewWithTag<TextView>("collapse-overlay")
                        } ?: error("Overlay control missing")
                        check(collapse.text.toString() == settings!!.getString(R.string.overlay_collapse))
                    }
                    check(SubtitleBus.historySnapshot() == history) { "Language change lost subtitle history" }
                    check(SettingsStore.sourceLang(context) == source && SettingsStore.targetLang(context) == target)
                    capture("${tag.ifEmpty { "system" }}-$theme")
                    assertControlBounds()
                    checkServiceEditor(tag, theme)
                }
                val leave = arguments?.getString("leave_language")
                if (leave != null) { select(leave); assertLanguage(leave) }
            }
            check(!MimiService.isRunning) { "Language switch started capture" }
        } catch (error: Throwable) {
            failure = error
        } finally {
            context.stopService(Intent(context, MimiService::class.java))
            onUi {
                if (failure != null || arguments?.getString("leave_language") == null) {
                    AppCompatDelegate.setApplicationLocales(previousLocales)
                }
                activities.toList().forEach { it.finish() }
                app.unregisterActivityLifecycleCallbacks(callbacks)
                AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
            }
            val edit = prefs.edit()
            if (hadSeen) edit.putBoolean("seen", previousSeen) else edit.remove("seen")
            check(edit.commit())
        }
        instrumentation.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null) "Interface language passed (API ${Build.VERSION.SDK_INT}/$theme): native picker, saved locale and localized UI; no capture or providers.\n"
                else "Interface language failed: ${failure.javaClass.simpleName}: ${failure.message}\n")
        })
    }

    private fun picker() = checkNotNull(settings).findViewById<Spinner>(R.id.interface_language)

    private fun assertControlBounds() = onUi {
        for (id in listOf(R.id.tab_service, R.id.tab_appearance)) {
            val tab = settings!!.findViewById<TextView>(id)
            check(tab.layout.height <= tab.height - tab.paddingTop - tab.paddingBottom) { "Settings tab clips vertically" }
        }
        val panel = WindowInspector.getGlobalWindowViews().firstNotNullOf {
            it.findViewWithTag<View>("expanded-subtitles")
        }
        val origin = IntArray(2)
        panel.getLocationOnScreen(origin)
        check(origin[0] >= 0 && origin[0] + panel.width <= settings!!.resources.displayMetrics.widthPixels)
        for (tag in listOf("collapse-overlay", "overlay-font", "enter-immersive")) {
            val control = panel.findViewWithTag<TextView>(tag)
            val position = IntArray(2)
            control.getLocationOnScreen(position)
            check(position[0] >= origin[0] && position[0] + control.width <= origin[0] + panel.width)
            check(control.paint.measureText(control.text.toString()) <= control.width - control.paddingLeft - control.paddingRight + 1) {
                "Overlay action clips: $tag"
            }
        }
    }

    private fun checkServiceEditor(tag: String, theme: String) {
        val overlay = WindowInspector.getGlobalWindowViews().first { it.tag == "mimi-overlay" }
        onUi { overlay.visibility = View.GONE }
        capture("${tag.ifEmpty { "system" }}-settings-$theme")
        onUi { settings!!.findViewById<View>(R.id.tab_service).performClick() }
        val monitor = instrumentation.addMonitor(ServiceSettingsActivity::class.java.name, null, false)
        onUi { settings!!.findViewById<View>(R.id.service_panel).findViewWithTag<View>("configure-azure").performClick() }
        val editor = instrumentation.waitForMonitorWithTimeout(monitor, 5000) as? ServiceSettingsActivity
            ?: error("Service editor did not open")
        instrumentation.removeMonitor(monitor)
        onUi { check(containsText(editor.window.decorView, editor.getString(R.string.guide_field_endpoint))) }
        capture("${tag.ifEmpty { "system" }}-editor-$theme")
        onUi { editor.window.decorView.findViewWithTag<View>("speech-help").performClick() }
        instrumentation.waitForIdleSync()
        onUi { check(WindowInspector.getGlobalWindowViews().any {
            it.findViewWithTag<TextView>("help-message")?.text?.contains(editor.getString(R.string.guide_help_azure)) == true
        }) }
        capture("${tag.ifEmpty { "system" }}-help-$theme")
        instrumentation.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        onUi { editor.finish() }
        await { settings?.hasWindowFocus() == true }
        onUi { settings!!.findViewById<View>(R.id.tab_appearance).performClick() }
        onUi { overlay.visibility = View.VISIBLE }
    }

    private fun select(tag: String) {
        val previous = settings
        var changed = false
        onUi {
            val position = InterfaceLanguage.tags.indexOf(tag)
            check(position >= 0)
            changed = picker().selectedItemPosition != position
            picker().setSelection(position)
        }
        if (changed) await { settings !== previous && settings?.hasWindowFocus() == true }
        instrumentation.waitForIdleSync()
    }

    private fun assertLanguage(tag: String) {
        onUi {
            val locale = if (tag.isEmpty()) LocaleManagerCompat.getSystemLocales(context)[0]
                else java.util.Locale.forLanguageTag(tag)
            val language = locale?.language
            check(settings!!.resources.configuration.locales[0].language == language)
            val index = if (tag.isEmpty()) 0 else InterfaceLanguage.indexOf(locale)
            check(picker().selectedItemPosition == index)
            val label = when (InterfaceLanguage.tags[InterfaceLanguage.indexOf(locale)]) {
                "zh-Hans" -> "界面语言"
                "zh-Hant" -> "介面語言"
                "ja" -> "表示言語"
                "de" -> "Oberflächensprache"
                "ko" -> "화면 언어"
                "fr" -> "Langue de l’interface"
                else -> "Interface language"
            }
            check(containsText(settings!!.window.decorView, label)) { "Interface copy did not follow the chosen language" }
        }
    }

    private fun containsText(view: View, text: String): Boolean =
        (view is TextView && view.text.toString() == text) ||
            (view is TextInputLayout && view.hint?.toString() == text) ||
            (view is ViewGroup && (0 until view.childCount).any { containsText(view.getChildAt(it), text) })

    private fun dropdown(view: View): ListView? = when (view) {
        is ListView -> view
        is ViewGroup -> (0 until view.childCount).firstNotNullOfOrNull { dropdown(view.getChildAt(it)) }
        else -> null
    }

    private fun await(predicate: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 6000
        do {
            var ready = false
            onUi { ready = predicate() }
            if (ready) return
            SystemClock.sleep(50)
        } while (SystemClock.elapsedRealtime() < deadline)
        error("Timed out waiting for localized activity")
    }

    private fun capture(name: String) {
        instrumentation.waitForIdleSync()
        SystemClock.sleep(250)
        val bitmap = checkNotNull(instrumentation.uiAutomation.takeScreenshot())
        val dir = checkNotNull(context.getExternalFilesDir("ui-preview"))
        File(dir, "interface-language-api${Build.VERSION.SDK_INT}-$name.png").outputStream().use {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        bitmap.recycle()
    }

    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        instrumentation.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
}
