package app.yuxino.mimi.android.services.acceptance

import android.content.Context
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.provider.buildRuntimeConfiguration
import app.yuxino.mimi.android.services.presentation.runtimeModelNames
import app.yuxino.mimi.android.services.translation.ProfileSettingsSnapshot
import app.yuxino.mimi.android.settings.SettingsStore
import java.io.File
import org.json.JSONObject

/** Explicit local test input in private app cache; never part of the release APK. */
internal class TemporaryProviderAcceptance(private val context: Context) : AutoCloseable {
    private val input = File(context.cacheDir, "acceptance-provider.json")
    private var original: ProfileSettingsSnapshot? = null
    fun apply() {
        try {
            check(original == null && input.isFile && input.length() in 1..16_384)
            val value = JSONObject(input.readText(Charsets.UTF_8))
            val provider = ServiceProvider.fromId(value.getString("provider"))
            check(provider.id == value.getString("provider"))
            val fields = value.getJSONObject("credentials")
            check(fields.length() == provider.fields.size)
            val configuration = ServiceConfiguration(provider, provider.fields.associate {
                val field = fields.get(it.id); check(field is String); it.id to field
            }, alibabaRealtimeMode = value.optString("alibabaRealtimeMode", "asrMt"),
                alibabaWorkspaceId = value.optString("alibabaWorkspaceId", ""))
            val testTranslation = TranslationConfiguration(provider = TextTranslationProvider.BUILTIN)
            val (from, to) = provider.normalize(value.getString("source"), value.getString("target"),
                testTranslation.provider, configuration.alibabaRealtimeMode)
            val testModel = value.optString("translationModel", "lite")
            check(testModel in listOf("lite", "flash", "plus"))
            // Validate using the production PC factory before any saved state changes.
            runtimeModelNames(buildRuntimeConfiguration(configuration, testTranslation, from, to, testModel))
            original = ProfileSettingsSnapshot(context)
            // Always a distinct profile: never borrow or overwrite a user's same-provider entry.
            checkNotNull(SettingsStore.saveProfile(context, null, "Acceptance fixture", configuration,
                testTranslation, qwenMtModel = testModel, activate = true))
            SettingsStore.setSourceLang(context, from); SettingsStore.setTargetLang(context, to)
            check(SettingsStore.flushPendingWritesForTests(context))
        } catch (_: Exception) {
            throw IllegalStateException("invalid_acceptance_provider_input")
        } finally { check(!input.exists() || input.delete()) }
    }
    override fun close() {
        original?.close()
        original = null
        check(!input.exists() || input.delete())
    }
}
