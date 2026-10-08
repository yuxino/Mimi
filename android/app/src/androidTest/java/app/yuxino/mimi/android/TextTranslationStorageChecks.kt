package app.yuxino.mimi.android

import android.content.Context
import android.content.SharedPreferences
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration

/** Synthetic storage-only checks: no provider connection, capture, or credential logging. */
internal fun runTextTranslationStorageChecks(context: Context) {
    check(!MimiService.isRunning) { "Stop the active session before storage checks" }
    SettingsStore.textTranslationProvider(context)
    check(SettingsStore.flushPendingWritesForTests(context)) { "Could not synchronize storage fixture" }
    val preferences = SettingsStore::class.java.getDeclaredField("prefs").run {
        isAccessible = true
        get(SettingsStore) as SharedPreferences
    }
    fun snapshot(): Map<String, Any?> = preferences.all.mapValues { (_, value) ->
        if (value is Set<*>) value.toSet() else value
    }
    val speech = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-storage-speech-key"))
    val legacy = TranslationConfiguration(
        endpoint = "https://legacy.example.invalid/v1",
        model = "synthetic-legacy-model",
        apiKey = "synthetic-legacy-text-key",
        provider = TextTranslationProvider.OPENAI_COMPATIBLE,
    )
    val deepL = TranslationConfiguration(apiKey = "synthetic-storage-deepl-key:fx", provider = TextTranslationProvider.DEEPL)
    val deepLX = TranslationConfiguration(
        endpoint = "https://deeplx.example.invalid/proxy/translate",
        apiKey = "synthetic-storage-deeplx-key",
        provider = TextTranslationProvider.DEEPLX,
    )
    val openAI = TranslationConfiguration(
        endpoint = "https://chatmock.example.invalid/v1",
        model = "synthetic-storage-model",
        apiKey = "synthetic-storage-openai-key",
        provider = TextTranslationProvider.OPENAI_COMPATIBLE,
    )
    val chatMock = TranslationConfiguration(
        endpoint = "http://127.0.0.1:8000/v1",
        model = "synthetic-chatmock-model",
        apiKey = "synthetic-chatmock-key",
        allowLocalHttp = true,
        provider = TextTranslationProvider.CHAT_MOCK,
    )
    fun save(configuration: TranslationConfiguration) {
        check(SettingsStore.saveConfiguration(context, speech, configuration)) { "Synthetic translation save failed" }
        check(SettingsStore.textTranslationProvider(context) == configuration.provider) { "Saved translation mode was lost" }
        check(SettingsStore.configuration(context, ServiceProvider.DASHSCOPE).credentials == speech.credentials) {
            "Text configuration changed the recognition credentials"
        }
    }
    withTextTranslationSettingsSnapshot(context) {
        for (enabled in listOf(false, true)) {
            val seed = preferences.edit()
            preferences.all.keys.filter { it.startsWith("text_translation_") || it.startsWith("chatmock_") }
                .forEach(seed::remove)
            // Isolate any historical speech-key migration from this text-migration fixture.
            seed.remove("api_key")
                .putString("provider", ServiceProvider.DASHSCOPE.id)
                .putString("api_key_dashscope", speech.value("apiKey"))
                .putString("base_url_dashscope", "")
                .putString("model_dashscope", "")
                .putBoolean("chatmock_enabled", enabled)
                .putString("chatmock_endpoint", legacy.endpoint)
                .putString("chatmock_model", legacy.model)
                .putString("chatmock_api_key", legacy.apiKey)
                .putBoolean("chatmock_local_http", legacy.allowLocalHttp)
            check(seed.commit()) { "Could not seed legacy text configuration" }
            val beforeRead = snapshot()
            check(SettingsStore.textTranslationProvider(context) == if (enabled)
                TextTranslationProvider.OPENAI_COMPATIBLE else TextTranslationProvider.BUILTIN)
            check(!SettingsStore.useChatMockTranslation(context)) { "Legacy combined settings were relabelled as ChatMock" }
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.OPENAI_COMPATIBLE) == legacy)
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.CHAT_MOCK) == TranslationConfiguration(provider = TextTranslationProvider.CHAT_MOCK)) {
                "The separate ChatMock entry borrowed legacy compatible settings"
            }
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.DEEPL).apiKey.isEmpty())
            check(SettingsStore.translationConfiguration(context, TextTranslationProvider.DEEPLX).apiKey.isEmpty())
            check(snapshot() == beforeRead) { "Reading legacy text settings wrote or enabled a configuration" }
        }

        // Saving another service first must retain the old ChatMock configuration atomically.
        save(deepL)
        check(SettingsStore.translationConfiguration(context) == deepL)
        check(SettingsStore.translationConfiguration(context, TextTranslationProvider.OPENAI_COMPATIBLE) == legacy) {
            "The first DeepL save lost the previous ChatMock configuration"
        }
        check(preferences.all.keys.none { it.startsWith("chatmock_") }) { "Explicit save left legacy text settings behind" }
        save(chatMock)
        check(SettingsStore.useChatMockTranslation(context))
        check(SettingsStore.translationConfiguration(context, TextTranslationProvider.OPENAI_COMPATIBLE) == legacy) {
            "Saving the new ChatMock entry overwrote the legacy compatible service"
        }
        save(deepLX)
        save(openAI)
        val networkConfigurations = listOf(deepL, deepLX, openAI, chatMock)
        fun assertNetworkConfigurations() {
            for (configuration in networkConfigurations) {
                check(SettingsStore.translationConfiguration(context, configuration.provider) == configuration) {
                    "Switching text services changed another service's configuration"
                }
            }
        }
        // Re-select each saved service, preserving its own key and all other saved services.
        for (configuration in networkConfigurations) {
            save(SettingsStore.translationConfiguration(context, configuration.provider))
            check(SettingsStore.translationConfiguration(context) == configuration)
            assertNetworkConfigurations()
        }
        for (provider in listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) {
            save(TranslationConfiguration(provider = provider))
            check(SettingsStore.originalTextOnly(context) == (provider == TextTranslationProvider.NONE))
            check(SettingsStore.translationConfiguration(context).apiKey.isEmpty())
            assertNetworkConfigurations()
        }

        // A saved Alibaba-only route must not affect an integrated speech provider.
        val integrated = ServiceConfiguration(ServiceProvider.OPENAI, mapOf("apiKey" to "synthetic-storage-integrated-key"))
        check(SettingsStore.saveConfiguration(context, integrated))
        check(SettingsStore.activateProvider(context, ServiceProvider.OPENAI))
        check(!SettingsStore.originalTextOnly(context))
        check(SettingsStore.textTranslationProvider(context) == TextTranslationProvider.NONE)
        assertNetworkConfigurations()
        check(SettingsStore.activateProvider(context, ServiceProvider.DASHSCOPE))
        check(SettingsStore.originalTextOnly(context))

        val beforeInvalidSave = snapshot()
        val replacementSpeech = ServiceConfiguration(ServiceProvider.DASHSCOPE,
            mapOf("apiKey" to "synthetic-unsaved-replacement-key"), "wss://unsaved.example.invalid/realtime", "synthetic-unsaved-model")
        val invalidSave = runCatching {
            SettingsStore.saveConfiguration(context, replacementSpeech, deepL.copy(apiKey = ""))
        }
        check(invalidSave.exceptionOrNull() is IllegalArgumentException) { "An empty DeepL key was accepted" }
        check(snapshot() == beforeInvalidSave) { "Invalid text settings partially changed saved speech or translation values" }

        // The text model has its own preference. Legacy realtime values and all keys survive.
        val preservedSpeech = ServiceConfiguration(ServiceProvider.DASHSCOPE, speech.credentials,
            "wss://legacy.example.invalid/realtime", "qwen3.5-livetranslate-flash-realtime")
        check(SettingsStore.saveConfiguration(context, preservedSpeech, TranslationConfiguration(provider = TextTranslationProvider.BUILTIN)))
        check(preferences.edit().remove("qwen_mt_model_dashscope").commit())
        val beforeModelRead = snapshot()
        check(SettingsStore.qwenMtModel(context) == "lite")
        check(snapshot() == beforeModelRead) { "Reading the new text model changed legacy settings" }
        for (stored in listOf("", "unknown")) {
            check(preferences.edit().putString("qwen_mt_model_dashscope", stored).commit())
            val beforeEmptyRead = snapshot()
            check(SettingsStore.qwenMtModel(context) == "lite")
            check(snapshot() == beforeEmptyRead) { "Reading an empty text model rewrote preferences" }
        }
        for (model in listOf("lite", "flash", "plus")) {
            check(SettingsStore.saveConfiguration(context, preservedSpeech,
                TranslationConfiguration(provider = TextTranslationProvider.BUILTIN), qwenMtModel = model))
            check(SettingsStore.qwenMtModel(context) == model)
            check(SettingsStore.configuration(context, ServiceProvider.DASHSCOPE).model == preservedSpeech.model) {
                "The text picker overwrote the legacy realtime model"
            }
            check(SettingsStore.configuration(context, ServiceProvider.DASHSCOPE).credentials == speech.credentials)
            assertNetworkConfigurations()
        }
        val beforeInvalidModel = snapshot()
        check(runCatching { SettingsStore.saveConfiguration(context, preservedSpeech,
            TranslationConfiguration(provider = TextTranslationProvider.BUILTIN), qwenMtModel = "") }.exceptionOrNull() is IllegalArgumentException)
        check(snapshot() == beforeInvalidModel) { "Invalid text model partially changed stored credentials" }
        check(preferences.edit().putString("source_lang", "yue").putString("target_lang", "zh").commit())
        val beforeEffectiveRead = snapshot()
        val effective = SettingsStore.runtimeConfiguration(context)
        check(effective.getString("sourceLanguage") != "yue") { "The effective Audio3 session retained an unsupported legacy ASR hint" }
        check(effective.getString("qwenMtModel") == "plus")
        check(snapshot() == beforeEffectiveRead) { "Building native input rewrote the stored language or key preference" }
        check(!MimiService.isRunning) { "Storage checks started an audio session" }
    }
}

