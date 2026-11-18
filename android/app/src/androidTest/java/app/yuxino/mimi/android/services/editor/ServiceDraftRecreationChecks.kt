package app.yuxino.mimi.android.services.editor

import android.app.Activity
import android.app.Application
import android.app.Instrumentation
import android.content.Intent
import android.os.Bundle
import android.os.Parcel
import android.os.SystemClock
import android.view.View
import android.widget.CheckBox
import android.widget.Spinner
import android.widget.TextView
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.services.setup.TextTranslationPresets
import app.yuxino.mimi.android.services.translation.translationProviderLabel
import app.yuxino.mimi.android.settings.SettingsStore
import com.google.android.material.textfield.TextInputEditText
import java.io.File
import java.security.MessageDigest

/** Unsaved synthetic editors only; never saves a profile, opens a provider or captures audio. */
internal class ServiceDraftRecreationChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    @Volatile private var current: ServiceSettingsActivity? = null
    private var stage = "preconditions"

    fun run(arguments: Bundle?) {
        val focusedCase = arguments?.getString("translation_draft_case")
        val application = context.applicationContext as Application
        val activities = mutableListOf<ServiceSettingsActivity>()
        var secretInSavedState = false
        val callbacks = object : Application.ActivityLifecycleCallbacks {
            override fun onActivityCreated(activity: Activity, state: Bundle?) {
                if (activity is ServiceSettingsActivity) { current = activity; activities += activity }
            }
            override fun onActivityStarted(activity: Activity) = Unit
            override fun onActivityResumed(activity: Activity) = Unit
            override fun onActivityPaused(activity: Activity) = Unit
            override fun onActivityStopped(activity: Activity) = Unit
            override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) {
                if (activity is ServiceSettingsActivity) {
                    secretInSavedState = secretInSavedState || serializedStateContainsSecret(state)
                }
            }
            override fun onActivityDestroyed(activity: Activity) = Unit
        }
        var failure: String? = null
        try {
            check(!MimiService.isRunning && !MimiService.isStopping)
            check(serializedStateContainsSecret(Bundle().apply {
                putString("synthetic-positive-control", "synthetic-unsaved-text-key")
            }))
            application.registerActivityLifecycleCallbacks(callbacks)
            stage = "launch_unsaved_editor"
            // SettingsStore.profiles can trigger a legacy credential migration.
            // Hash metadata files directly; this check must not initialize it.
            val before = catalogSnapshot()
            when (focusedCase) {
                "custom_recreation" -> checkCustomRecreation()
                "pending_save" -> checkPendingSave()
                "pending_check" -> checkPendingCheck()
                null -> checkExistingRecreation { secretInSavedState }
                else -> error("unsupported_translation_draft_case")
            }
            stage = "credentials_not_serialized"
            check(!secretInSavedState)
            stage = "focused_case_does_not_save"
            check(catalogSnapshot() == before)
        } catch (error: Throwable) {
            val frame = error.stackTrace.firstOrNull { it.className.startsWith("app.yuxino.mimi.android.") }
            failure = "$stage:${error.javaClass.simpleName}@${frame?.fileName ?: "unknown"}:${frame?.lineNumber ?: -1}"
        } finally {
            onUi { activities.forEach { if (!it.isFinishing && !it.isDestroyed) it.finish() } }
            application.unregisterActivityLifecycleCallbacks(callbacks)
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "Service draft recreation case=${focusedCase ?: "all_original"} passed=${failure == null}; failure=${failure ?: "none"}; unsaved synthetic fields only, no profile writes, network or capture.\n")
        })
    }

    private fun checkExistingRecreation(secretInSavedState: () -> Boolean) {
            val before = catalogSnapshot()
            test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                .putExtra("provider", ServiceProvider.DASHSCOPE.id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            test.waitForIdleSync()
            val first = requireNotNull(current)
            stage = "edit_drafts"
            onUi {
                field<TextInputEditText>(first, "profile-name").setText("Synthetic unsaved name")
                field<TextInputEditText>(first, "credential-apiKey").setText("synthetic-unsaved-speech-key")
                field<TextInputEditText>(first, "credential-alibaba-workspace").setText("synthetic-workspace")
                field<Spinner>(first, "alibaba-mode").setSelection(0)
                selectTranslationProvider(first, TextTranslationProvider.OPENAI_COMPATIBLE)
            }
            test.waitForIdleSync()
            onUi {
                field<TextInputEditText>(first, "translation-endpoint").setText("http://127.0.0.1:8765/v1")
                field<TextInputEditText>(first, "translation-model").setText("synthetic-draft-model")
                field<TextInputEditText>(first, "translation-key").setText("synthetic-unsaved-text-key")
                field<CheckBox>(first, "translation-local-http").isChecked = true
                selectTranslationProvider(first, TextTranslationProvider.BUILTIN)
                field<Spinner>(first, "qwen-mt-model").setSelection(2)
                field<Spinner>(first, "alibaba-mode").setSelection(1)
                first.recreate()
            }
            stage = "await_recreation"
            val next = awaitRecreated(first)
            stage = "credentials_not_serialized"
            check(!secretInSavedState())
            stage = "profile_draft_preserved"
            onUi {
                check(field<TextInputEditText>(next, "profile-name").text.toString() == "Synthetic unsaved name")
                check(field<TextInputEditText>(next, "credential-apiKey").text.toString() == "synthetic-unsaved-speech-key")
                check(field<TextInputEditText>(next, "credential-alibaba-workspace").text.toString() == "synthetic-workspace")
                check(field<Spinner>(next, "alibaba-mode").selectedItemPosition == 1)
            }
            capture(next, "profile")
            onUi { field<Spinner>(next, "alibaba-mode").setSelection(0) }
            test.waitForIdleSync()
            stage = "translation_drafts_preserved"
            onUi {
                check(field<Spinner>(next, "qwen-mt-model").selectedItemPosition == 2)
                check(field<Spinner>(next, "translation-mode").selectedItem.toString() == next.getString(translationProviderLabel(TextTranslationProvider.BUILTIN)))
                selectTranslationProvider(next, TextTranslationProvider.OPENAI_COMPATIBLE)
            }
            test.waitForIdleSync()
            onUi {
                check(field<TextInputEditText>(next, "translation-endpoint").text.toString() == "http://127.0.0.1:8765/v1")
                check(field<TextInputEditText>(next, "translation-model").text.toString() == "synthetic-draft-model")
                check(field<TextInputEditText>(next, "translation-key").text.toString() == "synthetic-unsaved-text-key")
                check(field<CheckBox>(next, "translation-local-http").isChecked)
                check(field<TextInputEditText>(next, "translation-key").transformationMethod is android.text.method.PasswordTransformationMethod)
            }
            capture(next, "translation")
            onUi {
                next.findViewById<View>(R.id.back).performClick()
            }
            test.waitForIdleSync()
            stage = "cancel_does_not_save"
            check(catalogSnapshot() == before)
            stage = "fresh_editor_has_no_old_draft"
            test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                .putExtra("provider", ServiceProvider.DASHSCOPE.id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            test.waitForIdleSync()
            val fresh = requireNotNull(current)
            onUi {
                check(field<TextInputEditText>(fresh, "profile-name").text.toString() != "Synthetic unsaved name")
                check(field<TextInputEditText>(fresh, "credential-apiKey").text.isNullOrBlank())
            }
    }

    /** A known endpoint must not undo an explicit Custom choice on recreation. */
    private fun checkCustomRecreation() {
        val first = openCompatibleEditor()
        val kimi = TextTranslationPresets.entries.first { it.id == "kimi" }
        onUi { field<Spinner>(first, "translation-preset").setSelection(TextTranslationPresets.entries.indexOf(kimi) + 1) }
        test.waitForIdleSync()
        stage = "custom_known_endpoint_applied"
        onUi {
            check(field<TextInputEditText>(first, "translation-endpoint").text.toString() == kimi.endpoint)
            field<TextInputEditText>(first, "translation-key").setText("synthetic-unsaved-text-key")
            field<Spinner>(first, "translation-preset").setSelection(0)
        }
        test.waitForIdleSync()
        onUi { assertCustom(first, kimi.endpoint, "custom_before_recreation"); first.recreate() }
        val next = awaitRecreated(first)
        onUi {
            assertCustom(next, kimi.endpoint, "custom_after_recreation")
            stage = "custom_after_recreation_key"
            check(field<TextInputEditText>(next, "translation-key").text.toString() == "synthetic-unsaved-text-key")
        }
    }

    private fun assertCustom(activity: ServiceSettingsActivity, endpoint: String, prefix: String) {
        stage = "${prefix}_selection"
        check(field<Spinner>(activity, "translation-preset").selectedItemPosition == 0)
        stage = "${prefix}_endpoint"
        check(field<TextInputEditText>(activity, "translation-endpoint").text.toString() == endpoint)
        stage = "${prefix}_setup_actions"
        check(field<View>(activity, "translation-setup-actions").visibility == View.GONE)
        stage = "${prefix}_setup_key"
        check(field<View>(activity, "translation-setup-key").visibility == View.GONE)
        stage = "${prefix}_setup_guide"
        check(field<View>(activity, "translation-setup-guide").visibility == View.GONE)
    }

    /** Save must validate the newly selected route before Spinner dispatches its callback. */
    private fun checkPendingSave() {
        val editor = openCompatibleEditor()
        onUi {
            // A missing name is the safety stop after translation validation; no
            // credential can reach SettingsStore even if a future check changes.
            field<TextInputEditText>(editor, "profile-name").setText("")
            field<TextInputEditText>(editor, "translation-endpoint").setText("https://synthetic.example.invalid/v1")
            field<TextInputEditText>(editor, "translation-model").setText("")
            field<TextInputEditText>(editor, "translation-key").setText("synthetic-unsaved-text-key")
        }
        stage = "pending_route_save"
        onUi {
            // Keep selection and click in the same UI task: yielding here would
            // let onItemSelected hide the race being exercised.
            selectTranslationProvider(editor, TextTranslationProvider.NONE)
            stage = "pending_route_save_action"
            check(editor.findViewById<View>(R.id.save).performClick())
            stage = "pending_route_save_translation_${validationLabel(field(editor, "translation-status"))}"
            check(field<TextView>(editor, "translation-status").visibility == View.GONE)
            stage = "pending_route_save_profile_name_validation"
            check(field<TextInputEditText>(editor, "profile-name").error?.toString() == editor.getString(R.string.service_required))
            check(!editor.isFinishing)
        }
        test.waitForIdleSync()
        assertCompatibleDraftRetained(editor)
    }

    /** A pending DeepL choice must report its missing key, not the old model. */
    private fun checkPendingCheck() {
        val editor = openCompatibleEditor()
        onUi {
            field<TextInputEditText>(editor, "translation-endpoint").setText("https://synthetic.example.invalid/v1")
            field<TextInputEditText>(editor, "translation-model").setText("")
            field<TextInputEditText>(editor, "translation-key").setText("synthetic-unsaved-text-key")
        }
        stage = "pending_route_check"
        onUi {
            selectTranslationProvider(editor, TextTranslationProvider.DEEPL)
            stage = "pending_route_check_action"
            check(field<View>(editor, "translation-check").performClick())
            val result = field<TextView>(editor, "translation-status")
            stage = "pending_route_check_status_visible"
            check(result.visibility == View.VISIBLE)
            stage = "pending_route_check_label_${validationLabel(result)}"
            check(result.text.toString() == editor.getString(R.string.translation_key_required))
            stage = "pending_route_check_finished_locally"
            check(field<View>(editor, "translation-check").isEnabled)
        }
        test.waitForIdleSync()
        assertCompatibleDraftRetained(editor)
    }

    private fun assertCompatibleDraftRetained(editor: ServiceSettingsActivity) {
        stage = "pending_route_retains_previous_draft"
        onUi { selectTranslationProvider(editor, TextTranslationProvider.OPENAI_COMPATIBLE) }
        test.waitForIdleSync()
        onUi {
            check(field<TextInputEditText>(editor, "translation-endpoint").text.toString() == "https://synthetic.example.invalid/v1")
            check(field<TextInputEditText>(editor, "translation-model").text.isNullOrEmpty())
            check(field<TextInputEditText>(editor, "translation-key").text.toString() == "synthetic-unsaved-text-key")
        }
    }

    private fun openCompatibleEditor(): ServiceSettingsActivity {
        stage = "launch_unsaved_compatible_editor"
        test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
            .putExtra("provider", ServiceProvider.DASHSCOPE.id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        test.waitForIdleSync()
        val editor = requireNotNull(current)
        // Fresh Alibaba editors default to integrated live translation, which
        // keeps this form GONE and suppresses its Spinner layout callbacks.
        // These cases exercise independent translation after a real mode change.
        onUi { field<Spinner>(editor, "alibaba-mode").setSelection(0) }
        test.waitForIdleSync()
        onUi { selectTranslationProvider(editor, TextTranslationProvider.OPENAI_COMPATIBLE) }
        test.waitForIdleSync()
        stage = "compatible_editor_ready"
        onUi {
            check(field<View>(editor, "translation-mode").isShown)
            check(field<View>(editor, "translation-preset").isShown)
            check(field<View>(editor, "translation-check").isEnabled)
        }
        return editor
    }

    /** Fixed labels only: diagnostics never include a field or service response. */
    private fun validationLabel(result: TextView): String = when (result.text.toString()) {
        context.getString(R.string.translation_key_required) -> "key_required"
        context.getString(R.string.translation_endpoint_required) -> "endpoint_required"
        context.getString(R.string.translation_required) -> "model_required"
        context.getString(R.string.translation_invalid) -> "invalid"
        "" -> "empty"
        else -> "other"
    }

    private fun awaitRecreated(old: ServiceSettingsActivity): ServiceSettingsActivity {
        val deadline = SystemClock.elapsedRealtime() + 5_000
        do {
            test.waitForIdleSync()
            current?.let { if (it !== old && !it.isDestroyed) return it }
            SystemClock.sleep(50)
        } while (SystemClock.elapsedRealtime() < deadline)
        error("recreation_timeout")
    }
    private inline fun <reified T : View> field(activity: Activity, tag: String): T =
        requireNotNull(activity.window.decorView.findViewWithTag<T>(tag))
    private fun serializedStateContainsSecret(state: Bundle): Boolean {
        val parcel = Parcel.obtain()
        return try {
            state.writeToParcel(parcel, 0)
            val serialized = String(parcel.marshall(), Charsets.UTF_16LE)
            listOf("synthetic-unsaved-speech-key", "synthetic-unsaved-text-key").any(serialized::contains)
        } finally { parcel.recycle() }
    }
    private fun catalogSnapshot(): List<List<Byte>?> =
        listOf("service-profiles.json", "service-profiles.json.bak", "service-profiles.json.new").map { name ->
            val file = File(context.filesDir, name)
            if (!file.exists()) null else {
                check(file.isFile)
                val digest = MessageDigest.getInstance("SHA-256")
                file.inputStream().use { input ->
                    val buffer = ByteArray(8192)
                    while (true) {
                        val count = input.read(buffer)
                        if (count < 0) break
                        digest.update(buffer, 0, count)
                    }
                }
                digest.digest().toList()
            }
        }
    private fun capture(activity: Activity, name: String) {
        val deadline = SystemClock.elapsedRealtime() + 5_000
        var visible = false
        do {
            test.waitForIdleSync()
            onUi { visible = activity.hasWindowFocus() && activity.window.decorView.isShown }
            if (visible) break
            SystemClock.sleep(50)
        } while (SystemClock.elapsedRealtime() < deadline)
        check(visible)
        // Let TextInputLayout's hint animation and the compositor present the
        // recreated fields before recording their user-visible pixels.
        SystemClock.sleep(300)
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        try {
            File(context.cacheDir, "mimi-draft-recreation-$name.png").outputStream().use {
                check(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it))
            }
        } finally { bitmap.recycle() }
    }
    private fun selectTranslationProvider(activity: Activity, provider: TextTranslationProvider) {
        val picker = field<Spinner>(activity, "translation-mode")
        val label = activity.getString(translationProviderLabel(provider))
        picker.setSelection((0 until picker.count).single { picker.getItemAtPosition(it).toString() == label })
    }
    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
}
