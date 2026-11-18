package app.yuxino.mimi.android.acceptance

import android.app.Activity
import android.app.Instrumentation
import android.os.Bundle
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.services.acceptance.TemporaryProviderAcceptance
import app.yuxino.mimi.android.settings.SettingsStore
import java.io.File
import org.json.JSONObject

/** Actual Android JSON failures must not echo private acceptance input. */
internal class PrivateAcceptanceInputChecks(private val test: Instrumentation) {
    fun run() {
        val context = test.targetContext
        val input = File(context.cacheDir, "acceptance-provider.json")
        var failure: Throwable? = null
        try {
            check(!MimiService.isRunning && !input.exists())
            val before = listOf(SettingsStore.provider(context), SettingsStore.sourceLang(context), SettingsStore.targetLang(context))
            val marker = "synthetic-private-input-never-log"
            val provider = ServiceProvider.GEMINI
            for (fields in listOf(marker, JSONObject().put(provider.fields.single().id, JSONObject().put("private", marker)))) {
                input.writeText(JSONObject().put("provider", provider.id).put("source", "auto").put("target", "zh")
                    .put("credentials", fields).toString())
                val error = runCatching { TemporaryProviderAcceptance(context).apply() }.exceptionOrNull()
                check(error is IllegalStateException && error.message == "invalid_acceptance_provider_input" && error.cause == null)
                check(!input.exists())
                check(before == listOf(SettingsStore.provider(context), SettingsStore.sourceLang(context), SettingsStore.targetLang(context)))
            }
        } catch (error: Throwable) { failure = error }
        finally { check(!input.exists() || input.delete()) }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null) "Private acceptance input passed: malformed containers/fields rejected without echo, state changes, capture or providers.\n"
                else "Private acceptance input failed: ${failure!!.stackTraceToString()}\n")
        })
    }
}
