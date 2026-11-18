package app.yuxino.mimi.android.services.profiles

import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.provider.normalizeQwenMTModel

/** Pure one-time conversion; no writes, network, or inferred service from an endpoint. */
internal object LegacyServiceProfiles {
    class Migration(val profiles: List<Pair<ServiceProfile, ProfileContents>>, val activeId: String?) {
        override fun toString() = "Migration(redacted)"
    }

    private val ownedKeys = buildSet {
        addAll(listOf("provider", "api_key", "qwen_mt_model_dashscope", "alibaba_realtime_mode", "alibaba_workspace_id", "text_translation_provider"))
        ServiceProvider.entries.forEach { provider ->
            add("api_key_${provider.id}"); add("base_url_${provider.id}"); add("model_${provider.id}")
            provider.fields.forEach { add("credential_${provider.id}_${it.id}") }
        }
        TextTranslationProvider.entries.forEach { provider ->
            listOf("endpoint", "model", "api_key", "local_http").forEach { add("text_translation_${provider.storageId}_$it") }
        }
        listOf("enabled", "endpoint", "model", "api_key", "local_http").forEach { add("chatmock_$it") }
    }
    fun ownsKey(key: String): Boolean = key in ownedKeys

    fun read(values: Map<String, *>): Migration {
        fun string(key: String) = values[key] as? String ?: ""
        fun boolean(key: String) = values[key] as? Boolean ?: false
        val activeProvider = string("provider").ifEmpty { ServiceProvider.DASHSCOPE.id }
        val selected = if ("text_translation_provider" in values) TextTranslationProvider.fromStorageId(string("text_translation_provider"))
            else if (boolean("chatmock_enabled")) TextTranslationProvider.OPENAI_COMPATIBLE else TextTranslationProvider.BUILTIN
        val drafts = TextTranslationProvider.entries.filterNot { it in setOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE) }
            .mapNotNull { provider ->
                val prefix = "text_translation_${provider.storageId}_"
                val legacy = provider == TextTranslationProvider.OPENAI_COMPATIBLE && prefix + "endpoint" !in values
                val sourcePrefix = if (legacy) "chatmock_" else prefix
                if (values.keys.none { it.startsWith(sourcePrefix) }) null else provider to TranslationConfiguration(
                    string(sourcePrefix + "endpoint"), string(sourcePrefix + "model"), string(sourcePrefix + "api_key"),
                    boolean(sourcePrefix + "local_http"), provider)
            }.toMap()
        val profiles = ServiceProvider.entries.mapNotNull { provider ->
            val fieldKeys = provider.fields.map { if (it.id == "apiKey") "api_key_${provider.id}" else "credential_${provider.id}_${it.id}" }
            val sharedKey = provider.id == activeProvider && "api_key" in values
            val touched = sharedKey || "api_key_${provider.id}" in values || fieldKeys.any { it in values } || "base_url_${provider.id}" in values || "model_${provider.id}" in values ||
                (provider == ServiceProvider.DASHSCOPE && (drafts.isNotEmpty() || "text_translation_provider" in values ||
                    "qwen_mt_model_dashscope" in values || "alibaba_realtime_mode" in values || "alibaba_workspace_id" in values))
            if (!touched) return@mapNotNull null
            val credentials = provider.fields.associate { field ->
                val key = if (field.id == "apiKey") "api_key_${provider.id}" else "credential_${provider.id}_${field.id}"
                field.id to if (field.id == "apiKey" && key !in values && sharedKey) string("api_key") else string(key)
            }
            val preservedCredentials = if ("apiKey" !in credentials && ("api_key_${provider.id}" in values || sharedKey))
                credentials + ("apiKey" to if ("api_key_${provider.id}" in values) string("api_key_${provider.id}") else string("api_key")) else credentials
            val speech = ServiceConfiguration(provider, preservedCredentials, string("base_url_${provider.id}"), string("model_${provider.id}"),
                if (provider == ServiceProvider.DASHSCOPE) string("alibaba_realtime_mode").ifEmpty { "asrMt" } else "asrMt",
                if (provider == ServiceProvider.DASHSCOPE) string("alibaba_workspace_id") else "")
            ServiceProfile("legacy-${provider.id}", provider.title, provider) to if (provider == ServiceProvider.DASHSCOPE)
                ProfileContents(speech, selected, drafts, normalizeQwenMTModel(string("qwen_mt_model_dashscope")))
                else ProfileContents(speech, xaiIndependentTranslationMigrated = provider != ServiceProvider.XAI)
        }
        return Migration(profiles, profiles.firstOrNull { it.first.provider.id == activeProvider }?.first?.id)
    }
}
