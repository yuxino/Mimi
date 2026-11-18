package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.app.LocaleManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.os.LocaleList
import android.os.SystemClock
import android.text.InputType
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.widget.CheckBox
import android.widget.ListView
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import app.yuxino.mimi.android.captions.SubtitlePreviewView
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
    private var stage = "preconditions"
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
            stage = if (connectionFixture) "connection" else "storage"
            if (connectionFixture) checkConnection() else {
                runTextTranslationStorageChecks(context)
                stage = "forms"
                checkForms()
                if (savedFixture) {
                    stage = "saved_credentials"
                    checkSavedCredentials()
                }
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
            else "ChatMock settings failed: ${failureSummary(requireNotNull(failure))}\n")
        })
    }

    /** Exercise the real HTTP client and Android cleartext policy against this process only. */
    private fun checkConnection() {
        val originalSpeech = SettingsStore.configuration(context, ServiceProvider.DASHSCOPE)
        val originalTranslation = SettingsStore.translationConfiguration(context)
        val originalTranslations = TextTranslationProvider.entries.associateWith { SettingsStore.translationConfiguration(context, it) }
        val originalProvider = SettingsStore.provider(context)
        val originalEnabled = SettingsStore.useChatMockTranslation(context)
        val originalQwenModel = SettingsStore.qwenMtModel(context)
        ChatMockLoopbackFixture().use { server ->
            stage = "connection_chatmock_progress"
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
            stage = "connection_chatmock_success"
            val elapsed = assertTimedResult(editor, R.string.translation_check_success)
            check(elapsed >= 200 && elapsed <= SystemClock.elapsedRealtime() - startedAt + 100) {
                "Successful check duration does not measure the real held HTTP request"
            }
            capture("chatmock-check-success-$theme")

            stage = "connection_cancelled_stale_response"
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

            stage = "connection_chatmock_auth"
            onUi { field<TextInputEditText>(editor, "translation-model").setText("synthetic-auth") }
            click(editor, "translation-check")
            server.awaitRequest(2)
            server.releaseResponse(2)
            awaitCheckResult(editor)
            assertTimedResult(editor, R.string.translation_check_auth)
            capture("chatmock-check-auth-$theme")
            server.awaitResponse(2)
            stage = "connection_deeplx_success"
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
            stage = "connection_deeplx_quota"
            click(editor, "translation-check")
            server.awaitRequest(4); server.releaseResponse(4)
            awaitCheckResult(editor)
            assertTimedResult(editor, R.string.translation_check_busy)
            capture("translation-deeplx-check-quota-$theme")
            server.awaitResponse(4)
            stage = "connection_deeplx_rate_limit"
            click(editor, "translation-check")
            server.awaitRequest(5); server.releaseResponse(5)
            awaitCheckResult(editor)
            assertTimedResult(editor, R.string.translation_check_busy)
            capture("translation-deeplx-check-rate-limit-$theme")
            server.awaitResponse(5)
            server.assertHealthy()
            onUi { editor.finish() }
            instrumentation.waitForIdleSync()
        }
        stage = "connection_settings_unchanged"
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
            stage = "$stage:expected=${context.resources.getResourceEntryName(messageId)}:actual=${connectionOutcome(text, elapsed)}"
            check(text == context.getString(messageId, elapsed)) { "Connection result did not match the expected outcome" }
            check(status.isShown && status.textSize / status.resources.displayMetrics.scaledDensity >= 14f)
        }
        return elapsed
    }

    private fun checkForms() {
        val originalQwenModel = SettingsStore.qwenMtModel(context)
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
            check(SettingsStore.qwenMtModel(context) == originalQwenModel) { "Unsaved Qwen model changed stored preferences" }
            check(!MimiService.isRunning) { "Settings started an audio session" }
        }

        val editor = open(ServiceProvider.DASHSCOPE)
        val mode = field<Spinner>(editor, "translation-mode")
        val speechKey = field<TextInputEditText>(editor, "credential-apiKey")
        val translationKey = field<TextInputEditText>(editor, "translation-key")
        stage = "forms_default_translation_choices"
        onUi {
            check(mode.selectedItemPosition == 0 && mode.count == 7) { "Built-in translation must remain the default among seven text choices" }
            check((0 until mode.count).map { mode.getItemAtPosition(it).toString() } ==
                listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE, TextTranslationProvider.XAI,
                    TextTranslationProvider.DEEPL, TextTranslationProvider.DEEPLX,
                    TextTranslationProvider.OPENAI_COMPATIBLE, TextTranslationProvider.CHAT_MOCK)
                    .map { editor.getString(translationProviderLabel(it)) }) { "Text services are not in the expected order" }
            check(!translationKey.isShown) { "Custom fields must follow the selected translation mode" }
            val qwen = field<Spinner>(editor, "qwen-mt-model")
            stage = "forms_builtin_qwen_visible"
            check(qwen.isShown && qwen.count == 3)
            for (legacy in listOf("credential-baseUrl", "credential-model", "credential-hotwords")) {
                check(editor.window.decorView.findViewWithTag<View>(legacy) == null) { "Unused legacy speech setting is editable" }
            }
            check((0 until qwen.count).map { qwen.getItemAtPosition(it).toString() } == listOf("Qwen-MT Lite", "Qwen-MT Flash", "Qwen-MT Plus"))
            qwen.setSelection((qwen.selectedItemPosition + 1) % qwen.count)
            checkSecretField(speechKey)
            checkSecretField(translationKey)
            check(speechKey !== translationKey) { "Recognition and translation share a credential input" }
        }
        capture("chatmock-builtin-$theme")
        stage = "forms_translation_provider_menu"
        onUi { check(mode.performClick()) }
        instrumentation.waitForIdleSync()
        capture("translation-provider-menu-$theme")
        instrumentation.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        instrumentation.waitForIdleSync()
        onUi { selectMode(editor, TextTranslationProvider.CHAT_MOCK) }
        instrumentation.waitForIdleSync()
        unchanged()
        stage = "forms_chatmock_fields"
        onUi {
            check(translationKey.isShown) { "ChatMock selection did not reveal its fields" }
            check(!field<View>(editor, "qwen-mt-model").isShown)
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
        stage = "forms_provider_drafts"
        checkProviderDrafts(editor)
        stage = "forms_compatible_presets"
        checkCompatiblePresets(editor)

        stage = "forms_chatmock_help"
        onUi { check(editor.window.decorView.findViewWithTag<View>("translation-help").performLongClick()) }
        instrumentation.waitForIdleSync()
        onUi {
            val dialog = WindowInspector.getGlobalWindowViews().firstOrNull {
                it.findViewWithTag<TextView>("help-message")?.isShown == true
            } ?: error("Requirements must open a standard help dialog")
            val message = dialog.findViewWithTag<TextView>("help-message")
            check(message.text.contains(context.getString(R.string.translation_help_chatmock))) { "Long press must use the selected provider requirements" }
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
        stage = "forms_empty_translation_validation"
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
        stage = "forms_local_http_consent"
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
        stage = "forms_reopened_drafts_discarded"
        onUi {
            check(field<Spinner>(reopened, "translation-mode").selectedItemPosition == 0) { "Unsaved mode survived reopening" }
            check(field<Spinner>(reopened, "qwen-mt-model").selectedItem.toString() ==
                "Qwen-MT " + originalQwenModel.replaceFirstChar { it.uppercase() }) { "Unsaved Qwen model survived reopening" }
            checkSecretField(field(reopened, "credential-apiKey"))
            checkSecretField(field(reopened, "translation-key"))
            check(field<TextInputEditText>(reopened, "translation-endpoint").text.toString() == originalTranslation.endpoint) { "Unsaved endpoint survived reopening" }
            check(!field<CheckBox>(reopened, "translation-local-http").isChecked) { "Unsaved HTTP consent survived reopening" }
            reopened.finish()
        }
        instrumentation.waitForIdleSync()
        val openAI = open(ServiceProvider.OPENAI)
        stage = "forms_openai_simultaneous_translation_hidden"
        onUi {
            check(field<View>(openAI, "speech-setup-key").isShown) { "OpenAI has no credential setup action" }
            check(field<View>(openAI, "speech-setup-key").contentDescription.toString().contains("OpenAI"))
            check(field<Spinner>(openAI, "speech-translation-mode").selectedItemPosition == 0)
            check(!field<View>(openAI, "translation-mode").isShown) { "Integrated speech translation exposes an independent text route" }
            check(openAI.window.decorView.findViewWithTag<View>("credential-baseUrl") == null)
            check(openAI.window.decorView.findViewWithTag<View>("credential-model") == null)
            field<Spinner>(openAI, "speech-translation-mode").setSelection(1)
        }
        instrumentation.waitForIdleSync()
        stage = "forms_openai_asr_mt_translation_scope"
        onUi {
            val textMode = field<Spinner>(openAI, "translation-mode")
            check(textMode.isShown) { "ASR+MT hides its independent text route" }
            check((0 until textMode.count).any { textMode.getItemAtPosition(it).toString() == "ChatMock" })
            check(openAI.window.decorView.findViewWithTag<View>("credential-baseUrl") == null)
            check(openAI.window.decorView.findViewWithTag<View>("credential-model") == null)
            openAI.finish()
        }
        unchanged()
    }

    /** Opt-in fixture: only synthetic keys on an already-verified blank emulator. */
    private fun checkSavedCredentials() = withTextTranslationSettingsSnapshot(context) {
        try {
            stage = "saved_chatmock"
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
            stage = "saved_chatmock_reopened"
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
            stage = "saved_compatible_isolation"
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
            stage = "saved_deepl_isolation"
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
            stage = "saved_deeplx_isolation"
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
            stage = "saved_changed_endpoint_forgets_key"
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
            val appearance = (instrumentation.startActivitySync(Intent(context, MainActivity::class.java)
                .putExtra("settings_section", "appearance").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity).also { activities += it }
            instrumentation.waitForIdleSync()
            stage = "saved_original_only_appearance"
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
                val home = appearance
                stage = "saved_original_only_home"
                onUi { home.findViewById<View>(R.id.tab_caption).performClick() }
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
            stage = "forms_route_${provider.name}"
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

    private fun checkCompatiblePresets(editor: ServiceSettingsActivity) {
        onUi { selectMode(editor, TextTranslationProvider.OPENAI_COMPATIBLE) }
        instrumentation.waitForIdleSync()
        val picker = field<Spinner>(editor, "translation-preset")
        val endpoint = field<TextInputEditText>(editor, "translation-endpoint")
        val model = field<TextInputEditText>(editor, "translation-model")
        val key = field<TextInputEditText>(editor, "translation-key")
        val localHttp = field<CheckBox>(editor, "translation-local-http")
        stage = "forms_preset_setup_links"
        checkSetupLinks(editor, picker, endpoint, model, key)
        stage = "forms_preset_icons"
        checkPresetIcons(editor, picker)
        stage = "forms_preset_draft_selection"
        onUi {
            check(picker.isShown && picker.count == TextTranslationPresets.entries.size + 1)
            localHttp.isChecked = false
            key.setText("synthetic-preset-old-key")
            picker.setSelection(TextTranslationPresets.entries.indexOfFirst { it.id == "openai" } + 1)
        }
        instrumentation.waitForIdleSync()
        onUi {
            val openai = TextTranslationPresets.entries.first { it.id == "openai" }
            check(endpoint.text.toString() == openai.endpoint && model.text.toString() == openai.model)
            check(key.text.isNullOrBlank()) { "A preset moved a typed key to another destination" }
            key.setText("synthetic-preset-same-key")
            picker.setSelection(0)
        }
        instrumentation.waitForIdleSync()
        onUi {
            check(key.text.toString() == "synthetic-preset-same-key")
            val customEndpoint = endpoint.text.toString()
            model.setText("synthetic-custom-model")
            key.setText("synthetic-custom-key")
            check(picker.selectedItemPosition == 0) { "Editing custom fields changed the selected preset" }
            check(endpoint.text.toString() == customEndpoint && model.text.toString() == "synthetic-custom-model")
            picker.setSelection(TextTranslationPresets.entries.indexOfFirst { it.id == "openai" } + 1)
        }
        instrumentation.waitForIdleSync()
        onUi {
            check(key.text.toString() == "synthetic-custom-key") { "An equivalent preset discarded a typed key" }
            picker.setSelection(TextTranslationPresets.entries.indexOfFirst { it.id == "ollama" } + 1)
        }
        instrumentation.waitForIdleSync()
        onUi {
            check(endpoint.text.toString() == TextTranslationPresets.entries.first { it.id == "ollama" }.endpoint)
            check(key.text.isNullOrBlank())
            check(!localHttp.isChecked) { "A local preset silently enabled HTTP" }
            endpoint.setText("https://custom.example.invalid/v1")
        }
        instrumentation.waitForIdleSync()
        onUi {
            check(picker.selectedItemPosition == 0)
            selectMode(editor, TextTranslationProvider.CHAT_MOCK)
        }
        instrumentation.waitForIdleSync()
        check(!field<View>(editor, "translation-preset").isShown)
    }

    private fun checkPresetIcons(editor: ServiceSettingsActivity, picker: Spinner) {
        onUi {
            val adapter = picker.adapter
            val size = (24 * picker.resources.displayMetrics.density + 0.5f).toInt()
            val pictures = mutableListOf<IntArray>()
            fun picture(row: TextView, arrow: Boolean): Bitmap {
                val icon = checkNotNull(row.compoundDrawablesRelative[0]) { "A quick-setup choice has no provider icon" }
                check(icon.bounds.width() == size && icon.bounds.height() == size) { "Quick-setup icon dimensions differ" }
                check((row.compoundDrawablesRelative[2] != null) == arrow) { "A recycled quick-setup row kept the wrong arrow" }
                return Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888).also { icon.draw(Canvas(it)) }
            }
            var selected: View? = null
            var dropdown: View? = null
            for (position in 0 until picker.count) {
                val fresh = adapter.getView(position, null, picker) as TextView
                val expected = picture(fresh, arrow = true)
                selected = adapter.getView(position, selected, picker)
                dropdown = adapter.getDropDownView(position, dropdown, picker)
                for ((view, arrow) in listOf(selected to true, dropdown to false)) {
                    val row = view as TextView
                    check(row.text.toString() == adapter.getItem(position).toString()) { "A recycled quick-setup row kept another label" }
                    val actual = picture(row, arrow)
                    check(expected.sameAs(actual)) { "A recycled quick-setup row kept another provider icon" }
                    actual.recycle()
                }
                val pixels = IntArray(size * size).also { expected.getPixels(it, 0, size, 0, 0, size, size) }
                check(pixels.any { it ushr 24 != 0 }) { "A quick-setup icon is empty" }
                check(pictures.none { it.contentEquals(pixels) }) { "Different quick-setup providers share the same fallback icon" }
                pictures += pixels
                expected.recycle()
            }

            // Both the field and menu must wrap a real long provider label at
            // narrow widths, rather than clipping it behind a new icon slot.
            val longest = (0 until picker.count).maxBy { adapter.getItem(it).toString().length }
            val width = (180 * picker.resources.displayMetrics.density + 0.5f).toInt()
            for (row in listOf(adapter.getView(longest, null, picker), adapter.getDropDownView(longest, null, picker))) {
                row.measure(View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                    View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED))
                row.layout(0, 0, row.measuredWidth, row.measuredHeight)
                val label = row as TextView
                val layout = checkNotNull(label.layout)
                check(layout.lineCount > 1) { "A long quick-setup provider label did not wrap" }
                check(layout.getLineEnd(layout.lineCount - 1) == label.text.length &&
                    (0 until layout.lineCount).all { layout.getEllipsisCount(it) == 0 }) { "A quick-setup provider label was truncated" }
                check(row.measuredHeight >= layout.getLineBottom(layout.lineCount - 1) + label.compoundPaddingTop + label.compoundPaddingBottom) {
                    "A wrapped quick-setup provider label was vertically clipped"
                }
            }
        }
        show(editor, "translation-preset")
        // Earlier synthetic saves may leave a system Toast outside this app's
        // windows. Let the short Toast expire before either menu capture.
        SystemClock.sleep(2_500)
        val selectedPreset = picker.selectedItemPosition
        onUi { check(picker.performClick()) }
        instrumentation.waitForIdleSync()
        fun findList(view: View): ListView? {
            if (view is ListView) return view
            if (view !is ViewGroup) return null
            for (index in 0 until view.childCount) findList(view.getChildAt(index))?.let { return it }
            return null
        }
        lateinit var popup: ListView
        onUi {
            popup = WindowInspector.getGlobalWindowViews().mapNotNull(::findList)
                .single { it.isShown && it.count == picker.count }
            popup.setSelection(0)
        }
        instrumentation.waitForIdleSync()
        capture("translation-preset-icons-$theme")
        onUi { popup.setSelection(popup.count - 1) }
        instrumentation.waitForIdleSync()
        capture("translation-preset-icons-bottom-$theme")
        onUi { check(picker.selectedItemPosition == selectedPreset) { "Scrolling the quick-setup menu committed a preset" } }
        instrumentation.sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        instrumentation.waitForIdleSync()
    }

    private fun checkSetupLinks(editor: ServiceSettingsActivity, picker: Spinner,
        endpoint: TextInputEditText, model: TextInputEditText, key: TextInputEditText) {
        fun keyBox() = generateSequence(key.parent) { it.parent }.filterIsInstance<TextInputLayout>().first()
        val optionalHint = editor.getString(R.string.translation_key_optional)
        val opened = java.util.concurrent.CopyOnWriteArrayList<Intent>()
        val monitor = object : Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: Intent): Instrumentation.ActivityResult? {
                if (intent.action != Intent.ACTION_VIEW) return null
                opened += Intent(intent)
                return Instrumentation.ActivityResult(Activity.RESULT_CANCELED, null)
            }
        }
        instrumentation.addMonitor(monitor)
        try {
            onUi { check(field<View>(editor, "speech-setup-key").performClick()) }
            instrumentation.waitForIdleSync()
            check(opened.single().dataString == serviceSetupUrl(ServiceProvider.DASHSCOPE.wireProvider, TextTranslationSetupPage.API_KEY))
            opened.clear()
            val openai = TextTranslationPresets.entries.first { it.id == "openai" }
            val kimi = TextTranslationPresets.entries.first { it.id == "kimi" }
            val index = TextTranslationPresets.entries.first { it.id == "indexTranslate" }
            val local = TextTranslationPresets.entries.first { it.id == "ollama" }
            onUi { picker.setSelection(TextTranslationPresets.entries.indexOf(openai) + 1) }
            instrumentation.waitForIdleSync()
            check(opened.isEmpty()) { "Selecting a preset opened an external page" }
            onUi {
                check(field<View>(editor, "translation-setup-key").isShown)
                check(field<View>(editor, "translation-setup-guide").isShown)
                check(!field<View>(editor, "translation-setup-no-key").isShown)
                check(keyBox().placeholderText?.toString() != optionalHint) { "A cloud preset described its API key as optional" }
                check(field<View>(editor, "translation-setup-key").contentDescription.toString().contains(openai.label))
                // The action must resolve a pending Spinner choice before opening.
                picker.setSelection(TextTranslationPresets.entries.indexOf(kimi) + 1)
                check(field<View>(editor, "translation-setup-key").performClick())
            }
            instrumentation.waitForIdleSync()
            check(opened.single().dataString == kimi.apiKeyUrl) { "A setup action opened the previously selected service" }
            onUi {
                model.setText("synthetic-setup-model")
                key.setText("synthetic-setup-key")
                check(field<View>(editor, "translation-setup-guide").performClick())
            }
            instrumentation.waitForIdleSync()
            check(opened.last().dataString == kimi.guideUrl)
            onUi {
                check(endpoint.text.toString() == kimi.endpoint && model.text.toString() == "synthetic-setup-model")
                check(key.text.toString() == "synthetic-setup-key") { "Opening setup changed an unsaved key" }
                picker.setSelection(TextTranslationPresets.entries.indexOf(index) + 1)
            }
            instrumentation.waitForIdleSync()
            check(opened.size == 2) { "Selecting a keyless preset opened a page" }
            onUi {
                check(!field<View>(editor, "translation-setup-key").isShown)
                check(field<View>(editor, "translation-setup-no-key").isShown)
                check(key.isEnabled) { "Keyless guidance disabled the optional credential field" }
                picker.setSelection(TextTranslationPresets.entries.indexOf(local) + 1)
            }
            instrumentation.waitForIdleSync()
            onUi {
                check(!field<View>(editor, "translation-setup-key").isShown)
                check(field<TextView>(editor, "translation-setup-guide").text.toString() == editor.getString(R.string.translation_setup_local_guide))
                check(keyBox().placeholderText?.toString() == optionalHint) { "A local preset lost its optional-key hint" }
                check(field<View>(editor, "translation-setup-guide").performClick())
            }
            instrumentation.waitForIdleSync()
            check(opened.last().dataString == local.guideUrl)
            check(opened.size == 3)
            onUi {
                selectMode(editor, TextTranslationProvider.CHAT_MOCK)
                field<View>(editor, "translation-setup-guide").performClick()
            }
            instrumentation.waitForIdleSync()
            check(opened.size == 3) { "A stale setup action opened after the translation route changed" }
            for (provider in listOf(TextTranslationProvider.XAI, TextTranslationProvider.DEEPL)) {
                onUi { selectMode(editor, provider) }
                instrumentation.waitForIdleSync()
                val before = opened.size
                onUi {
                    key.setText("synthetic-unsaved-dedicated-key")
                    val setup = field<View>(editor, "translation-service-setup-key")
                    check(setup.isShown) { "A dedicated text service has no credential setup action" }
                    check(setup.performClick())
                    check(key.text.toString() == "synthetic-unsaved-dedicated-key") { "Opening credential setup changed an unsaved key" }
                }
                instrumentation.waitForIdleSync()
                check(opened.size == before + 1)
                check(opened.last().dataString == serviceSetupUrl(requireNotNull(textTranslationServiceSetupId(provider)), TextTranslationSetupPage.API_KEY))
            }
            val beforeCustom = opened.size
            onUi { selectMode(editor, TextTranslationProvider.CHAT_MOCK) }
            instrumentation.waitForIdleSync()
            onUi {
                check(!field<View>(editor, "translation-service-setup-key").isShown)
                field<View>(editor, "translation-service-setup-key").performClick()
            }
            instrumentation.waitForIdleSync()
            check(opened.size == beforeCustom) { "A custom text route opened a dedicated service's key page" }
            onUi { selectMode(editor, TextTranslationProvider.OPENAI_COMPATIBLE) }
            instrumentation.waitForIdleSync()
            onUi { picker.setSelection(0) }
            instrumentation.waitForIdleSync()
            onUi {
                check(!field<View>(editor, "translation-setup-actions").isShown)
                check(!field<View>(editor, "translation-setup-no-key").isShown)
                check(keyBox().placeholderText?.toString() == optionalHint) { "Custom did not restore its optional-key hint" }
            }
        } finally {
            instrumentation.removeMonitor(monitor)
        }
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

    /** Match fixed UI resources only; never include service responses or fields. */
    private fun connectionOutcome(text: String, elapsed: Long): String = listOf(
        R.string.translation_check_success, R.string.translation_check_auth,
        R.string.translation_check_not_found, R.string.translation_check_busy,
        R.string.translation_check_timeout, R.string.translation_check_response,
        R.string.translation_check_failure,
    ).firstOrNull { text == context.getString(it, elapsed) }
        ?.let(context.resources::getResourceEntryName) ?: "unknown_fixed_label"

    private fun failureSummary(error: Throwable): String {
        val frame = error.stackTrace.firstOrNull { it.className.startsWith("app.yuxino.mimi.android.") }
        return "$stage:${error.javaClass.simpleName}@${frame?.fileName ?: "unknown"}:${frame?.lineNumber ?: -1}"
    }

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

    private fun open(provider: ServiceProvider): ServiceSettingsActivity {
        val profile = SettingsStore.activeProfile(context)?.takeIf { it.provider == provider }
            ?: SettingsStore.profiles(context).firstOrNull { it.provider == provider }
        val intent = Intent(context, ServiceSettingsActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        if (profile != null) intent.putExtra("profileId", profile.id) else intent.putExtra("provider", provider.id)
        return (instrumentation.startActivitySync(intent) as ServiceSettingsActivity).also {
            activities += it
            // These fixtures exercise independent text translation explicitly; new Alibaba
            // profiles default to simultaneous translation in the ordinary product flow.
            if (provider == ServiceProvider.DASHSCOPE) onUi { field<Spinner>(it, "alibaba-mode").setSelection(0) }
            instrumentation.waitForIdleSync()
        }
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
