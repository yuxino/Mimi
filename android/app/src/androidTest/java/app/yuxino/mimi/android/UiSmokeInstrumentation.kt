package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.os.Bundle
import android.os.SystemClock
import android.provider.Settings
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.view.inspector.WindowInspector
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Spinner
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.SubtitleBus
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.materialswitch.MaterialSwitch
import java.io.File

/** Native UI checks plus an explicitly selected synthetic system-playback capture probe. */
class UiSmokeInstrumentation : Instrumentation() {
    private var theme = "light"
    private var demo = false
    private var guideCopy = false
    private var firstRun = false
    private var immersiveHelp = false
    private var playbackCapture = false
    private var chatMock = false
    private var interfaceLanguage = false
    private var captureArguments: Bundle? = null
    private var guideLocale = "en"
    private var overlayPreview = false
    private var expectLandscape = false
    private var screenshots = 0
    private var restoreTarget: String? = null
    private var restoreFont: Int? = null
    private var restoreColor: Int? = null
    private var restoreSource: String? = null
    private var restoreBackground: Int? = null

    override fun onCreate(arguments: Bundle?) {
        super.onCreate(arguments)
        theme = arguments?.getString("theme") ?: "light"
        demo = arguments?.getString("demo") == "true"
        guideCopy = arguments?.getString("guide_copy") == "true"
        firstRun = arguments?.getString("first_run") == "true"
        immersiveHelp = arguments?.getString("immersive_help") == "true"
        playbackCapture = arguments?.getString("playback_capture") == "true"
        chatMock = arguments?.getString("chatmock") == "true" || arguments?.getString("chatmock_saved") == "true" ||
            arguments?.getString("chatmock_connection") == "true"
        captureArguments = arguments
        interfaceLanguage = arguments?.getString("interface_language") == "true"
        guideLocale = arguments?.getString("locale") ?: "en"
        overlayPreview = arguments?.getString("overlay_preview") == "true"
        expectLandscape = arguments?.getString("expect_landscape") == "true"
        restoreTarget = arguments?.getString("restore_target")?.takeIf { it in listOf("zh", "en", "ja") }
        restoreFont = arguments?.getString("restore_font")?.toIntOrNull()?.takeIf { it in 12..24 }
        restoreColor = arguments?.getString("restore_color")?.toIntOrNull()?.takeIf { it in 0..4 }
        restoreSource = arguments?.getString("restore_source")?.takeIf { it in listOf("auto", "zh", "en", "ja", "ko") }
        restoreBackground = arguments?.getString("restore_background")?.toIntOrNull()?.takeIf { it in 0..90 }
        start()
    }

