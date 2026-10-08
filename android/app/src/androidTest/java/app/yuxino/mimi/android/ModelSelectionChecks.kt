package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.widget.Spinner
import android.widget.TextView
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceProvider
import org.json.JSONArray
import org.json.JSONObject

/** Explicit saved-model UI selection for local acceptance; no key input or provider request. */
internal class ModelSelectionChecks(private val test: Instrumentation) {
    fun run(arguments: Bundle?) {
        val context = test.targetContext
        var stage = "preconditions"
        var failure: String? = null
        var editor: ServiceSettingsActivity? = null
        var home: MainActivity? = null
        val inventory = JSONArray()
        try {
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0 && !MimiService.isRunning)
            arguments?.getString("restore_source")?.let {
                check(it in listOf("auto", "en", "zh", "ja")); SettingsStore.setSourceLang(context, it)
            }
            arguments?.getString("restore_target")?.let {
                check(it in listOf("zh", "en", "ja")); SettingsStore.setTargetLang(context, it)
            }
            check(SettingsStore.flushPendingWritesForTests(context))
            for (provider in ServiceProvider.entries) inventory.put(JSONObject().put("provider", provider.id)
                .put("configured", SettingsStore.isConfigured(context, provider)))
            val model = arguments?.getString("select_model")
            val providerId = arguments?.getString("select_provider")
            if (model != null || providerId != null) {
                val provider = ServiceProvider.fromId(providerId ?: SettingsStore.provider(context))
                check(providerId == null || provider.id == providerId)
                check(SettingsStore.isConfigured(context, provider))
                stage = "select_saved_model"
                editor = test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
                    .putExtra("provider", provider.id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity
                test.runOnMainSync {
                    if (model != null) {
                        check(provider == ServiceProvider.DASHSCOPE && model in listOf("lite", "flash", "plus"))
                        editor!!.findViewById<android.view.View>(android.R.id.content)
                            .findViewWithTag<Spinner>("qwen-mt-model").setSelection(listOf("lite", "flash", "plus").indexOf(model))
                    }
                }
                test.waitForIdleSync()
                test.runOnMainSync { check(editor!!.findViewById<android.view.View>(R.id.save).performClick()) }
                test.waitForIdleSync()
                stage = "saved_model_and_home"
                check(SettingsStore.provider(context) == provider.id)
                if (model != null) check(SettingsStore.qwenMtModel(context) == model)
                home = test.startActivitySync(Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
                test.runOnMainSync {
                    check(home!!.findViewById<TextView>(R.id.model_summary).text.toString() ==
                        modelNamesLabel(runtimeModelNames(SettingsStore.runtimeConfiguration(context))))
                }
            }
        } catch (_: Exception) { failure = stage }
        finally { test.runOnMainSync { editor?.finish(); home?.finish() } }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "Model selection passed=${failure == null}; failureStage=${failure ?: "none"}; active=${SettingsStore.provider(context)}; qwen=${SettingsStore.qwenMtModel(context)}; inventory=$inventory; no key values read out.\n")
        })
    }
}
