package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.app.LocaleManager
import android.content.Intent
import android.graphics.Bitmap
import android.os.Bundle
import android.os.LocaleList
import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import android.view.PointerIcon
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.widget.ImageButton
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import app.yuxino.mimi.android.capture.CaptureHealth
import app.yuxino.mimi.android.capture.MimiService
import java.io.File

/** Blank API 33+ emulator only: native hover, touch help and visible recovery, no capture/network. */
internal class HelpInteractionChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun ui(action: () -> Unit) { test.runOnMainSync(action); test.waitForIdleSync() }
    private fun views(view: View): List<View> = listOf(view) + if (view is ViewGroup)
        (0 until view.childCount).flatMap { views(view.getChildAt(it)) } else emptyList()
    private fun windows() = WindowInspector.getGlobalWindowViews()
    private fun contains(text: String) = windows().flatMap(::views).any { it is TextView && it.isShown && it.text.toString() == text }
    private fun capture(name: String) {
        test.waitForIdleSync(); SystemClock.sleep(200)
        val file = File(context.getExternalFilesDir(null), "ui-preview/$name.png").apply { parentFile!!.mkdirs() }
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        file.outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }; bitmap.recycle()
    }
    private fun field(name: String, value: Any?) {
        MimiService::class.java.getDeclaredField(name).apply { isAccessible = true; set(null, value) }
    }
    private fun help(host: Activity, tag: String, expected: String, screenshot: String, hover: Boolean = false) {
        val button = checkNotNull(host.findViewById<View>(android.R.id.content).findViewWithTag<ImageButton>(tag))
        ui {
            button.requestRectangleOnScreen(android.graphics.Rect(0, 0, button.width, button.height), true)
            check(button.contentDescription.isNotBlank())
            check(button.tooltipText.toString() == expected)
            check(button.pointerIcon == PointerIcon.getSystemIcon(context, PointerIcon.TYPE_HAND))
            val row = button.parent as ViewGroup
            val label = (0 until row.childCount).map { row.getChildAt(it) }.filterIsInstance<TextView>().firstOrNull()
            if (label != null) {
                check(label.right <= button.left + 1) { "Help overlaps label: $tag" }
                check(button.left - label.right <= 4) { "Help detached from label: $tag" }
                check(label.layout.height <= label.height - label.paddingTop - label.paddingBottom) { "Help label clipped: $tag" }
            }
        }
        if (hover) {
            fun event(action: Int) = MotionEvent.obtain(SystemClock.uptimeMillis(), SystemClock.uptimeMillis(), action,
                button.width / 2f, button.height / 2f, 0).apply { source = InputDevice.SOURCE_MOUSE }
            ui { event(MotionEvent.ACTION_HOVER_MOVE).also { button.dispatchGenericMotionEvent(it); it.recycle() } }
            SystemClock.sleep(900)
            ui { check(contains(expected)) { "Native hover explanation missing: $tag" } }
            capture("$screenshot-hover")
            ui { event(MotionEvent.ACTION_HOVER_EXIT).also { button.dispatchGenericMotionEvent(it); it.recycle() } }
            SystemClock.sleep(100)
            ui { check(!contains(expected)) { "Hover tooltip did not dismiss" } }
        }
        ui { button.performClick(); button.performClick() }
        ui {
            val dialogs = windows().filter { it.findViewWithTag<View>("help-message") != null }
            check(dialogs.size == 1) { "Duplicate help dialogs: $tag" }
            check(dialogs.single().findViewWithTag<TextView>("help-message").text.toString() == expected)
        }
        capture("$screenshot-detail")
        test.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK); test.waitForIdleSync()
        ui { check(windows().none { it.findViewWithTag<View>("help-message") != null }) }
    }
    fun run(arguments: Bundle?) {
        if (MimiService.isRunning || SettingsStore.isConfigured(context)) {
            test.finish(Activity.RESULT_CANCELED, Bundle().apply { putString("stream", "Use a blank idle emulator.\n") })
            return
        }
        val baseline = arguments?.getString("baseline") == "true"
        val locale = arguments?.getString("locale") ?: "zh-Hans"
        val prefix = "help-${if (baseline) "before" else "after"}-$locale"
        val manager = context.getSystemService(LocaleManager::class.java)
        val previousLocale = manager.applicationLocales
        val previousObservation = MimiService.captureObservation
        val prefs = context.getSharedPreferences("first_run", 0)
        val previousSeen = prefs.getBoolean("seen", false)
        val hadSeen = prefs.contains("seen")
        var home: MainActivity? = null
        var settings: SettingsActivity? = null
        var guide: FirstRunGuide? = null
        var failure: Throwable? = null
        try {
            check(prefs.edit().putBoolean("seen", true).commit())
            ui {
                manager.applicationLocales = LocaleList.forLanguageTags(locale)
                AppCompatDelegate.setDefaultNightMode(if (arguments?.getString("theme") == "dark")
                    AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO)
            }
            SystemClock.sleep(600) // LocaleManager configuration propagation is asynchronous.
            val currentHome = test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            home = currentHome
            check(InterfaceLanguage.indexOf(currentHome.resources.configuration.locales[0]) == InterfaceLanguage.tags.indexOf(locale)) { "Activity locale differs from requested sample" }
            capture("$prefix-home")
            if (!baseline) {
                help(currentHome, "home-help", currentHome.getString(R.string.home_description) + "\n\n" + currentHome.getString(R.string.home_privacy), "$prefix-home", hover = true)
                ui { check(currentHome.findViewById<View>(R.id.status_hint).isShown) }
                // Inject only content-free observations; no foreground service, audio or network starts.
                ui {
                    field("isRunning", true)
                    field("captureObservation", CaptureHealth.Snapshot(CaptureHealth.State.AUDIO, 0, 0))
                    MainActivity::class.java.getDeclaredMethod("refreshUi").apply { isAccessible = true; invoke(currentHome) }
                    check(!currentHome.findViewById<View>(R.id.status_hint).isShown)
                    check(currentHome.findViewById<ImageButton>(R.id.status_help).isShown)
                    field("captureObservation", CaptureHealth.Snapshot(CaptureHealth.State.NO_PCM, null, null))
                    MainActivity::class.java.getDeclaredMethod("refreshUi").apply { isAccessible = true; invoke(currentHome) }
                    check(currentHome.findViewById<TextView>(R.id.status_hint).text.toString() == currentHome.getString(R.string.capture_no_sound_hint))
                    check(currentHome.findViewById<View>(R.id.status_hint).isShown)
                }
                capture("$prefix-audio-recovery")
                ui {
                    field("isRunning", false); field("captureObservation", null)
                    MainActivity::class.java.getDeclaredMethod("refreshUi").apply { isAccessible = true; invoke(currentHome) }
                }
            }
            ui { currentHome.findViewById<View>(R.id.source_language_action).performClick() }
            capture("$prefix-language")
            if (!baseline) {
                val button = windows().firstNotNullOf { it.findViewWithTag<ImageButton>("language-help") }
                ui { check(button.tooltipText.toString() == currentHome.getString(R.string.language_sheet_hint)); button.performClick() }
                capture("$prefix-language-detail")
                test.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
            }
            test.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
            val currentSettings = test.startActivitySync(Intent(context, SettingsActivity::class.java).putExtra("settings_section", "appearance")
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as SettingsActivity
            settings = currentSettings
            capture("$prefix-appearance")
            if (!baseline) help(currentSettings, "immersive-help", currentSettings.getString(R.string.settings_immersive_help), "$prefix-immersive", hover = true)
            ui {
                currentSettings.findViewById<View>(R.id.appearance_more_toggle).performClick()
            }
            ui { currentSettings.findViewById<ScrollView>(R.id.settings_scroll).fullScroll(View.FOCUS_DOWN) }
            capture("$prefix-history")
            if (!baseline) help(currentSettings, "history-help", currentSettings.getString(R.string.settings_history_help), "$prefix-history")
            ui { currentSettings.finish() }
            val currentGuide = FirstRunGuide(currentHome) { error("Help must not start capture") }
            guide = currentGuide
            ui { currentGuide.open(1) }; capture("$prefix-guide-storage")
            if (!baseline) ui { check(!contains(currentHome.getString(R.string.guide_saved_hint))) }
            ui { currentGuide.open(2) }
            if (!baseline) ui { check(contains(currentHome.getString(R.string.guide_permission_audio))) }
            ui { currentGuide.open(3) }; capture("$prefix-guide-play")
            if (!baseline) ui {
                check(!contains(currentHome.getString(R.string.guide_capture_limits)))
                check(windows().any { it.findViewWithTag<View>("guide-capture-help")?.isShown == true })
                check(contains(currentHome.getString(R.string.guide_trial_hint, providerTitle(currentHome, app.yuxino.mimi.android.provider.ServiceProvider.DASHSCOPE))))
            }
        } catch (error: Throwable) { failure = error }
        finally {
            ui {
                field("isRunning", false); field("captureObservation", previousObservation)
                guide?.dismiss(); settings?.finish(); home?.finish()
                manager.applicationLocales = previousLocale
                AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
            }
            check((if (hadSeen) prefs.edit().putBoolean("seen", previousSeen) else prefs.edit().remove("seen")).commit())
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null && baseline) "Native $prefix captured: six comparable screens only; no capture or provider.\n"
                else if (failure == null) "Native $prefix passed: hover, touch, duplicate taps, adjacent labels, visible recovery and first-run disclosure; no capture or provider.\n"
                else "Native $prefix failed: ${failure.javaClass.simpleName}: ${failure.message}\n")
        })
    }
}
