package app.yuxino.mimi.android

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
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceProvider
import com.google.android.material.textfield.TextInputEditText
import java.io.File
import java.security.MessageDigest

/** Unsaved synthetic editors only; never saves a profile, opens a provider or captures audio. */
internal class ServiceDraftRecreationChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    @Volatile private var current: ServiceSettingsActivity? = null
    private var stage = "preconditions"

    fun run() {
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
                field<Spinner>(first, "translation-mode").setSelection(4)
            }
            test.waitForIdleSync()
            onUi {
                field<TextInputEditText>(first, "translation-endpoint").setText("http://127.0.0.1:8765/v1")
                field<TextInputEditText>(first, "translation-model").setText("synthetic-draft-model")
                field<TextInputEditText>(first, "translation-key").setText("synthetic-unsaved-text-key")
                field<CheckBox>(first, "translation-local-http").isChecked = true
                field<Spinner>(first, "translation-mode").setSelection(0)
                field<Spinner>(first, "qwen-mt-model").setSelection(2)
                field<Spinner>(first, "alibaba-mode").setSelection(1)
                first.recreate()
            }
            stage = "await_recreation"
            val next = awaitRecreated(first)
            stage = "credentials_not_serialized"
            check(!secretInSavedState)
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
                check(field<Spinner>(next, "translation-mode").selectedItemPosition == 0)
                field<Spinner>(next, "translation-mode").setSelection(4)
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
        } catch (error: Throwable) {
            failure = "$stage:${error.javaClass.simpleName}"
        } finally {
            onUi { activities.forEach { if (!it.isFinishing && !it.isDestroyed) it.finish() } }
            application.unregisterActivityLifecycleCallbacks(callbacks)
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "Service draft recreation passed=${failure == null}; failure=${failure ?: "none"}; unsaved synthetic fields only, no profile writes, network or capture.\n")
        })
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
    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
}
