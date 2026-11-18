package app.yuxino.mimi.android.services.editor

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.view.View
import android.widget.Spinner
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.services.profiles.AndroidProfilePersistence
import app.yuxino.mimi.android.services.profiles.ServiceProfileRepository
import app.yuxino.mimi.android.services.translation.ProfileSettingsSnapshot
import app.yuxino.mimi.android.settings.SettingsStore
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout

/** Native Save/probe acceptance using isolated synthetic profiles only; no network or capture. */
internal class SpeechCredentialDraftChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private val activities = mutableListOf<ServiceSettingsActivity>()
    private var stage = "preconditions"

    fun run() {
        var failure: String? = null
        var snapshot: ProfileSettingsSnapshot? = null
        try {
            check(context.packageName.endsWith(".dev") &&
                context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!MimiService.isStarting && !MimiService.isRunning && !MimiService.isStopping)
            snapshot = ProfileSettingsSnapshot(context)
            SettingsStore.setSourceLang(context, "en")
            SettingsStore.setTargetLang(context, "original")
            check(SettingsStore.flushPendingWritesForTests(context))
            azure()
            tencent()
            baidu()
        } catch (_: Throwable) {
            failure = stage
        } finally {
            try {
                onUi { activities.forEach { if (!it.isFinishing) it.finish() } }
                test.waitForIdleSync()
                snapshot?.close()
            } catch (_: Throwable) { failure = "restore" }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null)
                "Speech credential draft UI passed: Azure endpoint, Tencent account/SecretID and Baidu account changes reject blank replacement secrets; unchanged owners retain synthetic secrets; Azure probe and native Save paths checked; exact profile/preferences snapshot restored; no provider request or capture.\n"
            else "Speech credential draft UI failed: stage=$failure; no credential values logged.\n")
        })
    }

    private fun azure() {
        stage = "azure_fixture"
        val original = mapOf("endpoint" to "https://synthetic-old.openai.azure.com", "deployment" to "",
            "transcriptionDeployment" to "synthetic-asr", "apiKey" to "synthetic-azure-old-key")
        val id = create(ServiceConfiguration(ServiceProvider.AZURE, original, speechTranslationMode = "asrMt"))
        var editor = open(id)
        onUi {
            field<Spinner>(editor, "speech-translation-mode").setSelection(1)
            field<Spinner>(editor, "translation-mode").setSelection(0)
        }
        test.waitForIdleSync()
        stage = "azure_invalid_deployment_does_not_mark_saved_key_missing"
        onUi {
            field<TextInputEditText>(editor, "credential-transcriptionDeployment").setText("invalid deployment")
            check(probe(editor) == null)
            var parent = field<TextInputEditText>(editor, "credential-apiKey").parent
            while (parent !is TextInputLayout) parent = parent.parent
            check(parent.error == null)
            field<TextInputEditText>(editor, "credential-transcriptionDeployment").setText("synthetic-asr")
            check(probe(editor) != null)
            val status = ServiceSettingsActivity::class.java.getDeclaredField("status").run {
                isAccessible = true; get(editor) as View
            }
            check(status.visibility == View.GONE)
        }
        stage = "azure_endpoint_requires_new_key"
        onUi {
            field<TextInputEditText>(editor, "credential-endpoint").setText("https://synthetic-new.openai.azure.com")
            check(field<TextInputEditText>(editor, "credential-apiKey").text.isNullOrBlank())
            check(ServiceSettingsActivity::class.java.getDeclaredMethod("speechProbeConfiguration").run {
                isAccessible = true; invoke(editor)
            } == null)
        }
        reject(editor, id, original, listOf("apiKey"))
        stage = "azure_explicit_key_save"
        onUi { field<TextInputEditText>(editor, "credential-apiKey").setText("synthetic-azure-new-key") }
        save(editor)
        val replacement = original + mapOf("endpoint" to "https://synthetic-new.openai.azure.com", "apiKey" to "synthetic-azure-new-key")
        check(SettingsStore.profileConfiguration(context, id).credentials == replacement)
        stage = "azure_same_owner_retains_key"
        editor = open(id)
        onUi {
            check(field<TextInputEditText>(editor, "credential-apiKey").text.isNullOrBlank())
            check(ServiceSettingsActivity::class.java.getDeclaredMethod("speechProbeConfiguration").run {
                isAccessible = true; invoke(editor)
            } != null)
            field<TextInputEditText>(editor, "credential-transcriptionDeployment").setText("synthetic-asr-updated")
        }
        save(editor)
        check(SettingsStore.profileConfiguration(context, id).credentials == replacement + ("transcriptionDeployment" to "synthetic-asr-updated"))
    }

    private fun tencent() {
        stage = "tencent_fixture"
        val original = mapOf("appId" to "1234567", "secretId" to "synthetic-tencent-old-id", "secretKey" to "synthetic-tencent-old-key")
        val id = create(ServiceConfiguration(ServiceProvider.TENCENT, original))
        var editor = open(id)
        stage = "tencent_account_requires_both_secrets"
        onUi { field<TextInputEditText>(editor, "credential-appId").setText("7654321") }
        reject(editor, id, original, listOf("secretId", "secretKey"))
        stage = "tencent_secret_id_requires_new_key"
        onUi {
            field<TextInputEditText>(editor, "credential-appId").setText(original.getValue("appId"))
            field<TextInputEditText>(editor, "credential-secretId").setText("synthetic-tencent-new-id")
        }
        reject(editor, id, original, listOf("secretKey"))
        stage = "tencent_explicit_secret_pair_save"
        onUi { field<TextInputEditText>(editor, "credential-secretKey").setText("synthetic-tencent-new-key") }
        save(editor)
        val replacement = original + mapOf("secretId" to "synthetic-tencent-new-id", "secretKey" to "synthetic-tencent-new-key")
        check(SettingsStore.profileConfiguration(context, id).credentials == replacement)
        stage = "tencent_same_owner_retains_pair"
        editor = open(id)
        onUi {
            check(field<TextInputEditText>(editor, "credential-secretId").text.isNullOrBlank())
            check(field<TextInputEditText>(editor, "credential-secretKey").text.isNullOrBlank())
        }
        save(editor)
        check(SettingsStore.profileConfiguration(context, id).credentials == replacement)
    }

    private fun baidu() {
        stage = "baidu_fixture"
        val original = mapOf("appId" to "1234567", "appKey" to "synthetic-baidu-old-key")
        val id = create(ServiceConfiguration(ServiceProvider.BAIDU, original))
        var editor = open(id)
        stage = "baidu_account_requires_new_key"
        onUi { field<TextInputEditText>(editor, "credential-appId").setText("7654321") }
        reject(editor, id, original, listOf("appKey"))
        stage = "baidu_explicit_key_save"
        onUi { field<TextInputEditText>(editor, "credential-appKey").setText("synthetic-baidu-new-key") }
        save(editor)
        val replacement = mapOf("appId" to "7654321", "appKey" to "synthetic-baidu-new-key")
        check(SettingsStore.profileConfiguration(context, id).credentials == replacement)
        stage = "baidu_same_owner_retains_key"
        editor = open(id)
        onUi { check(field<TextInputEditText>(editor, "credential-appKey").text.isNullOrBlank()) }
        save(editor)
        check(SettingsStore.profileConfiguration(context, id).credentials == replacement)
    }

    private fun create(config: ServiceConfiguration): String = checkNotNull(SettingsStore.saveProfile(context, null,
        "Synthetic speech draft fixture", config, if (config.provider == ServiceProvider.AZURE)
            TranslationConfiguration(provider = TextTranslationProvider.NONE) else null,
        translationEnabled = if (config.provider == ServiceProvider.AZURE) true else null, activate = false))

    private fun open(id: String): ServiceSettingsActivity {
        check(SettingsStore.profiles(context).first { it.id == id }.name == "Synthetic speech draft fixture")
        val editor = test.startActivitySync(Intent(context, ServiceSettingsActivity::class.java)
            .putExtra("profileId", id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ServiceSettingsActivity
        activities += editor
        test.waitForIdleSync()
        return editor
    }

    private fun reject(editor: ServiceSettingsActivity, id: String, expected: Map<String, String>, required: List<String>) {
        val before = catalog()
        onUi { check(editor.findViewById<View>(R.id.save).performClick()) }
        test.waitForIdleSync()
        check(!editor.isFinishing)
        check(catalog() == before)
        check(SettingsStore.profileConfiguration(context, id).credentials == expected)
        onUi {
            for (name in required) {
                var parent = field<TextInputEditText>(editor, "credential-$name").parent
                while (parent !is TextInputLayout) parent = parent.parent
                check(parent.error?.toString() == editor.getString(R.string.service_required))
            }
        }
    }

    private fun catalog(): String {
        val repository = SettingsStore::class.java.getDeclaredField("profileRepository").run { isAccessible = true; get(SettingsStore) }
        val disk = ServiceProfileRepository::class.java.getDeclaredField("disk").run { isAccessible = true; get(repository) as AndroidProfilePersistence }
        return checkNotNull(disk.readCatalog())
    }

    private fun save(editor: ServiceSettingsActivity) {
        onUi { check(editor.findViewById<View>(R.id.save).performClick()) }
        test.waitForIdleSync()
        check(editor.isFinishing)
    }

    private fun probe(editor: ServiceSettingsActivity): Any? =
        ServiceSettingsActivity::class.java.getDeclaredMethod("speechProbeConfiguration").run {
            isAccessible = true; invoke(editor)
        }

    private inline fun <reified T : View> field(editor: Activity, tag: String): T =
        checkNotNull(editor.findViewById<View>(android.R.id.content).findViewWithTag<T>(tag))
    private fun onUi(action: () -> Unit) = test.runOnMainSync(action)
}
