package app.yuxino.mimi.android.services.profiles

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.widget.Spinner
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import app.yuxino.mimi.android.MainActivity
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.localization.InterfaceLanguage
import app.yuxino.mimi.android.onboarding.FirstRunGuide
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.services.editor.ServiceSettingsActivity
import app.yuxino.mimi.android.services.editor.ServiceSettingsUi
import app.yuxino.mimi.android.services.presentation.modelNamesLabel
import app.yuxino.mimi.android.services.presentation.runtimeModelNames
import app.yuxino.mimi.android.services.translation.translationProviderLabel
import app.yuxino.mimi.android.settings.SettingsStore
import com.google.android.material.textfield.TextInputEditText
import java.io.File

/** Explicit synthetic UI acceptance on a blank debug installation; never clears user data. */
internal class ServiceProfileChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private val activities = mutableListOf<Activity>()
    private val created = linkedSetOf<String>()
    private var stage = "preconditions"
    private var screenshots = 0
    private var theme = "light"
    private var previewSettleMs = 250L
    private lateinit var home: MainActivity

    fun run(arguments: Bundle?) {
        if (arguments?.getString("realtime_modes") == "true") { runRealtimeModes(); return }
        var failure: String? = null
        var failureClass: String? = null
        var failureLocation: String? = null
        var ownsFixture = false
        val previousNight = AppCompatDelegate.getDefaultNightMode()
        val localeManager = if (android.os.Build.VERSION.SDK_INT >= 33) context.getSystemService(android.app.LocaleManager::class.java) else null
        val previousLocales = if (android.os.Build.VERSION.SDK_INT >= 33) localeManager?.applicationLocales else null
        val previousCompatLocales = AppCompatDelegate.getApplicationLocales()
        val locale = arguments?.getString("locale")
        val guidePrefs = context.getSharedPreferences("first_run", 0)
        val hadSeen = guidePrefs.contains("seen")
        val wasSeen = guidePrefs.getBoolean("seen", false)
        try {
            stage = "debug_build"
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            stage = "session_stopped"
            check(!MimiService.isRunning && !MimiService.isStopping)
            stage = "empty_catalog"
            check(SettingsStore.profiles(context).isEmpty()) { "Use a blank debug installation" }
            ownsFixture = true
            stage = "guide_preference"
            check(guidePrefs.edit().putBoolean("seen", true).commit())
            theme = arguments?.getString("theme") ?: "light"
            previewSettleMs = arguments?.getString("preview_settle_ms")?.toLongOrNull()?.coerceIn(250L, 5_000L) ?: 250L
            stage = "locale"
            if (locale != null) {
                check(locale in InterfaceLanguage.tags.filter(String::isNotEmpty))
                onUi {
                    if (android.os.Build.VERSION.SDK_INT >= 33) localeManager?.applicationLocales = android.os.LocaleList.forLanguageTags(locale)
                    else AppCompatDelegate.setApplicationLocales(androidx.core.os.LocaleListCompat.forLanguageTags(locale))
                }
                SystemClock.sleep(300)
            }
            stage = "theme"
            onUi { AppCompatDelegate.setDefaultNightMode(if (theme == "dark") AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO) }
            stage = "home_launch"
            home = test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            activities += home
            stage = "wake_fixture_window"
            onUi {
                home.setShowWhenLocked(true)
                home.setTurnScreenOn(true)
                home.window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                context.getSystemService(android.app.KeyguardManager::class.java).requestDismissKeyguard(home, null)
            }
            stage = "list_navigation"
            showList()
            stage = "empty_picker_and_cancel"
            check(field<View>(home, "add-dashscope").isShown)
            capture("empty")
            val cancelled = openFrom("add-dashscope")
            onUi { field<TextInputEditText>(cancelled, "profile-name").setText("Discarded synthetic draft"); cancelled.findViewById<View>(R.id.back).performClick() }
            test.waitForIdleSync()
            check(SettingsStore.profiles(context).isEmpty())

            stage = "first_profile"
            val firstEditor = openFrom("add-dashscope")
            onUi {
                field<TextInputEditText>(firstEditor, "profile-name").setText(if (home.resources.configuration.locales[0].language == "zh") "视频字幕" else "Video captions")
                field<Spinner>(firstEditor, "alibaba-mode").setSelection(0)
            }
            test.waitForIdleSync()
            onUi { field<TextInputEditText>(firstEditor, "credential-apiKey").setText("synthetic-speech-one") }
            save(firstEditor)
            val first = SettingsStore.profiles(context).single().id.also(created::add)
            check(SettingsStore.activeProfileId(context) == first)
            showList(); capture("one")

            stage = "second_same_provider_and_mode_drafts"
            val secondEditor = openAdded()
            onUi {
                field<TextInputEditText>(secondEditor, "profile-name").setText(if (home.resources.configuration.locales[0].language == "zh") "阿里同声" else "Alibaba simultaneous")
                field<TextInputEditText>(secondEditor, "credential-apiKey").setText("synthetic-speech-two")
                field<TextInputEditText>(secondEditor, "credential-alibaba-workspace").setText("synthetic-workspace")
                field<Spinner>(secondEditor, "alibaba-mode").setSelection(0)
            }
            test.waitForIdleSync()
            onUi {
                val picker = field<Spinner>(secondEditor, "translation-mode")
                val label = secondEditor.getString(translationProviderLabel(app.yuxino.mimi.android.provider.TextTranslationProvider.OPENAI_COMPATIBLE))
                picker.setSelection((0 until picker.count).single { picker.getItemAtPosition(it).toString() == label })
            }
            test.waitForIdleSync()
            onUi {
                field<TextInputEditText>(secondEditor, "translation-endpoint").setText("https://synthetic.example.invalid/v1")
                field<TextInputEditText>(secondEditor, "translation-model").setText("synthetic-model")
                field<TextInputEditText>(secondEditor, "translation-key").setText("synthetic-text-key")
                field<Spinner>(secondEditor, "alibaba-mode").setSelection(1)
            }
            test.waitForIdleSync()
            onUi {
                check(!field<Spinner>(secondEditor, "translation-mode").isShown)
                check(field<TextInputEditText>(secondEditor, "credential-alibaba-workspace").text.toString() == "synthetic-workspace")
                field<Spinner>(secondEditor, "alibaba-mode").setSelection(0)
            }
            test.waitForIdleSync()
            onUi {
                check(field<TextInputEditText>(secondEditor, "translation-model").text.toString() == "synthetic-model")
                check(field<TextInputEditText>(secondEditor, "translation-key").text.toString() == "synthetic-text-key")
                field<Spinner>(secondEditor, "alibaba-mode").setSelection(1)
            }
            test.waitForIdleSync(); capture("simultaneous-editor")
            save(secondEditor)
            val second = SettingsStore.profiles(context).last().id.also(created::add)
            check(first != second && SettingsStore.profiles(context).size == 2)
            check(SettingsStore.activeProfileId(context) == first)
            check(SettingsStore.profileConfiguration(context, first).value("apiKey") == "synthetic-speech-one")
            check(SettingsStore.profileConfiguration(context, second).usesAlibabaLiveTranslate)
            check(SettingsStore.profileTranslationConfiguration(context, second, TextTranslationProvider.OPENAI_COMPATIBLE).apiKey == "synthetic-text-key")
            showList(); capture("two")

            stage = "edit_preserves_active_and_order"
            val edit = openFrom("configure-$second")
            onUi {
                check(field<TextInputEditText>(edit, "credential-apiKey").text.isNullOrBlank())
                check(field<Spinner>(edit, "alibaba-mode").selectedItemPosition == 1)
                field<TextInputEditText>(edit, "profile-name").setText(if (home.resources.configuration.locales[0].language == "zh") "会议同传" else "Meeting translation")
            }
            save(edit)
            check(SettingsStore.activeProfileId(context) == first)
            check(SettingsStore.profiles(context).map { it.id } == listOf(first, second))
            showList(); click(home, "select-$second")
            check(SettingsStore.activeProfileId(context) == second)
            check(SettingsStore.profiles(context).map { it.id } == listOf(first, second))

            stage = "single_choice_state"
            onUi {
                check(!field<android.widget.RadioButton>(home, "selection-$first").isChecked)
                check(field<android.widget.RadioButton>(home, "selection-$second").isChecked)
            }
            click(home, "select-$second")
            test.waitForIdleSync()
            onUi { check(home.hasWindowFocus() && field<View>(home, "configure-$second").isShown) }
            check(SettingsStore.activeProfileId(context) == second)

            stage = "selected_profile_home_summary"
            onUi { home.findViewById<android.widget.RadioGroup>(R.id.settings_tabs).check(R.id.tab_caption) }
            test.waitForIdleSync()
            onUi {
                check(home.findViewById<TextView>(R.id.provider_summary).text.toString() == SettingsStore.profiles(context).first { it.id == second }.name)
                check(home.findViewById<TextView>(R.id.model_summary).text.toString() ==
                    modelNamesLabel(runtimeModelNames(SettingsStore.runtimeConfiguration(context))))
            }
            showList()

            stage = "duplicate_and_delete_confirmation"
            val duplicateEditor = openFrom("configure-$second")
            click(duplicateEditor, "profile-more"); dialogText(R.string.profiles_duplicate)
            val copy = SettingsStore.profiles(context).last().id.also(created::add)
            check(copy != second && SettingsStore.profiles(context).size == 3)
            check(SettingsStore.profileConfiguration(context, copy).value("apiKey") == "synthetic-speech-two")
            check(SettingsStore.profileTranslationConfiguration(context, copy, TextTranslationProvider.OPENAI_COMPATIBLE).apiKey == "synthetic-text-key")
            check(SettingsStore.activeProfileId(context) == second)
            onUi { duplicateEditor.finish() }
            test.waitForIdleSync(); showList(); capture("three")
            val deleteEditor = openFrom("configure-$copy")
            click(deleteEditor, "profile-more"); dialogText(R.string.profiles_delete); dialogText(android.R.string.cancel)
            check(SettingsStore.profiles(context).size == 3)
            click(deleteEditor, "profile-more"); dialogText(R.string.profiles_delete); dialogText(R.string.profiles_delete)
            check(SettingsStore.profiles(context).map { it.id } == listOf(first, second))

            stage = "running_guard"
            onUi {
                runningFixture(true)
                ServiceSettingsUi.renderList(home, home.findViewById(R.id.service_panel))
                check(!field<View>(home, "profiles-add").isEnabled)
                for (id in listOf(first, second)) {
                    check(!field<View>(home, "configure-$id").isEnabled)
                    check(!field<View>(home, "select-$id").isEnabled)
                }
                check(!field<View>(home, "select-$first").isEnabled)
                check(field<View>(home, "service-stop").isEnabled)
                check(!SettingsStore.activateProfile(context, first))
                check(!SettingsStore.deleteProfile(context, second))
            }
            val guardedEditor = test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                .putExtra("profileId", second).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity
            activities += guardedEditor
            test.waitForIdleSync()
            onUi {
                check(!guardedEditor.findViewById<View>(R.id.save).isEnabled)
                check(!field<View>(guardedEditor, "profile-name").isEnabled)
                check(!field<View>(guardedEditor, "alibaba-mode").isEnabled)
                check(!field<View>(guardedEditor, "credential-apiKey").isEnabled)
                check(!field<View>(guardedEditor, "profile-more").isEnabled)
                runningFixture(false)
                stoppingFixture(true)
                ServiceSettingsActivity::class.java.getDeclaredMethod("refreshSessionUi").apply { isAccessible = true; invoke(guardedEditor) }
                check(!guardedEditor.findViewById<View>(R.id.save).isEnabled)
                ServiceSettingsUi.renderList(home, home.findViewById(R.id.service_panel))
                check(!field<View>(home, "profiles-add").isEnabled)
                check(!field<View>(home, "configure-$first").isEnabled)
                check(!field<View>(home, "select-$second").isEnabled)
                check(!field<View>(home, "service-stop").isEnabled)
                stoppingFixture(false)
                guardedEditor.finish()
                ServiceSettingsUi.renderList(home, home.findViewById(R.id.service_panel))
            }
            stage = "delete_active_and_last"
            val activeEditor = openFrom("configure-$second")
            click(activeEditor, "profile-more"); dialogText(R.string.profiles_delete); dialogText(R.string.profiles_delete)
            check(SettingsStore.activeProfileId(context) == first)
            awaitHomeTag("configure-$first")
            val lastEditor = openFrom("configure-$first")
            click(lastEditor, "profile-more"); dialogText(R.string.profiles_delete); dialogText(R.string.profiles_delete)
            check(SettingsStore.profiles(context).isEmpty() && SettingsStore.activeProfileId(context) == null)
            awaitHomeTag("add-dashscope")
            check(field<View>(home, "add-dashscope").isShown)
            checkRealtimeModes()
            stage = "unreadable_catalog_is_not_empty_setup"
            val persistence = AndroidProfilePersistence(context)
            val originalCatalog = checkNotNull(persistence.readCatalog())
            val guide = MainActivity::class.java.getDeclaredField("guide").apply { isAccessible = true }.get(home) as FirstRunGuide
            try {
                check(persistence.writeCatalog("{invalid-synthetic-catalog"))
                onUi {
                    ServiceSettingsUi.renderList(home, home.findViewById(R.id.service_panel))
                    check(field<View>(home, "profiles-retry").isShown)
                    check(home.window.decorView.findViewWithTag<View>("add-dashscope") == null)
                    check(home.window.decorView.findViewWithTag<View>("profiles-add") == null)
                    guide.open(0)
                }
                test.waitForIdleSync()
                onUi {
                    val windows = WindowInspector.getGlobalWindowViews()
                    check(windows.any { it.findViewWithTag<View>("guide-storage-retry")?.isShown == true })
                    check(windows.none { it.findViewWithTag<View>("guide-provider")?.isShown == true })
                }
                check(persistence.writeCatalog(originalCatalog))
                onUi {
                    val retry = WindowInspector.getGlobalWindowViews().firstNotNullOf { it.findViewWithTag<View>("guide-storage-retry") }
                    check(retry.performClick())
                }
                test.waitForIdleSync()
                onUi { check(WindowInspector.getGlobalWindowViews().any { it.findViewWithTag<View>("guide-provider")?.isShown == true }) }
            } finally {
                check(persistence.writeCatalog(originalCatalog))
                onUi { guide.dismiss(); ServiceSettingsUi.renderList(home, home.findViewById(R.id.service_panel)) }
            }
        } catch (error: Throwable) {
            failure = stage
            failureClass = error.javaClass.simpleName
            failureLocation = error.stackTrace.filter { it.className.startsWith("app.yuxino.mimi.android") }
                .take(3).joinToString(">") { "${it.fileName}:${it.lineNumber}" }
            if (ownsFixture && ::home.isInitialized) {
                runCatching { capture("failure-$stage") }
                onUi {
                    val panel = home.findViewById<View>(R.id.service_panel)
                    val ancestors = generateSequence(panel) { it.parent as? View }.joinToString("/") {
                        "${it.javaClass.simpleName}:${it.visibility}:${it.isAttachedToWindow}:${it.width}x${it.height}"
                    }
                    test.sendStatus(0, Bundle().apply { putString("stream", "Profile layout ancestors=$ancestors; windowFocus=${home.hasWindowFocus()}; finishing=${home.isFinishing}; destroyed=${home.isDestroyed}\n") })
                }
            }
        }
        finally {
            if (ownsFixture) {
                onUi {
                    runningFixture(false); stoppingFixture(false)
                    if (::home.isInitialized) {
                        home.window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                        home.setTurnScreenOn(false); home.setShowWhenLocked(false)
                    }
                    activities.forEach { if (!it.isFinishing) it.finish() }
                }
                // Only IDs created by this test are eligible for cleanup, even after an assertion fails.
                SettingsStore.profiles(context).filter { it.id in created }
                    .forEach { if (!SettingsStore.deleteProfile(context, it.id)) failure = "fixture_cleanup" }
                val editor = guidePrefs.edit()
                if (hadSeen) editor.putBoolean("seen", wasSeen) else editor.remove("seen")
                if (!editor.commit()) failure = "preference_restore"
                onUi {
                    if (locale != null) {
                        if (android.os.Build.VERSION.SDK_INT >= 33 && previousLocales != null) localeManager?.applicationLocales = previousLocales
                        else AppCompatDelegate.setApplicationLocales(previousCompatLocales)
                    }
                    AppCompatDelegate.setDefaultNightMode(previousNight)
                }
            }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "Profile UI passed=${failure == null}; failureStage=${failure ?: "none"}; errorClass=${failureClass ?: "none"}; location=${failureLocation ?: "none"}; screenshots=$screenshots; synthetic credentials only, no network or capture.\n")
        })
    }

    /** Scope only new synthetic profiles; existing metadata, active choice and secrets are untouched. */
    private fun runRealtimeModes() {
        var failure: String? = null
        var failureClass: String? = null
        var failureLocation: String? = null
        var ownsFixture = false
        val before = SettingsStore.profiles(context)
        val activeBefore = SettingsStore.activeProfileId(context)
        try {
            stage = "debug_build"
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            stage = "session_stopped"
            check(!MimiService.isRunning && !MimiService.isStopping)
            check(before.size < 20)
            ownsFixture = true
            checkRealtimeModes()
            stage = "existing_profiles_preserved"
            check(SettingsStore.profiles(context) == before)
            check(SettingsStore.activeProfileId(context) == activeBefore)
        } catch (error: Throwable) {
            failure = stage; failureClass = error.javaClass.simpleName
            failureLocation = error.stackTrace.filter { it.className.startsWith("app.yuxino.mimi.android") }
                .take(3).joinToString(">") { "${it.fileName}:${it.lineNumber}" }
        } finally {
            if (ownsFixture) {
                onUi {
                    runningFixture(false); stoppingFixture(false)
                    activities.forEach { if (!it.isFinishing) it.finish() }
                }
                SettingsStore.profiles(context).filter { it.id in created }.forEach {
                    if (!SettingsStore.deleteProfile(context, it.id)) failure = "fixture_cleanup"
                }
                if (SettingsStore.profiles(context) != before || SettingsStore.activeProfileId(context) != activeBefore)
                    failure = "existing_profiles_changed"
            }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "Realtime modes UI passed=${failure == null}; failureStage=${failure ?: "none"}; errorClass=${failureClass ?: "none"}; location=${failureLocation ?: "none"}; providers=3; originalProfiles=${before.size}; synthetic credentials only, no activation, provider requests or capture.\n")
        })
    }

    private fun checkRealtimeModes() {
        for (provider in listOf(ServiceProvider.OPENAI, ServiceProvider.GEMINI, ServiceProvider.AZURE)) {
            stage = "${provider.id}_mode_defaults"
            val beforeIds = SettingsStore.profiles(context).map { it.id }.toSet()
            val editor = test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                .putExtra("provider", provider.id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity
            activities += editor; test.waitForIdleSync()
            onUi {
                check(field<Spinner>(editor, "speech-translation-mode").selectedItemPosition == 0)
                check(!field<Spinner>(editor, "translation-mode").isShown)
                field<TextInputEditText>(editor, "profile-name").setText("Synthetic ${provider.id}")
                field<TextInputEditText>(editor, "credential-apiKey").setText("synthetic-speech-key")
                if (provider == ServiceProvider.AZURE) {
                    field<TextInputEditText>(editor, "credential-endpoint").setText("https://synthetic.openai.azure.com")
                    field<TextInputEditText>(editor, "credential-transcriptionDeployment").setText("synthetic-asr")
                }
                field<Spinner>(editor, "speech-translation-mode").setSelection(1)
            }
            test.waitForIdleSync()
            stage = "${provider.id}_independent_draft"
            onUi {
                check(field<Spinner>(editor, "translation-mode").isShown)
                check(field<View>(editor, "speech-check").isShown)
                if (provider == ServiceProvider.AZURE) check(!field<View>(editor, "credential-deployment").isShown)
                field<TextInputEditText>(editor, "translation-endpoint").setText("https://synthetic.example.invalid/v1")
                field<TextInputEditText>(editor, "translation-model").setText("synthetic-text-model")
                field<TextInputEditText>(editor, "translation-key").setText("synthetic-text-key")
                field<Spinner>(editor, "speech-translation-mode").setSelection(0)
            }
            test.waitForIdleSync()
            onUi {
                check(!field<Spinner>(editor, "translation-mode").isShown)
                if (provider == ServiceProvider.AZURE) check(field<View>(editor, "credential-deployment").isShown)
                field<Spinner>(editor, "speech-translation-mode").setSelection(1)
            }
            test.waitForIdleSync()
            onUi {
                check(field<TextInputEditText>(editor, "translation-key").text.toString() == "synthetic-text-key")
                runningFixture(true)
                ServiceSettingsActivity::class.java.getDeclaredMethod("refreshSessionUi").apply { isAccessible = true; invoke(editor) }
                check(!field<Spinner>(editor, "speech-translation-mode").isEnabled)
                check(!field<View>(editor, "speech-check").isEnabled)
                runningFixture(false)
                ServiceSettingsActivity::class.java.getDeclaredMethod("refreshSessionUi").apply { isAccessible = true; invoke(editor) }
            }
            save(editor)
            val id = SettingsStore.profiles(context).single { it.id !in beforeIds }.id.also(created::add)
            check(SettingsStore.profileConfiguration(context, id).speechTranslationMode == "asrMt")
            check(SettingsStore.profileTranslationConfiguration(context, id).apiKey == "synthetic-text-key")
            stage = "${provider.id}_simultaneous_restore"
            val restore = test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                .putExtra("profileId", id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity
            activities += restore; test.waitForIdleSync()
            onUi {
                check(field<Spinner>(restore, "speech-translation-mode").selectedItemPosition == 1)
                check(field<TextInputEditText>(restore, "translation-model").text.toString() == "synthetic-text-model")
                field<Spinner>(restore, "speech-translation-mode").setSelection(0)
            }
            test.waitForIdleSync()
            if (provider == ServiceProvider.AZURE) onUi {
                check(field<TextInputEditText>(restore, "credential-endpoint").text.toString() == "https://synthetic.openai.azure.com")
                check(field<TextInputEditText>(restore, "credential-transcriptionDeployment").text.toString() == "synthetic-asr")
                check(field<TextInputEditText>(restore, "credential-deployment").text.isNullOrBlank())
                field<TextInputEditText>(restore, "credential-deployment").setText("synthetic-translate")
            }
            save(restore)
            check(SettingsStore.profileConfiguration(context, id).speechTranslationMode == "simultaneous")
            check(SettingsStore.profileConfiguration(context, id).value("apiKey") == "synthetic-speech-key")
            check(SettingsStore.profileTranslationConfiguration(context, id).apiKey == "synthetic-text-key")
            check(SettingsStore.deleteProfile(context, id))
            if (::home.isInitialized) showList()
        }
    }

    private fun save(editor: ServiceSettingsActivity) {
        val beforeIds = SettingsStore.profiles(context).map { it.id }.toSet()
        onUi { check(editor.findViewById<View>(R.id.save).performClick()) }
        test.waitForIdleSync()
        SettingsStore.profiles(context).filter { it.id !in beforeIds }.forEach { created += it.id }
        check(editor.isFinishing)
    }
    private fun showList() {
        val priorStage = stage
        onUi {
            stage = "list_tab_enabled"
            val tab = home.findViewById<android.widget.RadioButton>(R.id.tab_service)
            check(tab.isEnabled)
            tab.performClick()
            stage = "list_tab_checked"
            check(tab.isChecked)
            stage = "list_panel_visible"
            check(home.findViewById<View>(R.id.service_panel).visibility == View.VISIBLE)
        }
        // Window attachment/visibility can lag activity creation and the first tab change.
        stage = "list_panel_shown"
        val deadline = SystemClock.elapsedRealtime() + 5_000
        var shown = false
        do {
            test.waitForIdleSync()
            onUi { shown = home.findViewById<View>(R.id.service_panel).isShown }
            if (shown) break
            SystemClock.sleep(50)
        } while (SystemClock.elapsedRealtime() < deadline)
        check(shown)
        stage = priorStage
    }
    private fun awaitHomeTag(tag: String) {
        val deadline = SystemClock.elapsedRealtime() + 5_000
        do {
            test.waitForIdleSync()
            var ready = false
            onUi { ready = home.hasWindowFocus() && home.window.decorView.findViewWithTag<View>(tag)?.isShown == true }
            if (ready) return
            SystemClock.sleep(50)
        } while (SystemClock.elapsedRealtime() < deadline)
        error("profile_list_did_not_resume")
    }
    private fun openAdded(): ServiceSettingsActivity {
        click(home, "profiles-add")
        return openFrom("add-dashscope", windows = true)
    }
    private fun openFrom(tag: String, windows: Boolean = false): ServiceSettingsActivity {
        val monitor = test.addMonitor(ServiceSettingsActivity::class.java.name, null, false)
        onUi {
            val target = if (windows) WindowInspector.getGlobalWindowViews().firstNotNullOfOrNull { it.findViewWithTag<View>(tag) }
                else field<View>(home, tag)
            check(target != null && target.isEnabled && target.performClick())
        }
        val editor = test.waitForMonitorWithTimeout(monitor, 5000) as? ServiceSettingsActivity ?: error("editor_missing")
        test.removeMonitor(monitor); activities += editor; test.waitForIdleSync(); return editor
    }
    private fun click(activity: Activity, tag: String) {
        onUi { val view = field<View>(activity, tag); check(view.isEnabled && view.performClick()) }
        test.waitForIdleSync()
    }
    private fun dialogText(label: Int) {
        onUi {
            fun find(view: View): View? = when {
                view is TextView && view.text.toString() == home.getString(label) && view.isShown -> view
                view is ViewGroup -> (0 until view.childCount).firstNotNullOfOrNull { find(view.getChildAt(it)) }
                else -> null
            }
            val target = WindowInspector.getGlobalWindowViews().asReversed().firstNotNullOfOrNull(::find) ?: error("dialog_action_missing")
            val list = generateSequence(target) { it.parent as? View }.filterIsInstance<android.widget.ListView>().firstOrNull()
            if (list != null) {
                val position = list.getPositionForView(target)
                check(position >= 0 && list.performItemClick(target, position, list.adapter.getItemId(position)))
            } else {
                val clickable = generateSequence(target) { it.parent as? View }.first { it.isClickable }
                check(clickable.isEnabled && clickable.performClick())
            }
        }
        test.waitForIdleSync()
    }
    private fun capture(name: String) {
        val expected = activities.lastOrNull { !it.isFinishing && !it.isDestroyed } ?: error("preview_activity_missing")
        val keyguard = context.getSystemService(android.app.KeyguardManager::class.java)
        val deadline = SystemClock.elapsedRealtime() + 5_000
        var visible = false
        do {
            test.waitForIdleSync()
            onUi { visible = !keyguard.isKeyguardLocked && expected.hasWindowFocus() && expected.window.decorView.isShown }
            if (visible) break
            SystemClock.sleep(50)
        } while (SystemClock.elapsedRealtime() < deadline)
        check(visible) { "preview_window_not_ready" }
        // User-facing captures may wait for native transient toasts to expire.
        SystemClock.sleep(previewSettleMs)
        onUi { check(!keyguard.isKeyguardLocked && expected.hasWindowFocus() && expected.window.decorView.isShown) }
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        try {
            val dir = File(context.filesDir, "ui-preview").apply { check(isDirectory || mkdirs()) }
            File(dir, "profiles-$name-$theme.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        } finally { bitmap.recycle() }
        screenshots++
    }
    private fun runningFixture(running: Boolean) {
        MimiService::class.java.getDeclaredField("isRunning").apply { isAccessible = true; set(null, running) }
    }
    private fun stoppingFixture(stopping: Boolean) {
        MimiService::class.java.getDeclaredField("isStopping").apply { isAccessible = true; set(null, stopping) }
    }
    private inline fun <reified T : View> field(activity: Activity, tag: String): T =
        activity.findViewById<View>(android.R.id.content).findViewWithTag<T>(tag) ?: error("control_missing")
    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
}

/** Recover only this fixture's five PNG previews, using the owning app's scoped storage access. */
internal fun copyProfileScreenshots(context: android.content.Context, theme: String): String {
    check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
    check(theme in setOf("light", "dark"))
    val source = checkNotNull(context.getExternalFilesDir("ui-preview"))
    val destination = File(context.filesDir, "ui-preview").apply { check(isDirectory || mkdirs()) }
    val names = listOf("empty", "one", "two", "three", "simultaneous-editor")
    var copied = 0
    for (name in names) {
        val filename = "profiles-$name-$theme.png"
        val input = File(source, filename)
        if (!input.exists()) continue
        check(input.isFile && input.canonicalFile.parentFile == source.canonicalFile && input.length() in 8..20_000_000)
        val header = input.inputStream().use { stream -> ByteArray(8).also { check(stream.read(it) == it.size) } }
        check(header.contentEquals(byteArrayOf(137.toByte(), 80, 78, 71, 13, 10, 26, 10)))
        input.copyTo(File(destination, filename), overwrite = true)
        copied++
    }
    return "Profile screenshot export copied=$copied; app-owned PNG previews only."
}
