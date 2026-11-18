package app.yuxino.mimi.android.services.translation

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.widget.TextView
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.services.acceptance.ChatMockLoopbackFixture
import app.yuxino.mimi.android.services.editor.ServiceSettingsActivity
import app.yuxino.mimi.android.settings.SettingsStore
import com.google.android.material.textfield.TextInputEditText

/** One held loopback request in a new synthetic profile; existing Dev data is restored exactly. */
internal class TextCheckStateChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private var stage = "preconditions"

    fun run() {
        var failure: String? = null
        var snapshot: ProfileSettingsSnapshot? = null
        var editor: ServiceSettingsActivity? = null
        var server: ChatMockLoopbackFixture? = null
        try {
            check(context.packageName.endsWith(".dev") &&
                context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!MimiService.isStarting && !MimiService.isRunning && !MimiService.isStopping)
            snapshot = ProfileSettingsSnapshot(context)
            SettingsStore.setSourceLang(context, "en")
            SettingsStore.setTargetLang(context, "zh")
            server = ChatMockLoopbackFixture()
            stage = "synthetic_profile"
            val name = "Synthetic text check state fixture"
            val id = checkNotNull(SettingsStore.saveProfile(context, null, name,
                ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-unused-speech-key")),
                TranslationConfiguration(endpoint = server.baseUrl, model = "synthetic-success", apiKey = "",
                    allowLocalHttp = true, provider = TextTranslationProvider.CHAT_MOCK),
                translationEnabled = true, activate = false))
            check(SettingsStore.flushPendingWritesForTests(context))
            val activity = test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                .putExtra("profileId", id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity
            editor = activity
            test.waitForIdleSync()
            stage = "pending_request"
            onUi {
                check(field<TextInputEditText>(activity, "translation-model").text.toString() == "synthetic-success")
                check(field<TextInputEditText>(activity, "translation-key").text.isNullOrBlank())
                check(field<View>(activity, "translation-check").performClick())
            }
            server.awaitRequest(0)
            stage = "idle_refresh_during_request"
            onUi {
                val action = field<TextView>(activity, "translation-check")
                check(!action.isEnabled && action.text.toString() == activity.getString(R.string.translation_checking))
                ServiceSettingsActivity::class.java.getDeclaredMethod("refreshSessionUi").run {
                    isAccessible = true; invoke(activity)
                }
                check(!action.isEnabled && action.text.toString() == activity.getString(R.string.translation_checking))
                action.performClick()
                check(!action.isEnabled)
                check(field<View>(activity, "translation-status").visibility == View.GONE)
            }
            server.releaseResponse(0)
            awaitResult(activity)
            server.awaitResponse(0)
            server.assertHealthy()
            stage = "request_completed"
            onUi {
                val status = field<TextView>(activity, "translation-status")
                val elapsed = Regex("([0-9]+)\\s*ms").find(status.text.toString())
                    ?.groupValues?.get(1)?.toLongOrNull() ?: error("missing_duration")
                check(status.text.toString() == activity.getString(R.string.translation_check_success, elapsed))
                check(field<View>(activity, "translation-check").isEnabled)
            }
            stage = "disabled_refresh_and_dispose"
            onUi {
                val settings = ServiceSettingsActivity::class.java.getDeclaredField("translationSettings").run {
                    isAccessible = true; get(activity) as TextTranslationSettings
                }
                settings.setEditingEnabled(false)
                field<TextInputEditText>(activity, "translation-model").setText("synthetic-disabled-edit")
                settings.dispose()
                val action = field<View>(activity, "translation-check")
                check(!action.isEnabled)
                action.performClick()
                check(field<View>(activity, "translation-status").visibility == View.GONE)
                settings.setEditingEnabled(true)
                check(action.isEnabled)
            }
            check(!MimiService.isStarting && !MimiService.isRunning && !MimiService.isStopping)
        } catch (_: Throwable) {
            failure = stage
        } finally {
            try {
                onUi { editor?.takeUnless { it.isFinishing }?.finish() }
                test.waitForIdleSync()
            } catch (_: Throwable) { failure = "close_editor" }
            try { server?.close() } catch (_: Throwable) { failure = "close_loopback" }
            try { snapshot?.close() } catch (_: Throwable) { failure = "restore" }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null)
                "Text check state UI passed: idle refresh and stale click preserve one pending loopback request; completion restores the action; disabled draft refresh/disposal stay disabled; exact profile/preferences snapshot restored; no external provider or audio capture.\n"
            else "Text check state UI failed: stage=$failure; no credential values logged.\n")
        })
    }

    private fun awaitResult(activity: Activity) {
        val deadline = SystemClock.elapsedRealtime() + 8_000
        do {
            var ready = false
            onUi {
                ready = field<View>(activity, "translation-check").isEnabled &&
                    field<View>(activity, "translation-status").visibility == View.VISIBLE
            }
            if (ready) return
            SystemClock.sleep(50)
        } while (SystemClock.elapsedRealtime() < deadline)
        error("check_result_timeout")
    }

    private inline fun <reified T : View> field(activity: Activity, tag: String): T =
        checkNotNull(activity.findViewById<View>(android.R.id.content).findViewWithTag<T>(tag))
    private fun onUi(action: () -> Unit) = test.runOnMainSync(action)
}
