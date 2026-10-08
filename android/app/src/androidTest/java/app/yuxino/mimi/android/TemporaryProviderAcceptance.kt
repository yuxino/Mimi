package app.yuxino.mimi.android

import android.content.Context
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.provider.buildRuntimeConfiguration
import java.io.File
import org.json.JSONObject

/** Explicit local test input in private app cache; never part of the release APK. */
internal class TemporaryProviderAcceptance(private val context: Context) : AutoCloseable {
    private val input = File(context.cacheDir, "acceptance-provider.json")
    private val active = SettingsStore.provider(context)
    private val source = SettingsStore.sourceLang(context)
    private val target = SettingsStore.targetLang(context)
    private var original: ServiceConfiguration? = null
    fun apply() {
        try {
            check(input.isFile && input.length() in 1..16_384)
            val value = JSONObject(input.readText(Charsets.UTF_8))
            val provider = ServiceProvider.fromId(value.getString("provider"))
            check(provider.id == value.getString("provider") && provider != ServiceProvider.DASHSCOPE)
            val fields = value.getJSONObject("credentials")
            check(fields.length() == provider.fields.size)
            val configuration = ServiceConfiguration(provider, provider.fields.associate { it.id to fields.getString(it.id) })
            val (from, to) = provider.normalize(value.getString("source"), value.getString("target"))
            // Validate using the production PC factory before any saved state changes.
            runtimeModelNames(buildRuntimeConfiguration(configuration, TranslationConfiguration(provider = TextTranslationProvider.BUILTIN), from, to, "lite"))
            original = SettingsStore.configuration(context, provider)
            check(SettingsStore.saveConfiguration(context, configuration) && SettingsStore.activateProvider(context, provider))
            SettingsStore.setSourceLang(context, from); SettingsStore.setTargetLang(context, to)
            check(SettingsStore.flushPendingWritesForTests(context))
        } finally { check(!input.exists() || input.delete()) }
    }
    override fun close() {
        original?.let {
            check(SettingsStore.saveConfiguration(context, it))
            check(SettingsStore.configuration(context, it.provider).credentials == it.credentials)
        }
        check(SettingsStore.activateProvider(context, ServiceProvider.fromId(active)))
        SettingsStore.setSourceLang(context, source); SettingsStore.setTargetLang(context, target)
        check(SettingsStore.flushPendingWritesForTests(context))
        check(SettingsStore.provider(context) == active && SettingsStore.sourceLang(context) == source && SettingsStore.targetLang(context) == target)
        check(!input.exists() || input.delete())
    }
}