    override fun onStart() {
        super.onStart()
        if (captureArguments?.getString("help_interaction") == "true") {
            HelpInteractionChecks(this).run(captureArguments)
            return
        }
        if (captureArguments?.getString("overlay_interaction") == "true") {
            OverlayInteractionChecks(this).run(captureArguments)
            return
        }
        if (interfaceLanguage) {
            InterfaceLanguageChecks(this).run(captureArguments)
            return
        }
        if (playbackCapture) {
            PlaybackCaptureChecks(this).run(captureArguments)
            return
        }
        if (chatMock) {
            ChatMockSettingsChecks(this).run(captureArguments)
            return
        }
        val prefs = AppearanceSnapshot()
        pause("Initial preferences: source=${SettingsStore.sourceLang(targetContext)}, target=${SettingsStore.targetLang(targetContext)}, font=${SettingsStore.fontSize(targetContext)}, color=${SettingsStore.translationColorIndex(targetContext)}, background=${SettingsStore.overlayBgAlpha(targetContext)}", 0)
        var failure: Throwable? = null
        try {
            check(!MimiService.isRunning) { "Stop the active session before running UI checks." }
            onUi {
                if (firstRun || guideCopy || immersiveHelp) {
                    if (android.os.Build.VERSION.SDK_INT >= 33) {
                        targetContext.getSystemService(android.app.LocaleManager::class.java).applicationLocales = android.os.LocaleList.forLanguageTags(guideLocale)
                    } else AppCompatDelegate.setApplicationLocales(androidx.core.os.LocaleListCompat.forLanguageTags(guideLocale))
                }
                AppCompatDelegate.setDefaultNightMode(if (theme == "dark")
                    AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO)
            }
            when {
                guideCopy -> guideCopyScreens()
                firstRun -> firstRunGuide()
                immersiveHelp -> immersiveHelpChecks()
                overlayPreview -> previewOverlay()
                demo -> demonstrate()
                else -> smoke()
            }
        } catch (error: Throwable) {
            failure = error
        } finally {
            onUi {
                prefs.restore()
                AppCompatDelegate.setDefaultNightMode(AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
            }
            check(SettingsStore.flushPendingWritesForTests(targetContext)) { "Preference restore did not reach disk" }
        }
        finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null && guideCopy)
                "Guide copy UI passed ($guideLocale): $screenshots real emulator screenshots; synthetic stopped-sharing state, no credentials, permissions or provider session requested.\n"
            else if (failure == null && immersiveHelp)
                "Immersive help passed ($guideLocale/$theme): $screenshots native screenshots; both entries, cancel, duplicate taps, acknowledgement, exit and service teardown; synthetic overlay only, no provider or capture started.\n"
            else if (failure == null && firstRun)
                "First-run UI passed ($theme): $screenshots real emulator screenshots; synthetic credential fixture only, no provider or capture session started. Clear the dedicated emulator app data after review.\n"
            else if (failure == null)
                "UI ${if (overlayPreview) "overlay preview" else if (demo) "demo" else "smoke"} passed ($theme): $screenshots screenshots; non-secret preferences restored; no credentials saved or provider session started.\n"
            else "UI check failed: ${failure.javaClass.simpleName}: ${failure.message}\n")
        })
    }

    /** Narrow copy regression on a dedicated blank emulator, without system permission requests. */
    private fun guideCopyScreens() {
        check(!SettingsStore.isConfigured(targetContext)) { "Use a blank emulator for guide copy checks" }
        targetContext.getSharedPreferences("first_run", 0).edit().putBoolean("seen", true).commit()
        val home = launchHome()
        var requests = 0
        lateinit var guide: FirstRunGuide
        val previousError = MimiService.lastCaptureError
        try {
            onUi { guide = FirstRunGuide(home) { requests++ }; guide.open(2) }
            capture("guide-access-$guideLocale-$theme")
            onUi { guide.open(4) }
            capture("guide-waiting-$guideLocale-$theme")
            onUi {
                val views = WindowInspector.getGlobalWindowViews()
                check(views.any { root -> containsText(root, home.getString(R.string.guide_wait_caption_title)) })
                setCaptureErrorFixture("capture.projection_stopped")
                check(MimiService.lastCaptureError == "capture.projection_stopped") { "Fixture error was not retained" }
                check(projectionSharingEnded(MimiService.isRunning, MimiService.lastCaptureError)) { "Sharing-ended predicate mismatch" }
                guide.refresh()
            }
            capture("guide-sharing-ended-$guideLocale-$theme")
            onUi {
                val control = WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull {
                    it.findViewWithTag<View>("guide-finish")
                } as? TextView ?: error("Reopen sharing button missing")
                check(control.text.toString() == home.getString(R.string.guide_reopen_sharing)) { "Expected reopen-sharing control, got: ${control.text}" }
                check(control.performClick())
                check(requests == 1)
                check(!MimiService.isRunning)
            }
        } finally {
            onUi { guide.dismiss(); setCaptureErrorFixture(previousError); home.finish() }
        }
    }

    // Instrumentation-only injection; keep the production setter private.
    private fun setCaptureErrorFixture(value: String?) {
        check(!MimiService.isRunning)
        MimiService::class.java.getDeclaredField("lastCaptureError").apply {
            isAccessible = true
            set(null, value)
        }
    }

    private fun containsText(view: View, text: String): Boolean =
        (view is TextView && view.text.toString() == text) ||
        (view is android.view.ViewGroup && (0 until view.childCount).any { containsText(view.getChildAt(it), text) })

    /** Dedicated blank emulator only; fixture is synthetic and never opens a provider session. */
    private fun firstRunGuide() {
        check(!SettingsStore.isConfigured(targetContext)) { "Use a blank emulator for first-run fixtures" }
        val guidePrefs = targetContext.getSharedPreferences("first_run", 0)
        guidePrefs.edit().clear().putBoolean("seen", false).commit()
        val home = launchHome()
        fun guideClick(tag: String) {
            onUi {
                val control = WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull { it.findViewWithTag<View>(tag) }
                    ?: error("Guide control missing: $tag")
                check(control.performClick())
            }
            waitForIdleSync()
        }
        capture("guide-first-$theme")
        sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_TAB)
        onUi { check(WindowInspector.getGlobalWindowViews().any { it.findFocus()?.isEnabled == true }) }
        capture("guide-keyboard-focus-$theme")
        guideClick("guide-skip")
        check(!guidePrefs.getBoolean("completed", false))
        click(home, R.id.first_run_guide)
        guideClick("guide-next")
        capture("guide-credentials-missing-$theme")
        guideClick("guide-skip")
        onUi {
            check(SettingsStore.saveConfiguration(targetContext,
                app.yuxino.mimi.android.provider.ServiceConfiguration(ServiceProvider.DASHSCOPE,
                    mapOf("apiKey" to "synthetic-ui-fixture-never-sent"))))
            check(SettingsStore.activateProvider(targetContext, ServiceProvider.DASHSCOPE))
        }
        click(home, R.id.first_run_guide)
        guideClick("guide-next")
        capture("guide-credentials-saved-$theme")
        guideClick("guide-next")
        capture("guide-permissions-unknown-$theme")
        guideClick("guide-skip")
        click(home, R.id.first_run_guide); guideClick("guide-next"); guideClick("guide-next")
        guideClick("guide-permissions")
        Thread.sleep(900)
        capture("guide-system-overlay-request-$theme")
        sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        Thread.sleep(900)
        capture("guide-permissions-denied-$theme")
        check(guidePrefs.getBoolean("overlay-denied", false))
        fun shell(command: String) {
            uiAutomation.executeShellCommand(command).use { descriptor ->
                android.os.ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { it.readBytes() }
            }
        }
        shell("appops set ${targetContext.packageName} SYSTEM_ALERT_WINDOW allow")
        guideClick("guide-permissions")
        Thread.sleep(900)
        capture("guide-system-audio-request-$theme")
        sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        Thread.sleep(900)
        check(guidePrefs.getBoolean("audio-denied", false))
        check(!MimiService.isRunning)
        capture("guide-audio-permission-denied-$theme")
        shell("pm grant ${targetContext.packageName} android.permission.RECORD_AUDIO")
        guideClick("guide-skip")
        click(home, R.id.first_run_guide); guideClick("guide-next"); guideClick("guide-next")
        capture("guide-permissions-granted-$theme")
        check(Settings.canDrawOverlays(targetContext))
        guideClick("guide-next")
        guideClick("guide-test")
        Thread.sleep(900)
        capture("guide-system-projection-request-$theme")
        sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        Thread.sleep(900)
        check(!MimiService.isRunning)
        check(guidePrefs.getBoolean("projection-denied", false))
        guideClick("guide-back")
        capture("guide-projection-cancelled-$theme")
        shell("appops set ${targetContext.packageName} SYSTEM_ALERT_WINDOW deny")
        guideClick("guide-skip")
        click(home, R.id.first_run_guide); guideClick("guide-next"); guideClick("guide-next")
        capture("guide-overlay-revoked-$theme")
        check(!Settings.canDrawOverlays(targetContext))
        guideClick("guide-skip")
        val guide = FirstRunGuide(home) { error("Fixture must not request a real capture") }
        onUi { guide.open(3) }; capture("guide-audio-not-started-$theme")
        onUi { guide.dismiss(); guide.open(4) }; capture("guide-caption-not-complete-$theme")
        // UI-only recovery fixture: no MediaProjection or provider is started.
        val previousCaptureError = MimiService.lastCaptureError
        try {
            onUi {
                setCaptureErrorFixture("capture.projection_stopped")
                guide.refresh()
            }
            capture("guide-sharing-ended-$theme")
            onUi {
                val action = WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull {
                    it.findViewWithTag<TextView>("guide-finish")
                }
                check(action?.text == targetContext.getString(R.string.guide_reopen_sharing))
            }
        } finally {
            onUi { setCaptureErrorFixture(previousCaptureError); guide.refresh() }
        }
        onUi { guide.dismiss() }
        check(!MimiService.isRunning && !MimiService.firstRunEvidence.complete)
        check(!guidePrefs.getBoolean("completed", false))
        onUi { home.finish() }
        waitForIdleSync()
        guidePrefs.edit().remove("seen").commit()
        val upgraded = launchHome()
        Thread.sleep(300)
        check(guidePrefs.getBoolean("seen", false))
        onUi { check(WindowInspector.getGlobalWindowViews().none { it.findViewWithTag<View>("guide-character-0")?.isShown == true }) }
        capture("guide-existing-install-no-forced-guide-$theme")
        onUi { upgraded.finish() }
    }

    private fun immersiveHelpWindow(): View? = WindowInspector.getGlobalWindowViews().firstOrNull {
        containsText(it, targetContext.getString(R.string.guide_immersive_hint))
    }

    /** Sample the actual shown surface; service and dialog can initially have different night modes. */
    private fun checkImmersiveHelpContrast() {
        var sampleX = 0
        var sampleY = 0
        var titleColor = 0
        var buttonColors = emptyList<Int>()
        onUi {
            val root = checkNotNull(immersiveHelpWindow())
            fun findTitle(view: View): TextView? {
                if (view is TextView && view.text.toString() == targetContext.getString(R.string.guide_immersive)) return view
                if (view is android.view.ViewGroup) {
                    for (index in 0 until view.childCount) findTitle(view.getChildAt(index))?.let { return it }
                }
                return null
            }
            val title = checkNotNull(findTitle(root))
            val location = IntArray(2)
            title.getLocationOnScreen(location)
            sampleX = location[0] + title.width / 2
            sampleY = location[1] - (4 * title.resources.displayMetrics.density).toInt()
            titleColor = title.currentTextColor
            buttonColors = listOf(android.R.id.button1, android.R.id.button2).map {
                root.findViewById<TextView>(it).currentTextColor
            }
        }
        val screenshot = checkNotNull(uiAutomation.takeScreenshot())
        try {
            val background = screenshot.getPixel(sampleX, sampleY)
            check(androidx.core.graphics.ColorUtils.calculateContrast(titleColor, background) >= 3.0) {
                "Immersive help title is unreadable against its actual surface"
            }
            check(buttonColors.all { androidx.core.graphics.ColorUtils.calculateContrast(it, background) >= 4.5 }) {
                "Immersive help actions are unreadable against its actual surface"
            }
        } finally { screenshot.recycle() }
    }

    private fun acknowledgeImmersiveHelpIfShown() {
        onUi { immersiveHelpWindow()?.findViewById<View>(android.R.id.button1)?.performClick() }
        waitForIdleSync()
    }

    /** Both real UI entry points; preview service never opens capture or a provider. */
    private fun immersiveHelpChecks() {
        check(Settings.canDrawOverlays(targetContext)) { "Grant overlay permission on the dedicated emulator." }
        check(!SettingsStore.isConfigured(targetContext)) { "Use a blank emulator for immersive help checks." }
        val prefs = targetContext.getSharedPreferences("first_run", 0)
        prefs.edit().putBoolean("seen", true).remove("immersive_seen").commit()
        SettingsStore.setImmersiveSubtitles(targetContext, false)
        val home = launchHome()
        val settings = openSettings(home, appearance = true)
        val toggle = settings.findViewById<MaterialSwitch>(R.id.immersive_subtitles)
        try {
            click(settings, R.id.immersive_subtitles)
            check(!SettingsStore.immersiveSubtitles(targetContext) && !toggle.isChecked) {
                "First settings entry enabled immersion before acknowledgement"
            }
            capture("immersive-settings-before-confirm-$guideLocale-$theme")
            checkImmersiveHelpContrast()
            onUi {
                val explanation = checkNotNull(immersiveHelpWindow()) { "Settings explanation missing" }
                explanation.findViewById<View>(android.R.id.button2).performClick()
            }
            waitForIdleSync()
            check(!prefs.getBoolean("immersive_seen", false) && !toggle.isChecked)
            // Repeated taps share one pending explanation instead of changing the mode.
            click(settings, R.id.immersive_subtitles)
            click(settings, R.id.immersive_subtitles)
            onUi {
                check(WindowInspector.getGlobalWindowViews().count {
                    containsText(it, targetContext.getString(R.string.guide_immersive_hint))
                } == 1)
            }
            acknowledgeImmersiveHelpIfShown()
            check(prefs.getBoolean("immersive_seen", false) && SettingsStore.immersiveSubtitles(targetContext))
            click(settings, R.id.immersive_subtitles)
            click(settings, R.id.immersive_subtitles)
            onUi { check(immersiveHelpWindow() == null) { "Acknowledged settings explanation repeated" } }
            check(toggle.isChecked)
            click(settings, R.id.immersive_subtitles)
            onUi { settings.finish() }
            waitForIdleSync()

            prefs.edit().remove("immersive_seen").commit()
            targetContext.startService(Intent(targetContext, MimiService::class.java).setAction(MimiService.ACTION_UI_PREVIEW))
            val overlay = checkNotNull(waitForOverlayTag("mimi-overlay"))
            val compact = overlay.findViewWithTag<View>("compact-subtitle")
            onUi { compact.performClick() }
            val entry = overlay.findViewWithTag<View>("enter-immersive")
            onUi { entry.performClick(); entry.performClick() }
            check(!SettingsStore.immersiveSubtitles(targetContext)) { "Overlay enabled immersion before acknowledgement" }
            onUi {
                val explanation = checkNotNull(immersiveHelpWindow()) { "Overlay explanation missing" }
                check((explanation.layoutParams as WindowManager.LayoutParams).type == WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY)
                check(WindowInspector.getGlobalWindowViews().count {
                    containsText(it, targetContext.getString(R.string.guide_immersive_hint))
                } == 1)
            }
            capture("immersive-overlay-before-confirm-$guideLocale-$theme")
            checkImmersiveHelpContrast()
            onUi { immersiveHelpWindow()!!.findViewById<View>(android.R.id.button2).performClick() }
            waitForIdleSync()
            check(!prefs.getBoolean("immersive_seen", false) && !SettingsStore.immersiveSubtitles(targetContext))
            onUi { entry.performClick() }
            acknowledgeImmersiveHelpIfShown()
            val exit = checkNotNull(waitForOverlayTag("exit-immersive"))
            check(SettingsStore.immersiveSubtitles(targetContext) && prefs.getBoolean("immersive_seen", false))
            capture("immersive-overlay-after-confirm-$guideLocale-$theme")
            onUi { exit.performClick() }
            waitForIdleSync()
            check(!SettingsStore.immersiveSubtitles(targetContext))
            val restored = checkNotNull(waitForOverlayTag("mimi-overlay"))
            onUi {
                check(restored.findViewWithTag<View>("compact-subtitle").isShown)
                restored.findViewWithTag<View>("compact-subtitle").performClick()
                restored.findViewWithTag<View>("enter-immersive").performClick()
                check(immersiveHelpWindow() == null) { "Acknowledged overlay explanation repeated" }
            }
            val secondExit = checkNotNull(waitForOverlayTag("exit-immersive"))
            onUi { secondExit.performClick() }
            waitForIdleSync()
            // A pending overlay explanation belongs to its service lifecycle.
            prefs.edit().remove("immersive_seen").commit()
            val lastOverlay = checkNotNull(waitForOverlayTag("mimi-overlay"))
            onUi {
                lastOverlay.findViewWithTag<View>("compact-subtitle").performClick()
                lastOverlay.findViewWithTag<View>("enter-immersive").performClick()
                check(immersiveHelpWindow() != null)
            }
            targetContext.startService(MimiService.stopIntent(targetContext))
            waitForIdleSync()
            onUi { check(immersiveHelpWindow() == null) { "Stopped service retained its explanation" } }
            check(!MimiService.isRunning && !MimiService.firstRunEvidence.complete && !SettingsStore.immersiveSubtitles(targetContext))
        } finally {
            targetContext.startService(MimiService.stopIntent(targetContext))
            onUi { settings.finish(); home.finish() }
            waitForIdleSync()
        }
    }

    private fun smoke() {
        val home = launchHome()
        onUi { WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull { it.findViewWithTag<View>("guide-skip") }?.performClick() }
        click(home, R.id.copy_capture_diagnostics)
        onUi {
            val clipboard = targetContext.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
            val diagnostic = clipboard.primaryClip?.getItemAt(0)?.text?.toString() ?: error("No diagnostic copied")
            check(diagnostic.startsWith("mimi Android capture diagnostics v1\n"))
            check("source=android_playback_capture" in diagnostic)
            check("microphone=false" in diagnostic && "observation=STOPPED" in diagnostic)
            check(diagnostic.lines().size == 10) { "Unexpected diagnostic fields" }
            clipboard.clearPrimaryClip()
        }
        val originalTarget = SettingsStore.targetLang(targetContext)
        val testTarget = if (originalTarget == "ja") "en" else "ja"
        click(home, R.id.target_language_action)
        selectLanguage(testTarget)
        check(SettingsStore.targetLang(targetContext) == testTarget)
        check(home.findViewById<TextView>(R.id.target_summary).text.isNotEmpty())
        onUi {
            val undo = home.findViewById<View>(com.google.android.material.R.id.snackbar_action)
            check(undo != null && undo.performClick()) { "Language Undo unavailable" }
        }
        check(SettingsStore.targetLang(targetContext) == originalTarget) { "Undo did not restore language" }
        capture("home-$theme")

        val initialFont = SettingsStore.fontSize(targetContext)
        val initialColor = SettingsStore.translationColorIndex(targetContext)
        val initialHistory = SettingsStore.historyLines(targetContext)
        val settings = openSettings(home, appearance = true)
        check(settings.findViewById<View>(R.id.appearance_panel).isShown)
        check(settings.findViewById<View>(R.id.service_panel).visibility == View.GONE)
        check(SettingsStore.fontSize(targetContext) == initialFont)
        check(SettingsStore.translationColorIndex(targetContext) == initialColor)
        check(SettingsStore.historyLines(targetContext) == initialHistory)
        val initialImmersive = SettingsStore.immersiveSubtitles(targetContext)
        val immersive = settings.findViewById<MaterialSwitch>(R.id.immersive_subtitles)
        check(immersive.isChecked == initialImmersive)
        click(settings, R.id.immersive_subtitles)
        acknowledgeImmersiveHelpIfShown()
        check(SettingsStore.immersiveSubtitles(targetContext) != initialImmersive)
        check(settings.findViewById<SeekBar>(R.id.overlay_bg_alpha).isEnabled == initialImmersive)
        capture("settings-immersive-$theme")
        click(settings, R.id.immersive_subtitles)
        check(SettingsStore.immersiveSubtitles(targetContext) == initialImmersive)
        val seek = settings.findViewById<SeekBar>(R.id.font_size)
        drag(seek, if (initialFont < 20) 20 else 16)
        check(SettingsStore.fontSize(targetContext) == seek.progress) { "Font was not automatically saved" }
        val changedFont = seek.progress
        onUi { settings.findViewById<Spinner>(R.id.translation_color).setSelection(0) }
        waitForIdleSync()
        check(SettingsStore.translationColorIndex(targetContext) == 1) { "Color was not automatically saved" }
        check(SettingsStore.historyLines(targetContext) == initialHistory) { "Appearance enabled history" }
        capture("settings-appearance-$theme")
        click(settings, R.id.back)
        val reopened = openSettings(home, appearance = true)
        check(reopened.findViewById<SeekBar>(R.id.font_size).progress == changedFont) { "Back discarded appearance" }
        click(reopened, R.id.appearance_more_toggle)
        check(reopened.findViewById<View>(R.id.appearance_more_panel).visibility == View.VISIBLE)
        onUi { reopened.findViewById<ScrollView>(R.id.settings_scroll).scrollTo(0, 10000) }
        waitForIdleSync()
        val history = reopened.findViewById<SeekBar>(R.id.history_lines)
        drag(history, 2)
        check(SettingsStore.historyLines(targetContext) == 2)
        onUi {
            SubtitleBus.onSourceFinal("Sample source")
            SubtitleBus.onTranslationFinal("Sample translation")
        }
        check(SubtitleBus.historySnapshot().isNotEmpty())
        drag(history, 0)
        check(SubtitleBus.historySnapshot().isEmpty()) { "Disabling history did not clear it immediately" }
        click(reopened, R.id.tab_service)
        capture("settings-services-$theme")
        val activeProvider = SettingsStore.provider(targetContext)
        for (provider in ServiceProvider.entries) {
            val configuredBefore = SettingsStore.isConfigured(targetContext, provider)
            val editor = openService(reopened, provider.id)
            val root = editor.findViewById<View>(android.R.id.content)
            for (field in provider.fields) {
                val input = root.findViewWithTag<TextInputEditText>("credential-${field.id}")
                check(input != null) { "Provider field missing: ${provider.id}/${field.id}" }
                if (field.secret) check(input.text.toString().isEmpty()) { "Stored credential was exposed in UI" }
            }
            if (provider.id in listOf("azure", "tencent", "dashscope")) capture("service-${provider.id}-$theme")
            if (!configuredBefore) {
                click(editor, R.id.save)
                check(SettingsStore.provider(targetContext) == activeProvider) { "Invalid form changed active provider" }
            }
            val secret = provider.fields.first { it.secret }
            onUi { root.findViewWithTag<TextInputEditText>("credential-${secret.id}").setText("unsaved-test-value") }
            click(editor, R.id.back)
            check(SettingsStore.isConfigured(targetContext, provider) == configuredBefore) { "Unsaved form changed configuration" }
        }
        val editor = openService(reopened, "openai")
        val key = editor.findViewById<View>(android.R.id.content).findViewWithTag<TextInputEditText>("credential-apiKey")
        onUi {
            check(key.text.toString().isEmpty()) { "Unsaved credential survived reopening" }
            key.requestFocus()
            WindowCompat.getInsetsController(editor.window, key).show(WindowInsetsCompat.Type.ime())
        }
        repeat(30) {
            if (ViewCompat.getRootWindowInsets(key)?.isVisible(WindowInsetsCompat.Type.ime()) != true) Thread.sleep(100)
        }
        check(ViewCompat.getRootWindowInsets(key)?.isVisible(WindowInsetsCompat.Type.ime()) == true) { "Keyboard did not open" }
        capture("settings-keyboard-$theme")
        onUi { editor.finish(); reopened.finish(); home.finish() }
        waitForIdleSync()
    }

    private fun previewOverlay() {
        check(Settings.canDrawOverlays(targetContext)) { "Grant the debug app overlay permission before preview." }
        check(!MimiService.isRunning) { "Stop the active session before preview." }
        // Use the new neutral defaults for both screenshots; AppearanceSnapshot
        // restores the emulator's existing preferences when this check finishes.
        SettingsStore.setTranslationColorIndex(targetContext, 1)
        SettingsStore.setOverlayBgAlpha(targetContext, 65)
        SettingsStore.setOverlayOpacity(targetContext, 100)
        SettingsStore.setImmersiveSubtitles(targetContext, false)
        launchHome()
        try {
            check(targetContext.startService(Intent(targetContext, MimiService::class.java)
                .setAction(MimiService.ACTION_UI_PREVIEW)) != null)
            var root: View? = null
            for (attempt in 0 until 30) {
                waitForIdleSync()
                root = WindowInspector.getGlobalWindowViews().firstOrNull { it.tag == "mimi-overlay" }
                if (root != null) break
                SystemClock.sleep(100)
            }
            val overlay = root ?: error("Actual floating overlay did not appear; windows=" +
                WindowInspector.getGlobalWindowViews().map { it.tag })
            val compact = overlay.findViewWithTag<View>("compact-subtitle")
            val expanded = overlay.findViewWithTag<View>("expanded-subtitles")
            check(compact != null && expanded != null)
            targetContext.startActivity(Intent(Intent.ACTION_MAIN)
                .addCategory(Intent.CATEGORY_HOME)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            waitForIdleSync()
            SystemClock.sleep(1200)
            if (expectLandscape) {
                var wide = false
                for (attempt in 0 until 30) {
                    val screenshot = checkNotNull(uiAutomation.takeScreenshot())
                    wide = screenshot.width > screenshot.height
                    screenshot.recycle()
                    if (wide) break
                    SystemClock.sleep(100)
                }
                check(wide) { "Landscape display did not settle before screenshot" }
            }
            check(compact.isShown && !expanded.isShown)
            capture("overlay-before-compact-$theme")
            onUi { check(compact.performClick()) }
            check(expanded.isShown && !compact.isShown) { "Expanded floating overlay did not open" }
            if (targetContext.resources.displayMetrics.widthPixels > targetContext.resources.displayMetrics.heightPixels) {
                val params = overlay.layoutParams as WindowManager.LayoutParams
                check(params.width <= (560 * targetContext.resources.displayMetrics.density).toInt()) {
                    "Landscape panel is too wide"
                }
                check(params.height <= (targetContext.resources.displayMetrics.heightPixels * 0.48f).toInt()) {
                    "Landscape panel blocks too much height"
                }
            }
            capture("overlay-after-expanded-default-$theme")
            checkOverlayHeaderLabels(expanded)
            val savedHistory = SettingsStore.historyLines(targetContext)
            targetContext.startService(Intent(targetContext, MimiService::class.java)
                .setAction(MimiService.ACTION_UI_PREVIEW_HISTORY))
            waitForIdleSync()
            check(SettingsStore.historyLines(targetContext) == savedHistory)
            capture("overlay-after-expanded-history-$theme")
            checkCurrentCaptionStart(expanded)
            checkLongExpandedCaption(expanded)
            onUi { check(expanded.findViewWithTag<View>("collapse-overlay").performClick()) { "Collapse click failed" } }
            check(compact.isShown && !expanded.isShown) { "Floating overlay did not collapse" }
            capture("overlay-long-compact-$theme")
            onUi { check(compact.performClick()) { "Compact reopen click failed" } }
            onUi { check(expanded.findViewWithTag<View>("enter-immersive").performClick()) { "Immersive entry click failed" } }
            acknowledgeImmersiveHelpIfShown()
            check(SettingsStore.immersiveSubtitles(targetContext)) { "Immersive entry did not update preference" }
            val exit = WindowInspector.getGlobalWindowViews()
                .firstOrNull { it.tag == "exit-immersive" }
            check(exit?.isShown == true) { "Immersive mode has no in-overlay exit" }
            var touchThrough = false
            for (attempt in 0 until 30) {
                waitForIdleSync()
                touchThrough = WindowInspector.getGlobalWindowViews().any {
                    it.tag == "mimi-overlay" &&
                        (it.layoutParams as WindowManager.LayoutParams).flags and
                            WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE != 0
                }
                if (touchThrough) break
                SystemClock.sleep(100)
            }
            check(touchThrough) { "Subtitle window is not touch-through" }
            check((exit.layoutParams as WindowManager.LayoutParams).flags and
                WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE == 0) { "Exit control is not touchable" }
            capture("overlay-after-immersive-$theme")
            onUi { check(exit.performClick()) { "Exit control click failed" } }
            waitForIdleSync()
            check(!SettingsStore.immersiveSubtitles(targetContext)) { "Exit did not clear immersive preference" }
            val restored = WindowInspector.getGlobalWindowViews()
                .firstOrNull { it.tag == "mimi-overlay" }
            check(restored === overlay && expanded.isShown && !compact.isShown) {
                "Exit did not restore the same expanded reading window"
            }
            capture("overlay-after-expanded-restored-$theme")
            check(WindowInspector.getGlobalWindowViews().none { it.tag == "exit-immersive" }) {
                "Exit control remained after leaving immersive mode"
            }
            onUi { check(expanded.findViewWithTag<View>("collapse-overlay").performClick()) }
            waitForIdleSync()
            check(compact.isShown && !expanded.isShown) { "Restored overlay did not collapse" }
            capture("overlay-after-restored-compact-$theme")
            SettingsStore.setImmersiveSubtitles(targetContext, true)
            targetContext.startService(Intent(targetContext, MimiService::class.java)
                .setAction(MimiService.ACTION_APPLY_APPEARANCE))
            val settingsExit = waitForOverlayTag("exit-immersive")
            check(settingsExit?.isShown == true) { "Settings change did not apply to the active overlay" }
            onUi { SubtitleBus.clear(); SubtitleBus.hideLive() }
            check(SubtitleBus.liveHidden && SubtitleBus.displaySource.isEmpty() && SubtitleBus.displayTranslation.isEmpty())
            check(settingsExit.isShown) { "Immersive exit disappeared when speech paused" }
            capture("overlay-empty-$theme")
            SettingsStore.setImmersiveSubtitles(targetContext, false)
            targetContext.startService(Intent(targetContext, MimiService::class.java)
                .setAction(MimiService.ACTION_APPLY_APPEARANCE))
            for (attempt in 0 until 30) {
                waitForIdleSync()
                if (WindowInspector.getGlobalWindowViews().none { it.tag == "exit-immersive" }) break
                SystemClock.sleep(100)
            }
            check(WindowInspector.getGlobalWindowViews().none { it.tag == "exit-immersive" })
        } finally {
            targetContext.startService(MimiService.stopIntent(targetContext))
            waitForIdleSync()
        }
    }

    private fun waitForOverlayTag(tag: String): View? {
        repeat(30) {
            waitForIdleSync()
            WindowInspector.getGlobalWindowViews().firstOrNull { it.tag == tag }?.let { return it }
            SystemClock.sleep(100)
        }
        return null
    }

    private fun checkOverlayHeaderLabels(panel: View) {
        onUi {
            val panelLocation = IntArray(2)
            panel.getLocationOnScreen(panelLocation)
            for (tag in listOf("collapse-overlay", "enter-immersive", "overlay-font", "overlay-route")) {
                val label = checkNotNull(panel.findViewWithTag<TextView>(tag))
                check(label.lineCount == 1) { "Overlay action wraps: $tag" }
                if (tag != "overlay-route") {
                    check(label.paint.measureText(label.text.toString()) <= label.width - label.paddingLeft - label.paddingRight + 1) {
                        "Overlay action text does not fit: $tag"
                    }
                }
                val location = IntArray(2)
                label.getLocationOnScreen(location)
                check(label.width > 0 && location[0] >= panelLocation[0] + panel.paddingLeft &&
                    location[0] + label.width <= panelLocation[0] + panel.width - panel.paddingRight) {
                    "Overlay action extends outside its panel: $tag"
                }
            }
        }
    }

    private fun checkLongExpandedCaption(panel: View) {
        val source = "This is a deliberately long synthetic subtitle for layout review. It checks how the native reading panel wraps several lines while keeping the complete translation readable without starting audio capture."
        val translation = "这是一段用于界面检查的合成长字幕，用来观察原生阅读面板如何换行。它包含较长的完整句子，以及足够多的文字，以便检查小屏幕上的排版和可读性。本次只展示合成内容，没有开启音频采集，也没有连接任何翻译服务。"
        onUi {
            SubtitleBus.clear()
            SubtitleBus.setHistoryLimit(0)
            SubtitleBus.onFinalPair(source, translation, "en")
        }
        capture("overlay-long-expanded-$theme")
        checkCurrentCaptionStart(panel)
        val scroll = checkNotNull(panel.findViewWithTag<ScrollView>("subtitle-transcript-scroll"))
        val sourceView = checkNotNull(panel.findViewWithTag<TextView>("expanded-source"))
        val translationView = checkNotNull(panel.findViewWithTag<TextView>("expanded-translation"))
        onUi {
            check(sourceView.text.toString() == source && translationView.text.toString() == translation)
            check(sourceView.lineCount > 2 && translationView.lineCount > 3) { "Expanded captions kept compact line limits" }
            check(scroll.canScrollVertically(1)) { "Long captions have no scrollable continuation" }
            scroll.fullScroll(View.FOCUS_DOWN)
        }
        capture("overlay-long-expanded-end-$theme")
        onUi {
            check(scroll.scrollY > 0 && !scroll.canScrollVertically(1)) { "Caption end cannot be reached" }
            val layout = checkNotNull(translationView.layout)
            check(layout.getLineEnd(layout.lineCount - 1) == translation.length) { "Caption ending was truncated" }
            val viewport = IntArray(2)
            val text = IntArray(2)
            scroll.getLocationOnScreen(viewport)
            translationView.getLocationOnScreen(text)
            check(text[1] + translationView.height <= viewport[1] + scroll.height - scroll.paddingBottom + 1) {
                "Caption ending is outside the reading viewport"
            }
        }
        // A new completed pair starts at its beginning even after the reader
        // reached the end of the previous long pair.
        onUi { SubtitleBus.onFinalPair("Another complete sentence. $source", translation, "en") }
        waitForIdleSync()
        SystemClock.sleep(250)
        checkCurrentCaptionStart(panel)

        onUi {
            SubtitleBus.setHistoryLimit(3)
            repeat(3) { index -> SubtitleBus.onFinalPair("History sentence $index. $source", translation, "en") }
            SubtitleBus.onFinalPair(source, translation, "en")
        }
        waitForIdleSync()
        SystemClock.sleep(250)
        checkCurrentCaptionStart(panel)
        onUi {
            check(SubtitleBus.historySnapshot().isNotEmpty())
            check(sourceView.top > 0) { "Fixture has no history above the current sentence" }
            scroll.scrollTo(0, 0)
        }
        waitForIdleSync()
        onUi { SubtitleBus.onFinalPair("The reader is reviewing history. $source", translation, "en") }
        waitForIdleSync()
        SystemClock.sleep(250)
        onUi { check(scroll.scrollY == 0) { "New sentence interrupted history review" } }
        // Reopening is an explicit request to return to the current sentence.
        onUi {
            panel.findViewWithTag<View>("collapse-overlay").performClick()
            panel.rootView.findViewWithTag<View>("compact-subtitle").performClick()
        }
        waitForIdleSync()
        SystemClock.sleep(250)
        checkCurrentCaptionStart(panel)
        onUi {
            SubtitleBus.clear()
            SubtitleBus.setHistoryLimit(0)
            SubtitleBus.onFinalPair(source, translation, "en")
        }
        waitForIdleSync()
        SystemClock.sleep(250)
    }

    private fun checkCurrentCaptionStart(panel: View) {
        onUi {
            val scroll = checkNotNull(panel.findViewWithTag<ScrollView>("subtitle-transcript-scroll"))
            val source = checkNotNull(panel.findViewWithTag<TextView>("expanded-source"))
            val viewport = IntArray(2)
            val text = IntArray(2)
            scroll.getLocationOnScreen(viewport)
            source.getLocationOnScreen(text)
            check(text[1] >= viewport[1] - 1 && text[1] < viewport[1] + scroll.height) {
                "Current sentence beginning is outside the reading viewport"
            }
        }
    }

    private fun demonstrate() {
        val home = launchHome()
        pause("READY_FOR_RECORDING", 12000)
        pause("首页：语言直接切换，预览直达外观", 3000)
        click(home, R.id.target_language_action)
        pause("选择字幕语言", 1800)
        selectLanguage("ja")
        pause("选完即保存，首页示例同步变化", 3000)
        click(home, R.id.source_language_action)
        pause("原声语言也在首页", 1600)
        selectLanguage("en")
        pause("无需进入设置或点保存", 2600)
        val settings = openSettings(home, appearance = true)
        pause("点字幕预览，直达外观", 2500)
        drag(settings.findViewById(R.id.font_size), 21)
        pause("调整字号，自动保存", 2200)
        drag(settings.findViewById(R.id.overlay_bg_alpha), 65)
        pause("背景与字幕实时预览", 2200)
        onUi { settings.findViewById<Spinner>(R.id.translation_color).setSelection(0) }
        pause("白色字幕", 1600)
        click(settings, R.id.appearance_more_toggle)
        onUi { settings.findViewById<ScrollView>(R.id.settings_scroll).smoothScrollTo(0, 650) }
        pause("低频选项收起，历史仍默认关闭", 2500)
        click(settings, R.id.tab_service)
        pause("八家服务，已配置的可直接切换", 2800)
        val azure = openService(settings, "azure")
        pause("Azure：只显示资源地址、部署名和 Key", 3000)
        click(azure, R.id.back)
        val tencent = openService(settings, "tencent")
        pause("腾讯云：独立的 AppID 与密钥表单", 2800)
        click(tencent, R.id.back)
        val aliyun = openService(settings, "dashscope")
        pause("已保存的密钥不会回填到输入框", 2600)
        onUi { check(aliyun.window.decorView.findViewWithTag<Spinner>("qwen-mt-model").isShown) }
        pause("Audio 3.0 转写，Qwen-MT Lite、Flash 或 Plus 翻译", 3000)
        click(aliyun, R.id.back)
        click(settings, R.id.back)
        pause("返回首页，外观已经记住", 3000)
        capture("demo-home-$theme")
        pause("演示结束：未启动音频捕获或真实翻译", 15000)
    }

    private fun launchHome(): MainActivity {
        val activity = startActivitySync(Intent(targetContext, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
        waitForIdleSync()
        return activity
    }

    private fun openSettings(home: MainActivity, appearance: Boolean): SettingsActivity {
        val monitor = addMonitor(SettingsActivity::class.java.name, null, false)
        click(home, if (appearance) R.id.subtitle_preview else R.id.go_settings)
        val settings = waitForMonitorWithTimeout(monitor, 5000) as? SettingsActivity ?: error("Settings did not open")
        removeMonitor(monitor)
        waitForIdleSync()
        return settings
    }

    private fun openService(settings: SettingsActivity, provider: String): ServiceSettingsActivity {
        val monitor = addMonitor(ServiceSettingsActivity::class.java.name, null, false)
        onUi {
            val row = settings.findViewById<View>(R.id.service_panel).findViewWithTag<View>("configure-$provider")
            check(row != null && row.performClick()) { "Provider editor did not open" }
        }
        val editor = waitForMonitorWithTimeout(monitor, 5000) as? ServiceSettingsActivity ?: error("Service editor missing")
        removeMonitor(monitor); waitForIdleSync(); return editor
    }

    private fun click(activity: Activity, id: Int) {
        onUi {
            val view = activity.findViewById<View>(id)
            check(view.isEnabled && view.isClickable) { "Control not clickable: ${activity.resources.getResourceEntryName(id)}" }
            view.performClick()
        }
        waitForIdleSync()
        Thread.sleep(220)
    }

    private fun selectLanguage(code: String) {
        onUi {
            val row = WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull {
                it.findViewWithTag<View>("language-$code")
            } ?: error("Language sheet unavailable")
            check(row.performClick())
        }
        waitForIdleSync()
        Thread.sleep(350)
    }

    /** Actual touch events exercise fromUser persistence, including dragging inside a scroll view. */
    private fun drag(seek: SeekBar, value: Int) {
        val location = IntArray(2)
        var start = 0f
        var end = 0f
        var y = 0f
        onUi {
            seek.getLocationOnScreen(location)
            val left = location[0] + seek.paddingLeft
            val width = seek.width - seek.paddingLeft - seek.paddingRight
            start = left + width * (seek.progress - seek.min).toFloat() / (seek.max - seek.min)
            end = left + width * (value - seek.min).toFloat() / (seek.max - seek.min)
            y = location[1] + seek.height / 2f
        }
        val down = SystemClock.uptimeMillis()
        fun event(action: Int, x: Float) {
            val e = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, x, y, 0)
            sendPointerSync(e)
            e.recycle()
        }
        event(MotionEvent.ACTION_DOWN, start)
        for (step in 1..12) { Thread.sleep(45); event(MotionEvent.ACTION_MOVE, start + (end - start) * step / 12) }
        event(MotionEvent.ACTION_UP, end)
        waitForIdleSync()
    }

    private fun capture(name: String) {
        waitForIdleSync()
        Thread.sleep(if (firstRun) 1200 else 400)
        val bitmap = checkNotNull(uiAutomation.takeScreenshot()) { "Screenshot unavailable" }
        val dir = checkNotNull(targetContext.getExternalFilesDir("ui-preview"))
        File(dir, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
        screenshots++
        pause("Captured $name", 0)
    }

    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        runOnMainSync {
            try { action() } catch (error: Throwable) { failure = error }
        }
        failure?.let { throw it }
    }

    private fun pause(message: String, ms: Long) {
        sendStatus(0, Bundle().apply { putString("stream", "$message\n") })
        Thread.sleep(ms)
    }

    private inner class AppearanceSnapshot {
        private val source = restoreSource ?: SettingsStore.sourceLang(targetContext)
        private val target = restoreTarget ?: SettingsStore.targetLang(targetContext)
        private val font = restoreFont ?: SettingsStore.fontSize(targetContext)
        private val color = restoreColor ?: SettingsStore.translationColorIndex(targetContext)
        private val opacity = SettingsStore.overlayOpacity(targetContext)
        private val background = restoreBackground ?: SettingsStore.overlayBgAlpha(targetContext)
        private val history = SettingsStore.historyLines(targetContext)
        private val immersive = SettingsStore.immersiveSubtitles(targetContext)
        private val helpPrefs = targetContext.getSharedPreferences("first_run", 0)
        private val hadImmersiveSeen = helpPrefs.contains("immersive_seen")
        private val immersiveSeen = helpPrefs.getBoolean("immersive_seen", false)
        fun restore() {
            SettingsStore.setSourceLang(targetContext, source)
            SettingsStore.setTargetLang(targetContext, target)
            SettingsStore.setFontSize(targetContext, font)
            SettingsStore.setTranslationColorIndex(targetContext, color)
            SettingsStore.setOverlayOpacity(targetContext, opacity)
            SettingsStore.setOverlayBgAlpha(targetContext, background)
            SettingsStore.setHistoryLines(targetContext, history)
            SettingsStore.setImmersiveSubtitles(targetContext, immersive)
            helpPrefs.edit().apply {
                if (hadImmersiveSeen) putBoolean("immersive_seen", immersiveSeen) else remove("immersive_seen")
            }.commit()
            SubtitleBus.clear()
        }
    }
}