/** Restore the entire encrypted store, including inactive translators and unrelated typed preferences. */
internal fun withTextTranslationSettingsSnapshot(context: Context, action: () -> Unit) {
    SettingsStore.textTranslationProvider(context)
    check(SettingsStore.flushPendingWritesForTests(context)) { "Could not synchronize storage fixture" }
    val preferences = SettingsStore::class.java.getDeclaredField("prefs").run {
        isAccessible = true
        get(SettingsStore) as SharedPreferences
    }
    fun snapshot(): Map<String, Any?> = preferences.all.mapValues { (_, value) ->
        if (value is Set<*>) value.toSet() else value
    }
    val original = snapshot()
    try {
        action()
    } finally {
        val restore = preferences.edit().clear()
        original.forEach { (key, value) ->
            when (value) {
                is String -> restore.putString(key, value)
                is Boolean -> restore.putBoolean(key, value)
                is Int -> restore.putInt(key, value)
                is Long -> restore.putLong(key, value)
                is Float -> restore.putFloat(key, value)
                is Set<*> -> restore.putStringSet(key, value.map {
                    check(it is String) { "Unsupported saved preference type" }
                    it
                }.toSet())
                null -> restore.putString(key, null)
                else -> error("Unsupported saved preference type")
            }
        }
        check(restore.commit()) { "Storage fixture restoration did not reach disk" }
        check(snapshot() == original) { "Storage fixture did not restore the complete preference snapshot" }
    }
}
