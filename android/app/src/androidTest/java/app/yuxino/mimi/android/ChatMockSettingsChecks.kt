package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.app.LocaleManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Bundle
import android.os.Build
import android.os.LocaleList
import android.os.SystemClock
import android.text.InputType
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.widget.CheckBox
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout
import java.io.File

/** Local form checks on a dedicated blank emulator: no provider request or audio capture. */
internal class ChatMockSettingsChecks(private val instrumentation: Instrumentation) {
    private val context get() = instrumentation.targetContext
    private var theme = "light"
    private var screenshots = 0
    private var savedFixture = false
    private var connectionFixture = false
    private val activities = mutableListOf<Activity>()

    fun run(arguments: Bundle?) {
        theme = arguments?.getString("theme") ?: "light"
        savedFixture = arguments?.getString("chatmock_saved") == "true"
        connectionFixture = arguments?.getString("chatmock_connection") == "true"
        val locale = arguments?.getString("locale")?.also { check(it in listOf("zh", "en", "ja")) { "Unsupported UI fixture locale" } }
        var failure: Throwable? = null
        val previousNightMode = AppCompatDelegate.getDefaultNightMode()
        val localeManager = if (Build.VERSION.SDK_INT >= 33) context.getSystemService(LocaleManager::class.java) else null
        val previousLocales = if (Build.VERSION.SDK_INT >= 33) localeManager?.applicationLocales else null
        val previousCompatLocales = AppCompatDelegate.getApplicationLocales()
        try {
            check(!MimiService.isRunning) { "Stop the active session before ChatMock UI checks" }
            check(ServiceProvider.entries.all { provider ->
                SettingsStore.configuration(context, provider).credentials.values.all(String::isBlank)
            }) { "Use a dedicated blank emulator for ChatMock UI checks" }
            check(!SettingsStore.useChatMockTranslation(context)) { "Use a blank ChatMock configuration" }
            check(TextTranslationProvider.entries.all { SettingsStore.translationConfiguration(context, it).apiKey.isBlank() }) { "Use blank translation keys" }
            onUi {
                if (locale != null) {
                    if (Build.VERSION.SDK_INT >= 33) localeManager?.applicationLocales = LocaleList.forLanguageTags(locale)
                    else AppCompatDelegate.setApplicationLocales(androidx.core.os.LocaleListCompat.forLanguageTags(locale))
                }
                AppCompatDelegate.setDefaultNightMode(if (theme == "dark")
                    AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO)
            }
            if (connectionFixture) checkConnection() else {
                runTextTranslationStorageChecks(context)
                checkForms()
                if (savedFixture) checkSavedCredentials()
            }
        } catch (error: Throwable) {
            failure = error
        } finally {
            onUi {
                activities.forEach { if (!it.isFinishing) it.finish() }
                if (locale != null) {
                    if (Build.VERSION.SDK_INT >= 33 && previousLocales != null) localeManager?.applicationLocales = previousLocales
                    else AppCompatDelegate.setApplicationLocales(previousCompatLocales)
                }
                AppCompatDelegate.setDefaultNightMode(previousNightMode)
            }
        }
        instrumentation.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null && connectionFixture)
                "Text translation connection UI passed ($theme): $screenshots native screenshots; local HTTP fixture, no external provider or audio; OpenAI/DeepLX POST, progress/duration, cancelled stale response, 401 and quota guidance verified; no credentials or configuration saved.\n"
            else if (failure == null)
                "ChatMock settings passed ($theme): $screenshots native screenshots; independent recognition/translation fields, local validation, help dialog and discarded drafts; ${if (savedFixture) "synthetic save/reopen/key-isolation fixture restored" else "no credentials saved"}; no provider request or capture started.\n"
            else "ChatMock settings failed: ${failure.javaClass.simpleName}: ${failure.message}\n")
        })
    }

    /** Exercise the real HTTP client and Android cleartext policy against this process only. */
    private fun checkConnection() {
        val originalSpeech = SettingsStore.configuration(context, ServiceProvider.DASHSCOPE)
        val originalTranslation = SettingsStore.translationConfiguration(context)
        val originalTranslations = TextTranslationProvider.entries.associateWith { SettingsStore.translationConfiguration(context, it) }
        val originalProvider = SettingsStore.provider(context)
        val originalEnabled = SettingsStore.useChatMockTranslation(context)
        ChatMockLoopbackFixture().use { server ->
            val editor = open(ServiceProvider.DASHSCOPE)
            onUi { selectMode(editor, TextTranslationProvider.CHAT_MOCK) }
            instrumentation.waitForIdleSync()
            onUi {
                field<TextInputEditText>(editor, "translation-endpoint").setText(server.baseUrl)
                field<TextInputEditText>(editor, "translation-model").setText("synthetic-success")
                field<TextInputEditText>(editor, "translation-key").setText("")
                field<CheckBox>(editor, "translation-local-http").isChecked = true
            }
            show(editor, "translation-check")
            val startedAt = SystemClock.elapsedRealtime()
            click(editor, "translation-check")
            server.awaitRequest(0)
            onUi {
                val action = field<TextView>(editor, "translation-check")
                check(!action.isEnabled && action.text.toString() == context.getString(R.string.translation_checking)) {
                    "Pending connection check did not show progress on its action"
                }
                check(field<TextView>(editor, "translation-status").visibility == View.GONE)
            }
            capture("chatmock-check-progress-$theme")
            server.releaseResponse(0)
            awaitCheckResult(editor)
            val elapsed = assertTimedResult(editor, R.string.translation_check_success)
            check(elapsed >= 200 && elapsed <= SystemClock.elapsedRealtime() - startedAt + 100) {
                "Successful check duration does not measure the real held HTTP request"
            }
            capture("chatmock-check-success-$theme")

            onUi { field<TextInputEditText>(editor, "translation-model").setText("synthetic-slow") }
            click(editor, "translation-check")
            server.awaitRequest(1)
            onUi {
                check(!field<View>(editor, "translation-check").isEnabled)
                field<TextInputEditText>(editor, "translation-model").setText("synthetic-after-edit")
                check(field<View>(editor, "translation-check").isEnabled) { "Editing a draft did not cancel the pending check" }
            }
            server.releaseResponse(1)
            server.awaitResponse(1)
            // Give a late callback ample time to try to overwrite the edited draft's state.
            SystemClock.sleep(700)
            instrumentation.waitForIdleSync()
            onUi {
                check(field<TextView>(editor, "translation-status").visibility == View.GONE) {
                    "A cancelled check displayed a stale result after the draft changed"
                }
                check(field<TextView>(editor, "translation-check").text.toString() == context.getString(R.string.translation_check))
            }
            capture("chatmock-check-cancelled-$theme")

            onUi { field<TextInputEditText>(editor, "translation-model").setText("synthetic-auth") }
            click(editor, "translation-check")
            server.awaitRequest(2)
            server.releaseResponse(2)
            awaitCheckResult(editor)
            assertTimedResult(editor, R.string.translation_check_auth)
            capture("chatmock-check-auth-$theme")
            server.awaitResponse(2)
            onUi { selectMode(editor, TextTranslationProvider.DEEPLX) }
            instrumentation.waitForIdleSync()
            onUi {
                check(!field<View>(editor, "translation-model").isShown)
                check(field<TextInputEditText>(editor, "translation-key").text.isNullOrBlank())
                field<TextInputEditText>(editor, "translation-endpoint").setText(server.deepLXUrl)
                field<TextInputEditText>(editor, "translation-key").setText("synthetic-deeplx-local-token")
                field<CheckBox>(editor, "translation-local-http").isChecked = true
            }
            click(editor, "translation-check")
            server.awaitRequest(3); server.releaseResponse(3)
            awaitCheckResult(editor)
            assertTimedResult(editor, R.string.translation_check_success)
            capture("translation-deeplx-check-success-$theme")
            server.awaitResponse(3)
            click(editor, "translation-check")
            server.awaitRequest(4); server.releaseResponse(4)
            awaitCheckResult(editor)
            assertTimedResult(editor, R.string.translation_check_busy)
            capture("translation-deeplx-check-quota-$theme")
            server.awaitResponse(4)
            server.assertHealthy()
            onUi { editor.finish() }
            instrumentation.waitForIdleSync()
        }
        check(sameSpeechConfiguration(SettingsStore.configuration(context, ServiceProvider.DASHSCOPE), originalSpeech))
        check(SettingsStore.translationConfiguration(context) == originalTranslation)
        check(TextTranslationProvider.entries.all { SettingsStore.translationConfiguration(context, it) == originalTranslations[it] })
        check(SettingsStore.provider(context) == originalProvider && SettingsStore.useChatMockTranslation(context) == originalEnabled)
        check(!MimiService.isRunning) { "Connection check started capture" }
    }

    private fun awaitCheckResult(activity: Activity) {
        val deadline = SystemClock.elapsedRealtime() + 8_000
        do {
            var ready = false
            onUi {
                ready = field<View>(activity, "translation-check").isEnabled &&
                    field<TextView>(activity, "translation-status").visibility == View.VISIBLE
            }
            if (ready) return
            SystemClock.sleep(50)
        } while (SystemClock.elapsedRealtime() < deadline)
        error("Local connection check did not finish")
    }

    private fun assertTimedResult(activity: Activity, messageId: Int): Long {
        var elapsed = -1L
        onUi {
            val status = field<TextView>(activity, "translation-status")
            val text = status.text.toString()
            elapsed = Regex("([0-9]+)\\s*ms").find(text)?.groupValues?.get(1)?.toLongOrNull()
                ?: error("Connection result omitted elapsed milliseconds")
            check(text == context.getString(messageId, elapsed)) { "Connection result did not match the expected outcome" }
            check(status.isShown && status.textSize / status.resources.displayMetrics.scaledDensity >= 14f)
        }
        return elapsed
    }

    private fun checkForms() {
        val originalSpeech = SettingsStore.configuration(context, ServiceProvider.DASHSCOPE)
        val originalTranslation = SettingsStore.translationConfiguration(context)
        val originalTranslations = TextTranslationProvider.entries.associateWith { SettingsStore.translationConfiguration(context, it) }
        val originalProvider = SettingsStore.provider(context)
        val originalEnabled = SettingsStore.useChatMockTranslation(context)
        fun unchanged() {
            check(sameSpeechConfiguration(SettingsStore.configuration(context, ServiceProvider.DASHSCOPE), originalSpeech)) { "Draft changed speech credentials" }
            check(SettingsStore.translationConfiguration(context) == originalTranslation) { "Draft changed translation settings" }
            check(TextTranslationProvider.entries.all { SettingsStore.translationConfiguration(context, it) == originalTranslations[it] }) { "A draft changed another translator" }
            check(SettingsStore.useChatMockTranslation(context) == originalEnabled) { "Draft enabled ChatMock" }
            check(SettingsStore.provider(context) == originalProvider) { "Draft changed active service" }
            check(!MimiService.isRunning) { "Settings started an audio session" }
        }

        val editor = open(ServiceProvider.DASHSCOPE)
        val mode = field<Spinner>(editor, "translation-mode")
        val speechKey = field<TextInputEditText>(editor, "credential-apiKey")
        val translationKey = field<TextInputEditText>(editor, "translation-key")
        onUi {
            check(mode.selectedItemPosition == 0 && mode.count == 6) { "Built-in translation must remain the default among six text choices" }
            check((0 until mode.count).map { mode.getItemAtPosition(it).toString() } ==
                listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.DEEPL, TextTranslationProvider.DEEPLX,
                    TextTranslationProvider.CHAT_MOCK, TextTranslationProvider.OPENAI_COMPATIBLE, TextTranslationProvider.NONE)
                    .map { editor.getString(translationProviderLabel(it)) }) { "Text services are not in the expected order" }
            check(!translationKey.isShown) { "Custom fields must follow the selected translation mode" }
            checkSecretField(speechKey)
            checkSecretField(translationKey)
            check(speechKey !== translationKey) { "Recognition and translation share a credential input" }
        }
        capture("chatmock-builtin-$theme")
        onUi { check(mode.performClick()) }
        instrumentation.waitForIdleSync()
        capture("translation-provider-menu-$theme")
        instrumentation.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        instrumentation.waitForIdleSync()
        onUi { selectMode(editor, TextTranslationProvider.CHAT_MOCK) }
        instrumentation.waitForIdleSync()
        unchanged()
        onUi {
            check(translationKey.isShown) { "ChatMock selection did not reveal its fields" }
            check(mode.selectedItem.toString() == "ChatMock") { "ChatMock must have its own entry" }
            check(field<TextInputEditText>(editor, "translation-endpoint").text.toString() == "http://127.0.0.1:8000/v1")
            check(field<TextInputEditText>(editor, "translation-model").text.isNullOrBlank()) { "ChatMock must not guess a model" }
            check(!field<CheckBox>(editor, "translation-local-http").isChecked) { "HTTP must require explicit consent" }
        }
        show(editor, "translation-endpoint")
        assertHorizontalFit(editor, "translation-mode", "translation-endpoint", "translation-model", "translation-key")
        capture("chatmock-fields-$theme")
        click(editor, "translation-check")
        onUi { check(field<TextView>(editor, "translation-status").text.toString() == context.getString(R.string.translation_required)) }
        unchanged()
        checkProviderDrafts(editor)

        click(editor, "translation-help")
        onUi {
            val dialog = WindowInspector.getGlobalWindowViews().firstOrNull {
                it.findViewWithTag<TextView>("help-message")?.isShown == true
            } ?: error("Requirements must open a standard help dialog")
            val message = dialog.findViewWithTag<TextView>("help-message")
            check(message.text.isNotBlank()) { "ChatMock requirements are missing" }
            check(message.textSize / message.resources.displayMetrics.scaledDensity >= 14f) { "Help copy is too small" }
            check(dialog.findViewById<View>(android.R.id.button1)?.isShown == true) { "Help has no dismissal action" }
        }
        capture("chatmock-help-$theme")
        onUi {
            val dialog = WindowInspector.getGlobalWindowViews().first {
                it.findViewWithTag<TextView>("help-message")?.isShown == true
            }
            check(dialog.findViewById<View>(android.R.id.button1).performClick())
        }
        instrumentation.waitForIdleSync()

        // Empty fields are rejected before a request can be constructed.
        onUi {
            field<TextInputEditText>(editor, "translation-endpoint").setText("")
            field<TextInputEditText>(editor, "translation-model").setText("")
        }
        click(editor, "translation-check")
        checkValidation(editor)
        unchanged()
        show(editor, "translation-check")
        capture("chatmock-missing-fields-$theme")
        onUi { check(editor.findViewById<View>(R.id.save).performClick()) }
        instrumentation.waitForIdleSync()
        check(!editor.isFinishing) { "Invalid form was saved" }
        unchanged()

        // The form holds synthetic drafts only. Toggling local HTTP never saves or connects.
        onUi {
            speechKey.setText("synthetic-unsaved-speech-key")
            field<TextInputEditText>(editor, "translation-endpoint").setText("http://127.0.0.1:18000/v1")
            field<TextInputEditText>(editor, "translation-model").setText("synthetic-model")
            translationKey.setText("synthetic-unsaved-translation-key")
        }
        // A loopback HTTP URL without consent must fail locally, even with complete fields.
        click(editor, "translation-check")
        onUi {
            check(field<TextView>(editor, "translation-status").text.toString() == context.getString(R.string.translation_invalid)) {
                "HTTP without consent was not rejected locally"
            }
        }
        unchanged()
        show(editor, "translation-check")
        capture("chatmock-http-consent-$theme")
        onUi {
            field<CheckBox>(editor, "translation-local-http").isChecked = true
            field<TextInputEditText>(editor, "translation-endpoint").setText("http://192.168.1.20:18000/v1")
        }
        click(editor, "translation-check")
        onUi {
            check(field<TextView>(editor, "translation-status").text.toString() == context.getString(R.string.translation_invalid)) {
                "HTTP consent must not allow a LAN destination"
            }
            field<TextInputEditText>(editor, "translation-endpoint").setText("http://127.0.0.1:18000/v1")
        }
        unchanged()
        onUi { check(editor.findViewById<View>(R.id.back).performClick()) }
        instrumentation.waitForIdleSync()
        unchanged()

        val reopened = open(ServiceProvider.DASHSCOPE)
        onUi {
            check(field<Spinner>(reopened, "translation-mode").selectedItemPosition == 0) { "Unsaved mode survived reopening" }
            checkSecretField(field(reopened, "credential-apiKey"))
            checkSecretField(field(reopened, "translation-key"))
            check(field<TextInputEditText>(reopened, "translation-endpoint").text.toString() == originalTranslation.endpoint) { "Unsaved endpoint survived reopening" }
            check(!field<CheckBox>(reopened, "translation-local-http").isChecked) { "Unsaved HTTP consent survived reopening" }
            reopened.finish()
        }
        instrumentation.waitForIdleSync()
        val openAI = open(ServiceProvider.OPENAI)
        onUi {
            check(openAI.findViewById<View>(android.R.id.content).findViewWithTag<View>("translation-mode") == null) {
                "ChatMock must not be presented as a realtime recognition provider"
            }
            openAI.finish()
        }
        unchanged()
    }

    /** Opt-in fixture: only synthetic keys on an already-verified blank emulator. */
    private fun checkSavedCredentials() = withTextTranslationSettingsSnapshot(context) {
        try {
            val editor = open(ServiceProvider.DASHSCOPE)
            onUi { selectMode(editor, TextTranslationProvider.CHAT_MOCK) }
            instrumentation.waitForIdleSync()
            onUi {
                field<TextInputEditText>(editor, "credential-apiKey").setText("synthetic-saved-speech-key")
                field<TextInputEditText>(editor, "translation-endpoint").setText("https://chatmock.example.invalid/v1")
                field<TextInputEditText>(editor, "translation-model").setText("synthetic-model")
                field<TextInputEditText>(editor, "translation-key").setText("synthetic-saved-translation-key")
                field<CheckBox>(editor, "translation-local-http").isChecked = false
                check(editor.findViewById<View>(R.id.save).performClick())
            }
            instrumentation.waitForIdleSync()
            check(editor.isFinishing) { "Valid draft did not save" }
            check(SettingsStore.useChatMockTranslation(context)) { "Saved ChatMock selection was lost" }
            check(SettingsStore.configuration(context, ServiceProvider.DASHSCOPE).value("apiKey") == "synthetic-saved-speech-key") {
                "Translation save overwrote the speech key"
            }
            check(SettingsStore.translationConfiguration(context).apiKey == "synthetic-saved-translation-key") {
                "Translation key was not saved separately"
            }
            val reopened = open(ServiceProvider.DASHSCOPE)
            onUi {
                check(field<Spinner>(reopened, "translation-mode").selectedItem.toString() == "ChatMock")
                checkSecretField(field(reopened, "credential-apiKey"))
                checkSecretField(field(reopened, "translation-key"))
                check(field<TextInputEditText>(reopened, "translation-endpoint").text.toString() == "https://chatmock.example.invalid/v1")
            }
            show(reopened, "translation-key")
            capture("chatmock-saved-write-only-$theme")
            // Removing a saved key still follows the draft/save boundary.
            click(reopened, "translation-remove-key")
            onUi { check(reopened.findViewById<View>(R.id.back).performClick()) }
            instrumentation.waitForIdleSync()
            check(SettingsStore.translationConfiguration(context).apiKey == "synthetic-saved-translation-key") {
                "Unsaved removal deleted the stored key"
            }
            val compatible = open(ServiceProvider.DASHSCOPE)
            onUi { selectMode(compatible, TextTranslationProvider.OPENAI_COMPATIBLE) }
            instrumentation.waitForIdleSync()
            onUi {
                check(field<TextInputEditText>(compatible, "translation-endpoint").text.isNullOrBlank()) { "Generic entry inherited ChatMock's address" }
                check(field<TextInputEditText>(compatible, "translation-model").text.isNullOrBlank())
                checkSecretField(field(compatible, "translation-key"))
                field<TextInputEditText>(compatible, "translation-endpoint").setText("https://compatible.example.invalid/v1")
                field<TextInputEditText>(compatible, "translation-model").setText("synthetic-compatible-model")
                field<TextInputEditText>(compatible, "translation-key").setText("synthetic-compatible-key")
                check(compatible.findViewById<View>(R.id.save).performClick())
            }
            instrumentation.waitForIdleSync()
            check(compatible.isFinishing && SettingsStore.textTranslationProvider(context) == TextTranslationProvider.OPENAI_COMPATIBLE)
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.CHAT_MOCK).apiKey == "synthetic-saved-translation-key")
            val deepL = open(ServiceProvider.DASHSCOPE)
            onUi { selectMode(deepL, TextTranslationProvider.DEEPL) }
            instrumentation.waitForIdleSync()
            onUi {
                checkSecretField(field(deepL, "translation-key"))
                field<TextInputEditText>(deepL, "translation-key").setText("synthetic-deepl-saved:fx")
                check(deepL.findViewById<View>(R.id.save).performClick())
            }
            instrumentation.waitForIdleSync()
            check(deepL.isFinishing && SettingsStore.textTranslationProvider(context) == TextTranslationProvider.DEEPL)
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.CHAT_MOCK).apiKey == "synthetic-saved-translation-key")
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.OPENAI_COMPATIBLE).apiKey == "synthetic-compatible-key")
            val deepLX = open(ServiceProvider.DASHSCOPE)
            onUi {
                checkSecretField(field(deepLX, "translation-key"))
                check(!field<View>(deepLX, "translation-endpoint").isShown)
                check(!field<View>(deepLX, "translation-remove-key").isShown)
            }
            show(deepLX, "translation-key")
            capture("translation-deepl-saved-$theme")
            onUi { selectMode(deepLX, TextTranslationProvider.DEEPLX) }
            instrumentation.waitForIdleSync()
            onUi {
                checkSecretField(field(deepLX, "translation-key"))
                field<TextInputEditText>(deepLX, "translation-endpoint").setText("https://deeplx.example.invalid/translate")
                field<TextInputEditText>(deepLX, "translation-key").setText("synthetic-deeplx-saved")
                check(deepLX.findViewById<View>(R.id.save).performClick())
            }
            instrumentation.waitForIdleSync()
            check(deepLX.isFinishing && SettingsStore.textTranslationProvider(context) == TextTranslationProvider.DEEPLX)
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.DEEPL).apiKey == "synthetic-deepl-saved:fx")
            val changed = open(ServiceProvider.DASHSCOPE)
            onUi { checkSecretField(field(changed, "translation-key")) }
            show(changed, "translation-key")
            capture("translation-deeplx-saved-$theme")
            onUi { selectMode(changed, TextTranslationProvider.CHAT_MOCK) }
            instrumentation.waitForIdleSync()
            onUi {
                field<TextInputEditText>(changed, "translation-endpoint").setText("https://other.example.invalid/v1")
                check(changed.findViewById<View>(R.id.save).performClick())
            }
            instrumentation.waitForIdleSync()
            check(changed.isFinishing)
            check(SettingsStore.translationConfiguration(context).apiKey.isBlank()) { "Changing destinations forwarded the saved translation key" }
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.OPENAI_COMPATIBLE).apiKey == "synthetic-compatible-key") {
                "Changing a ChatMock address affected the generic compatible key"
            }
            check(SettingsStore.configuration(context, ServiceProvider.DASHSCOPE).value("apiKey") == "synthetic-saved-speech-key") {
                "Changing translation destination changed the speech key"
            }
            check(!MimiService.isRunning)
            val appearance = (instrumentation.startActivitySync(Intent(context, SettingsActivity::class.java)
                .putExtra("settings_section", "appearance").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as SettingsActivity).also { activities += it }
            instrumentation.waitForIdleSync()
            onUi { check(previewTranslation(appearance).visibility == View.VISIBLE) }
            val originalOnly = open(ServiceProvider.DASHSCOPE)
            onUi { selectMode(originalOnly, TextTranslationProvider.NONE) }
            instrumentation.waitForIdleSync()
            onUi {
                check(!field<View>(originalOnly, "translation-check").isShown)
                check(originalOnly.findViewById<View>(R.id.save).performClick())
            }
            instrumentation.waitForIdleSync()
            check(originalOnly.isFinishing && SettingsStore.originalTextOnly(context))
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.DEEPLX).apiKey == "synthetic-deeplx-saved")
            onUi { check(previewTranslation(appearance).visibility == View.GONE) { "Returning from original-only save left a translated appearance preview" } }
            capture("translation-original-appearance-$theme")
            val firstRun = context.getSharedPreferences("first_run", 0)
            val hadSeen = firstRun.contains("seen")
            val oldSeen = firstRun.getBoolean("seen", false)
            try {
                check(firstRun.edit().putBoolean("seen", true).commit())
                val home = (instrumentation.startActivitySync(Intent(context, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity).also { activities += it }
                instrumentation.waitForIdleSync()
                onUi {
                    check(!home.findViewById<View>(R.id.target_language_action).isEnabled) { "Original-only mode still offers a target language" }
                    check(home.findViewById<TextView>(R.id.target_summary).text.toString() == context.getString(R.string.translation_none))
                    check(previewTranslation(home).visibility == View.GONE) { "Original-only home shows a sample translation" }
                }
                capture("translation-original-home-$theme")
                onUi { home.finish() }
            } finally {
                check(firstRun.edit().apply { if (hadSeen) putBoolean("seen", oldSeen) else remove("seen") }.commit())
            }
        } finally {
            onUi { activities.forEach { if (!it.isFinishing) it.finish() } }
        }
    }

    private fun checkProviderDrafts(editor: ServiceSettingsActivity) {
        fun select(provider: TextTranslationProvider) {
            onUi { selectMode(editor, provider) }
            instrumentation.waitForIdleSync()
            onUi {
                val label = field<Spinner>(editor, "translation-mode").selectedView as TextView
                val icon = checkNotNull(label.compoundDrawablesRelative[0]) { "Translation choice has no icon" }
                val size = (24 * label.resources.displayMetrics.density).toInt()
                check(icon.bounds.width() == size && icon.bounds.height() == size) { "Translation icons use inconsistent dimensions" }
                check(label.textSize / label.resources.displayMetrics.scaledDensity >= 16f)
            }
        }
        onUi {
            field<TextInputEditText>(editor, "translation-endpoint").setText("https://draft.example.invalid/v1")
            field<TextInputEditText>(editor, "translation-model").setText("draft-model")
            field<TextInputEditText>(editor, "translation-key").setText("synthetic-chatmock-draft")
        }
        select(TextTranslationProvider.OPENAI_COMPATIBLE)
        onUi {
            check(field<Spinner>(editor, "translation-mode").selectedItem.toString() == context.getString(R.string.translation_openai_compatible))
            check(field<TextInputEditText>(editor, "translation-endpoint").text.isNullOrBlank())
            check(field<TextInputEditText>(editor, "translation-model").text.isNullOrBlank())
            checkSecretField(field(editor, "translation-key"))
        }
        show(editor, "translation-endpoint")
        capture("translation-compatible-fields-$theme")
        onUi {
            field<TextInputEditText>(editor, "translation-endpoint").setText("https://compatible.example.invalid/v1")
            field<TextInputEditText>(editor, "translation-model").setText("compatible-draft-model")
            field<TextInputEditText>(editor, "translation-key").setText("synthetic-compatible-draft")
        }
        select(TextTranslationProvider.DEEPL)
        onUi {
            check(!field<View>(editor, "translation-endpoint").isShown && !field<View>(editor, "translation-model").isShown)
            check(!field<View>(editor, "translation-local-http").isShown)
            checkSecretField(field(editor, "translation-key"))
        }
        click(editor, "translation-check")
        onUi {
            check(field<TextView>(editor, "translation-status").text.toString() == context.getString(R.string.translation_key_required))
        }
        show(editor, "translation-check")
        capture("translation-deepl-$theme")
        onUi { field<TextInputEditText>(editor, "translation-key").setText("synthetic-deepl-draft:fx") }
        select(TextTranslationProvider.DEEPLX)
        onUi {
            check(field<View>(editor, "translation-endpoint").isShown && !field<View>(editor, "translation-model").isShown)
            check(field<View>(editor, "translation-local-http").isShown)
            checkSecretField(field(editor, "translation-key"))
            check(field<TextInputEditText>(editor, "translation-endpoint").text.isNullOrBlank())
            field<TextInputEditText>(editor, "translation-endpoint").setText("https://deeplx.example.invalid/translate")
            field<TextInputEditText>(editor, "translation-key").setText("synthetic-deeplx-draft")
        }
        show(editor, "translation-endpoint")
        capture("translation-deeplx-$theme")
        select(TextTranslationProvider.NONE)
        onUi {
            check(!field<View>(editor, "translation-key").isShown && !field<View>(editor, "translation-endpoint").isShown)
            check(!field<View>(editor, "translation-check").isShown) { "Original-only mode offers a translation request" }
        }
        show(editor, "translation-mode")
        capture("translation-original-only-$theme")
        select(TextTranslationProvider.DEEPL)
        onUi { check(field<TextInputEditText>(editor, "translation-key").text.toString() == "synthetic-deepl-draft:fx") }
        select(TextTranslationProvider.DEEPLX)
        onUi {
            check(field<TextInputEditText>(editor, "translation-key").text.toString() == "synthetic-deeplx-draft")
            check(field<TextInputEditText>(editor, "translation-endpoint").text.toString() == "https://deeplx.example.invalid/translate")
        }
        select(TextTranslationProvider.CHAT_MOCK)
        onUi {
            check(field<TextInputEditText>(editor, "translation-endpoint").text.toString() == "https://draft.example.invalid/v1")
            check(field<TextInputEditText>(editor, "translation-model").text.toString() == "draft-model")
            check(field<TextInputEditText>(editor, "translation-key").text.toString() == "synthetic-chatmock-draft")
        }
        select(TextTranslationProvider.OPENAI_COMPATIBLE)
        onUi {
            check(field<TextInputEditText>(editor, "translation-endpoint").text.toString() == "https://compatible.example.invalid/v1")
            check(field<TextInputEditText>(editor, "translation-model").text.toString() == "compatible-draft-model")
            check(field<TextInputEditText>(editor, "translation-key").text.toString() == "synthetic-compatible-draft")
        }
        select(TextTranslationProvider.CHAT_MOCK)
    }

    private fun selectMode(activity: Activity, provider: TextTranslationProvider) {
        val mode = field<Spinner>(activity, "translation-mode")
        val label = activity.getString(translationProviderLabel(provider))
        mode.setSelection((0 until mode.count).single { mode.getItemAtPosition(it).toString() == label })
    }

    private fun previewTranslation(activity: Activity): View = SubtitlePreviewView::class.java.getDeclaredField("translation").run {
        isAccessible = true
        get(activity.findViewById<SubtitlePreviewView>(R.id.subtitle_preview)) as View
    }

    private fun sameSpeechConfiguration(actual: ServiceConfiguration, expected: ServiceConfiguration): Boolean =
        actual.provider == expected.provider && actual.credentials == expected.credentials &&
            actual.endpoint == expected.endpoint && actual.model == expected.model

    private fun checkSecretField(input: TextInputEditText) {
        check(input.text.isNullOrEmpty()) { "Credential field must start empty" }
        check(!input.isSaveEnabled) { "Credential must not enter Android view saved state" }
        check(input.inputType and InputType.TYPE_TEXT_VARIATION_PASSWORD != 0) { "Credential input is not masked" }
        check(input.importantForAutofill == View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS) { "Credential input allows autofill retention" }
    }

    private fun checkValidation(activity: Activity) = onUi {
        val status = field<TextView>(activity, "translation-status")
        fun hasError(view: View): Boolean =
            (view is TextInputLayout && !view.error.isNullOrBlank()) ||
                (view is ViewGroup && (0 until view.childCount).any { hasError(view.getChildAt(it)) })
        check(status.text.isNotBlank() || hasError(activity.findViewById(android.R.id.content))) { "Missing fields had no actionable validation" }
        check(!activity.isFinishing) { "Validation closed the unsaved editor" }
    }

    private fun open(provider: ServiceProvider): ServiceSettingsActivity =
        (instrumentation.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
            .putExtra("provider", provider.id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity).also {
            activities += it
            instrumentation.waitForIdleSync()
        }

    private inline fun <reified T : View> field(activity: Activity, tag: String): T =
        activity.findViewById<View>(android.R.id.content).findViewWithTag<T>(tag) ?: error("Control missing: $tag")

    private fun click(activity: Activity, tag: String) {
        onUi {
            val view = field<View>(activity, tag)
            check(view.isEnabled && view.isClickable && view.performClick()) { "Control is unavailable: $tag" }
        }
        instrumentation.waitForIdleSync()
    }

    private fun show(activity: Activity, tag: String) {
        onUi {
            val target = field<View>(activity, tag)
            var parent = target.parent
            while (parent != null && parent !is ScrollView) parent = parent.parent
            val scroll = parent as? ScrollView ?: error("Settings form must scroll")
            val rect = Rect(0, 0, target.width, target.height)
            scroll.offsetDescendantRectToMyCoords(target, rect)
            scroll.scrollTo(0, (rect.top - (24 * target.resources.displayMetrics.density).toInt()).coerceAtLeast(0))
        }
        instrumentation.waitForIdleSync()
    }

    private fun assertHorizontalFit(activity: Activity, vararg tags: String) = onUi {
        val root = activity.findViewById<View>(android.R.id.content)
        val bounds = IntArray(2).also(root::getLocationOnScreen)
        tags.forEach { tag ->
            val view = field<View>(activity, tag)
            val location = IntArray(2).also(view::getLocationOnScreen)
            check(view.width > 0 && location[0] >= bounds[0] && location[0] + view.width <= bounds[0] + root.width) {
                "Control extends outside the settings width: $tag"
            }
        }
    }

    private fun capture(name: String) {
        instrumentation.waitForIdleSync()
        Thread.sleep(400)
        val bitmap = checkNotNull(instrumentation.uiAutomation.takeScreenshot()) { "Screenshot unavailable" }
        try {
            val dir = checkNotNull(context.getExternalFilesDir("ui-preview"))
            File(dir, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        } finally { bitmap.recycle() }
        screenshots++
        instrumentation.sendStatus(0, Bundle().apply { putString("stream", "Captured $name\n") })
    }

    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        instrumentation.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
}
