package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.pm.ApplicationInfo
import android.os.Bundle
import app.yuxino.mimi.android.capture.MimiService
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest

/** Explicit local acceptance only: compare saved credentials without changing a profile. */
internal class CredentialMatchChecks(private val test: Instrumentation) {
    fun run() {
        val context = test.targetContext
        val input = File(context.cacheDir, "acceptance-credentials.json")
        var stage = "preconditions"
        var failure: String? = null
        var provider = ""
        try {
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!MimiService.isRunning)
            stage = "private_input"
            check(input.isFile && input.length() in 1..16_384)
            val expected = JSONObject(input.readText(Charsets.UTF_8))
            val configuration = SettingsStore.configuration(context)
            provider = configuration.provider.wireProvider
            stage = "provider_mismatch"
            check(provider == expected.getString("provider"))
            check(configuration.provider.configured(configuration.credentials))
            val credentials = expected.getJSONObject("credentials")
            stage = "credential_mismatch"
            check(credentials.length() == configuration.provider.fields.size)
            for (field in configuration.provider.fields) {
                val saved = configuration.value(field.id).toByteArray(Charsets.UTF_8)
                val supplied = credentials.getString(field.id).toByteArray(Charsets.UTF_8)
                try { check(MessageDigest.isEqual(saved, supplied)) }
                finally { saved.fill(0); supplied.fill(0) }
            }
        } catch (_: Exception) {
            failure = stage
        } finally {
            if (input.exists() && !input.delete()) failure = "private_input_cleanup"
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null)
                "Credential comparison passed: provider=$provider; model=${SettingsStore.qwenMtModel(context)}; source=${SettingsStore.sourceLang(context)}; target=${SettingsStore.targetLang(context)}; history=${SettingsStore.historyLines(context)}; saved profile unchanged; private comparison input removed.\n"
            else "Credential comparison failed: $failure; no credential values logged.\n")
        })
    }
}
