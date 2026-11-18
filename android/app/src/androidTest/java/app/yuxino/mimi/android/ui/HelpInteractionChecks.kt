package app.yuxino.mimi.android.ui

import android.app.Activity
import android.app.Application
import android.app.Instrumentation
import android.app.LocaleManager
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Bitmap
import android.os.Bundle
import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import android.view.PointerIcon
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.widget.ImageButton
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import app.yuxino.mimi.android.MainActivity
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.captions.openCaptionStyles
import app.yuxino.mimi.android.capture.CaptureHealth
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.localization.InterfaceLanguage
import app.yuxino.mimi.android.onboarding.FirstRunGuide
import app.yuxino.mimi.android.services.editor.ServiceSettingsUi
import app.yuxino.mimi.android.services.setup.providerTitle
import app.yuxino.mimi.android.settings.SettingsStore
import java.io.File

/** Blank API 33+ emulator only: native hover, touch help and visible recovery, no capture/network. */
internal class HelpInteractionChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private fun ui(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        test.waitForIdleSync()
        failure?.let { throw it }
    }
    private fun views(view: View): List<View> = listOf(view) + if (view is ViewGroup)
        (0 until view.childCount).flatMap { views(view.getChildAt(it)) } else emptyList()
    private fun windows() = WindowInspector.getGlobalWindowViews()
    private fun contains(text: String) = windows().flatMap(::views).any { it is TextView && it.isShown && it.text.toString() == text }
    private fun capture(name: String) {
        test.waitForIdleSync(); SystemClock.sleep(600)
        val file = File(context.getExternalFilesDir(null), "ui-preview/$name.png").apply { parentFile!!.mkdirs() }
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        file.outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }; bitmap.recycle()
    }
    private fun back() {
        test.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        test.waitForIdleSync()
        SystemClock.sleep(300) // Wait for the dialog's exit animation before addressing its parent.
    }
    private fun field(name: String, value: Any?) {
        MimiService::class.java.getDeclaredField(name).apply { isAccessible = true; set(null, value) }
    }
    private fun help(host: Activity, tag: String, expected: String, screenshot: String, hover: Boolean = false) {
        val button = checkNotNull(host.findViewById<View>(android.R.id.content).findViewWithTag<ImageButton>(tag)
            ?: windows().firstNotNullOfOrNull { it.findViewWithTag<ImageButton>(tag) })
        ui {
            button.requestRectangleOnScreen(android.graphics.Rect(0, 0, button.width, button.height), true)
            check(button.contentDescription.isNotBlank())
            check(button.tooltipText == null) { "Framework tooltip would duplicate contextual help" }
            check(button.pointerIcon == PointerIcon.getSystemIcon(context, PointerIcon.TYPE_HAND))
            val row = button.parent as ViewGroup
            val label = (0 until row.childCount).map { row.getChildAt(it) }.filterIsInstance<TextView>().firstOrNull()
            if (label != null) {
                check(label.right <= button.left + 1) { "Help overlaps label: $tag" }
                check(button.left - label.right <= 4) { "Help detached from label: $tag" }
                check(label.layout.height <= label.height - label.paddingTop - label.paddingBottom) { "Help label clipped: $tag" }
            }
        }
        SystemClock.sleep(300) // ScrollView must finish layout before system pointer coordinates are read.
        if (hover) {
            test.setInTouchMode(true)
            ui { HelpUi.dismiss(host as androidx.appcompat.app.AppCompatActivity) }
            test.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_TAB)
            test.waitForIdleSync()
            ui { check(button.requestFocus()) { "Keyboard help target cannot receive focus: $tag" } }
            SystemClock.sleep(650)
            ui {
                val popup = windows().firstNotNullOf { it.findViewWithTag<TextView>("help-tooltip-message") }.rootView
                val origin = IntArray(2).also(popup::getLocationOnScreen)
                val anchor = IntArray(2).also(button::getLocationOnScreen)
                val bounds = android.graphics.Rect(origin[0], origin[1], origin[0] + popup.width, origin[1] + popup.height)
                val anchorBounds = android.graphics.Rect(anchor[0], anchor[1], anchor[0] + button.width, anchor[1] + button.height)
                check(tooltipMessage(popup) == expected) { "Keyboard explanation differs from its anchor: $tag" }
                check(!android.graphics.Rect.intersects(bounds, anchorBounds)) { "Help popup covers its anchor: $tag; popup=$bounds anchor=$anchorBounds" }
                button.clearFocus()
            }
            SystemClock.sleep(250)
            test.setInTouchMode(true)
            ui { HelpUi.dismiss(host as androidx.appcompat.app.AppCompatActivity) }
            val point = IntArray(2)
            ui {
                button.getLocationOnScreen(point)
                val visible = android.graphics.Rect()
                check(button.getGlobalVisibleRect(visible) && visible.height() == button.height && visible.width() == button.width) {
                    "Hover target is outside the visible viewport: $tag"
                }
            }
            fun pointer(x: Float, y: Float) {
                val props = MotionEvent.PointerProperties().apply { id = 0; toolType = MotionEvent.TOOL_TYPE_MOUSE }
                val coords = MotionEvent.PointerCoords().apply { this.x = x; this.y = y }
                val event = MotionEvent.obtain(SystemClock.uptimeMillis(), SystemClock.uptimeMillis(), MotionEvent.ACTION_HOVER_MOVE,
                    1, arrayOf(props), arrayOf(coords), 0, 0, 1f, 1f, 0, 0, InputDevice.SOURCE_MOUSE, 0)
                try { check(test.uiAutomation.injectInputEvent(event, true)) } finally { event.recycle() }
            }
            val x = point[0] + button.width / 2f
            val y = point[1] + button.height / 2f
            pointer(1f, 1f); SystemClock.sleep(250)
            pointer(x - 8 * context.resources.displayMetrics.density, y); pointer(x, y)
            pointer(x + 8 * context.resources.displayMetrics.density, y)
            SystemClock.sleep(900)
            capture("$screenshot-hover")
            ui {
                check(contains(expected)) { "Native hover explanation missing: $tag; hovered=${button.isHovered}; anchor=${point[0]},${point[1]}; windows=${windows().size}" }
                check(windows().flatMap(::views).count { it is TextView && it.isShown && it.text.toString() == expected } == 1) { "Duplicate hover explanations" }
                val tooltip = windows().firstNotNullOf { it.findViewWithTag<TextView>("help-tooltip-message") }
                check((0 until tooltip.layout.lineCount).all { tooltip.layout.getEllipsisCount(it) == 0 }) { "Hover explanation was truncated" }
            }
            pointer(1f, 1f)
            SystemClock.sleep(250)
            ui { check(!contains(expected)) { "Hover tooltip did not dismiss" } }
        }
        ui { button.performClick(); button.performClick() }
        ui {
            check(windows().none { it.findViewWithTag<View>("help-tooltip-message") != null }) { "Hover popup survived opening details: $tag" }
            val dialogs = windows().filter { it.findViewWithTag<View>("help-message") != null }
            check(dialogs.size == 1) { "Duplicate help dialogs: $tag" }
            check(dialogs.single().findViewWithTag<TextView>("help-message").text.toString() == expected)
        }
        capture("$screenshot-detail")
        back()
        ui { check(windows().none { it.findViewWithTag<View>("help-message") != null }) }
    }
    private fun tooltipMessage(view: View) = view.findViewWithTag<TextView>("help-tooltip-message").text.toString()
    fun run(arguments: Bundle?) {
        if (MimiService.isRunning || SettingsStore.profiles(context).isNotEmpty()) {
            test.finish(Activity.RESULT_CANCELED, Bundle().apply { putString("stream", "Use a blank idle emulator.\n") })
            return
        }
        val baseline = arguments?.getString("baseline") == "true"
        val locale = arguments?.getString("locale") ?: "zh-Hans"
        val prefix = "help-${if (baseline) "before" else "after"}-$locale"
        val requestedNightMode = if (arguments?.getString("theme") == "dark")
            AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO
        val requestedNightMask = if (requestedNightMode == AppCompatDelegate.MODE_NIGHT_YES)
            Configuration.UI_MODE_NIGHT_YES else Configuration.UI_MODE_NIGHT_NO
        val manager = context.getSystemService(LocaleManager::class.java)
        val previousLocale = manager.applicationLocales
        val previousNightMode = AppCompatDelegate.getDefaultNightMode()
        val previousObservation = MimiService.captureObservation
        val prefs = context.getSharedPreferences("first_run", 0)
        val previousSeen = prefs.getBoolean("seen", false)
        val hadSeen = prefs.contains("seen")
        val application = context.applicationContext as Application
        val fixtureId = java.util.UUID.randomUUID().toString()
        val ownerExtra = "mimi.help.fixture"
        val ownedActivities = mutableSetOf<MainActivity>()
        var resumedHome: MainActivity? = null
        val callbacks = object : Application.ActivityLifecycleCallbacks {
            override fun onActivityCreated(activity: Activity, state: Bundle?) {
                if (activity is MainActivity && activity.intent.getStringExtra(ownerExtra) == fixtureId) {
                    ownedActivities.add(activity)
                }
            }
            override fun onActivityResumed(activity: Activity) {
                if (activity is MainActivity && activity in ownedActivities) resumedHome = activity
            }
            override fun onActivityDestroyed(activity: Activity) {
                if (activity is MainActivity) ownedActivities.remove(activity)
                if (resumedHome === activity) resumedHome = null
            }
            override fun onActivityStarted(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivityStopped(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) = Unit
        }
        var guide: FirstRunGuide? = null
        var failure: Throwable? = null
        try {
            check(prefs.edit().putBoolean("seen", true).commit())
            ui {
                application.registerActivityLifecycleCallbacks(callbacks)
            }
            val launchedHome = test.startActivitySync(Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra(ownerExtra, fixtureId)) as MainActivity
            val requestedIndex = InterfaceLanguage.tags.indexOf(locale)
            check(requestedIndex >= 0) { "Unsupported help locale tag: $locale" }
            var themeChanged = false
            ui {
                themeChanged = (launchedHome.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) != requestedNightMask
                // Local mode belongs only to this fixture's Activity. The
                // process default may be UNSPECIFIED, which AppCompat cannot
                // restore through setDefaultNightMode after a global change.
                launchedHome.delegate.localNightMode = requestedNightMode
            }
            var languageOwner: MainActivity? = null
            var changed = false
            // Use the production picker after AppCompat has a live delegate,
            // then follow its replacement Activity rather than the old host.
            val languageDeadline = SystemClock.elapsedRealtime() + 5000
            var localizedHome: MainActivity? = null
            var languageStatus = "requested=$locale"
            do {
                ui {
                    val candidate = resumedHome
                    val actual = candidate?.resources?.configuration?.locales?.get(0)
                    val focused = candidate?.hasWindowFocus() == true
                    val nightMask = candidate?.resources?.configuration?.uiMode?.and(Configuration.UI_MODE_NIGHT_MASK)
                    val themeReady = nightMask == requestedNightMask && (!themeChanged || candidate !== launchedHome)
                    languageStatus = "requested=$locale; framework=${manager.applicationLocales.toLanguageTags()}; " +
                        "appcompat=${AppCompatDelegate.getApplicationLocales().toLanguageTags()}; " +
                        "activity=${actual?.toLanguageTag() ?: "none"}; resumed=${candidate != null}; " +
                        "focused=$focused; night=$nightMask; themeReady=$themeReady"
                    if (candidate != null && !candidate.isFinishing && !candidate.isDestroyed && focused && themeReady) {
                        if (languageOwner == null) {
                            candidate.findViewById<View>(R.id.tab_general).performClick()
                            val picker = candidate.findViewById<Spinner>(R.id.interface_language)
                            changed = picker.selectedItemPosition != requestedIndex
                            languageOwner = candidate
                            picker.setSelection(requestedIndex)
                        }
                        if ((!changed || candidate !== languageOwner) && InterfaceLanguage.indexOf(actual) == requestedIndex) {
                            localizedHome = candidate
                        }
                    }
                }
                if (localizedHome == null) SystemClock.sleep(50)
            } while (localizedHome == null && SystemClock.elapsedRealtime() < languageDeadline)
            val currentHome = checkNotNull(localizedHome) { "Help locale preparation timed out: $languageStatus" }
            check(InterfaceLanguage.indexOf(currentHome.resources.configuration.locales[0]) == InterfaceLanguage.tags.indexOf(locale)) {
                "Activity locale differs from requested sample: $languageStatus"
            }
            ui { currentHome.findViewById<View>(R.id.tab_caption).performClick() }
            capture("$prefix-home")
            if (!baseline) {
                help(currentHome, "home-help", currentHome.getString(R.string.home_description), "$prefix-home", hover = true)
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
                help(currentHome, "language-help", currentHome.getString(R.string.language_sheet_hint), "$prefix-language", hover = true)
            }
            back()
            ui { check(windows().none { it.findViewWithTag<View>("language-help") != null }) { "Language sheet still open" } }
            val currentSettings = currentHome
            ui { currentSettings.findViewById<View>(R.id.tab_service).performClick() }
            if (!baseline && currentSettings.resources.configuration.fontScale >= 1.5f) ui {
                for (provider in app.yuxino.mimi.android.provider.ServiceProvider.entries) {
                    val row = checkNotNull(currentSettings.findViewById<View>(R.id.service_panel)
                        .findViewWithTag<ViewGroup>("add-${provider.id}")) { "Blank service picker entry missing: ${provider.id}" }
                    val label = (0 until row.childCount).map { row.getChildAt(it) }.filterIsInstance<TextView>().single()
                    check(row.isClickable && row.isFocusable && row.height >= ServiceSettingsUi.dp(currentSettings, 48)) {
                        "Service picker lost its touch target: ${provider.id}"
                    }
                    check(label.width >= ServiceSettingsUi.dp(currentSettings, 150)) { "Service name column too narrow: ${provider.id}" }
                    check(label.left >= row.getChildAt(0).right && label.right <= row.getChildAt(row.childCount - 1).left) {
                        "Service name overlaps its mark or chevron: ${provider.id}"
                    }
                    check(label.layout.height <= label.height - label.compoundPaddingTop - label.compoundPaddingBottom &&
                        (0 until label.lineCount).all { label.layout.getEllipsisCount(it) == 0 }) {
                        "Service name clips at large font size: ${provider.id}"
                    }
                }
            }
            capture("$prefix-services")
            if (!baseline) help(currentSettings, "services-help", currentSettings.getString(R.string.profiles_list_help) + "\n\n" +
                currentSettings.getString(R.string.services_key_note), "$prefix-services", hover = true)
            ui { openCaptionStyles(currentSettings) }
            capture("$prefix-appearance")
            if (!baseline) help(currentSettings, "immersive-help", currentSettings.getString(R.string.settings_immersive_help), "$prefix-immersive", hover = true)
            ui { currentSettings.findViewById<ScrollView>(R.id.settings_scroll).fullScroll(View.FOCUS_DOWN) }
            if (!baseline) ui {
                check(views(currentSettings.findViewById(android.R.id.content)).count {
                    it is TextView && it.text.toString() == currentSettings.getString(R.string.settings_history)
                } == 1) { "Duplicate history label" }
            }
            capture("$prefix-history")
            if (!baseline) help(currentSettings, "history-help", currentSettings.getString(R.string.settings_history_help), "$prefix-history")
            ui { currentSettings.findViewById<View>(R.id.tab_caption).performClick() }
            val resumedDeadline = SystemClock.elapsedRealtime() + 5000
            var resumed = false
            do {
                ui { resumed = currentHome.hasWindowFocus() }
                if (!resumed) SystemClock.sleep(50)
            } while (!resumed && SystemClock.elapsedRealtime() < resumedDeadline)
            check(resumed) { "Home did not resume before guide sample" }
            // Its ActivityResult launcher belongs to onCreate, before STARTED; reuse the activity-owned guide.
            val currentGuide = MainActivity::class.java.getDeclaredField("guide").run {
                isAccessible = true; get(currentHome) as FirstRunGuide
            }
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
            try {
                ui {
                    field("isRunning", false); field("captureObservation", previousObservation)
                    guide?.dismiss()
                    manager.applicationLocales = previousLocale
                    check(AppCompatDelegate.getDefaultNightMode() == previousNightMode) { "Help fixture changed the default night mode" }
                }
            } finally {
                try {
                    // The restore above waits for idle and can recreate the
                    // host. Keep observing until every fixture-owned host is
                    // destroyed, including replacements created during close.
                    val closeDeadline = SystemClock.elapsedRealtime() + 5000
                    var closed = false
                    do {
                        ui { ownedActivities.toList().forEach { it.finish() } }
                        ui { closed = ownedActivities.isEmpty() }
                        if (!closed) SystemClock.sleep(50)
                    } while (!closed && SystemClock.elapsedRealtime() < closeDeadline)
                    check(closed) { "Help fixture Activity cleanup timed out" }
                } finally {
                    try {
                        ui { application.unregisterActivityLifecycleCallbacks(callbacks) }
                    } finally {
                        check((if (hadSeen) prefs.edit().putBoolean("seen", previousSeen) else prefs.edit().remove("seen")).commit())
                    }
                }
            }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null && baseline) "Native $prefix captured: six comparable screens only; no capture or provider.\n"
                else if (failure == null) "Native $prefix passed: hover, touch, duplicate taps, adjacent labels, visible recovery and first-run disclosure; no capture or provider.\n"
                else "Native $prefix failed: ${failure.javaClass.simpleName}: ${failure.message}; ${failure.stackTrace.firstOrNull { it.className.contains("HelpInteractionChecks") }}\n")
        })
    }
}
