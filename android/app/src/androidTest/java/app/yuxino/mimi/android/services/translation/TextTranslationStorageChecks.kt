package app.yuxino.mimi.android.services.translation

import android.content.Context
import android.content.SharedPreferences
import android.content.pm.ApplicationInfo
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.services.profiles.AndroidProfilePersistence
import app.yuxino.mimi.android.services.profiles.LegacyServiceProfiles
import app.yuxino.mimi.android.services.profiles.ServiceProfileRepository
import app.yuxino.mimi.android.settings.SettingsStore

/** Synthetic storage-only checks: no provider connection, capture, or credential logging. */
internal fun runTextTranslationStorageChecks(context: Context) = withTextTranslationSettingsSnapshot(context) {
    val speech = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "synthetic-storage-speech-key"))
    val legacy = TranslationConfiguration(endpoint = "https://legacy.example.invalid/v1", model = "legacy-model",
        apiKey = "synthetic-legacy-text-key", provider = TextTranslationProvider.OPENAI_COMPATIBLE)
    val deepL = TranslationConfiguration(apiKey = "synthetic-storage-deepl-key:fx", provider = TextTranslationProvider.DEEPL)
    val deepLX = TranslationConfiguration(endpoint = "https://deeplx.example.invalid/translate", apiKey = "synthetic-deeplx-key", provider = TextTranslationProvider.DEEPLX)
    val openAI = TranslationConfiguration(endpoint = "https://compatible.example.invalid/v1", model = "synthetic-model", apiKey = "synthetic-openai-key", provider = TextTranslationProvider.OPENAI_COMPATIBLE)
    val chatMock = TranslationConfiguration(endpoint = "http://127.0.0.1:8000/v1", model = "synthetic-chatmock-model",
        apiKey = "synthetic-chatmock-key", allowLocalHttp = true, provider = TextTranslationProvider.CHAT_MOCK)

    // The pure migration fixture retains the old generic endpoint without calling it ChatMock.
    for (enabled in listOf(false, true)) {
        val migrated = LegacyServiceProfiles.read(mapOf("provider" to "dashscope", "api_key_dashscope" to speech.value("apiKey"),
            "chatmock_enabled" to enabled, "chatmock_endpoint" to legacy.endpoint, "chatmock_model" to legacy.model,
            "chatmock_api_key" to legacy.apiKey)).profiles.single().second
        check(migrated.selectedTranslation == if (enabled) TextTranslationProvider.OPENAI_COMPATIBLE else TextTranslationProvider.BUILTIN)
        check(migrated.translations[TextTranslationProvider.OPENAI_COMPATIBLE] == legacy)
        check(TextTranslationProvider.CHAT_MOCK !in migrated.translations)
    }
    val id = checkNotNull(SettingsStore.saveProfile(context, null, "Storage fixture", speech, legacy, activate = true))
    fun save(configuration: TranslationConfiguration, qwen: String? = null, replacement: ServiceConfiguration = speech) {
        check(SettingsStore.saveProfile(context, id, "Storage fixture", replacement, configuration, qwenMtModel = qwen) == id)
        check(SettingsStore.textTranslationProvider(context) == configuration.provider)
        check(SettingsStore.profileConfiguration(context, id).credentials == speech.credentials)
    }
    save(deepL)
    check(SettingsStore.profileTranslationConfiguration(context, id, TextTranslationProvider.OPENAI_COMPATIBLE) == legacy)
    save(chatMock)
    check(SettingsStore.useChatMockTranslation(context))
    check(SettingsStore.profileTranslationConfiguration(context, id, TextTranslationProvider.OPENAI_COMPATIBLE) == legacy)
    save(deepLX); save(openAI)
    val networkConfigurations = listOf(deepL, deepLX, openAI, chatMock)
    fun assertDrafts() {
        networkConfigurations.forEach { check(SettingsStore.profileTranslationConfiguration(context, id, it.provider) == it) }
    }
    networkConfigurations.forEach { save(SettingsStore.profileTranslationConfiguration(context, id, it.provider)); assertDrafts() }
    listOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE).forEach {
        save(TranslationConfiguration(provider = it))
        check(SettingsStore.originalTextOnly(context) == (it == TextTranslationProvider.NONE))
        check(SettingsStore.translationConfiguration(context).apiKey.isEmpty()); assertDrafts()
    }
    // A separate integrated profile must neither inherit nor replace Alibaba's translator drafts.
    val integrated = ServiceConfiguration(ServiceProvider.OPENAI, mapOf("apiKey" to "synthetic-integrated-key"))
    val integratedId = checkNotNull(SettingsStore.saveProfile(context, null, "Integrated fixture", integrated, activate = true))
    check(!SettingsStore.originalTextOnly(context))
    // Integrated mode retains the independent editor's selection for a later
    // mode switch; the effective runtime must use only this speech profile.
    val integratedRuntime = SettingsStore.runtimeConfiguration(context)
    check(integratedRuntime.getString("provider") == "openAIRealtime")
    check(integratedRuntime.getString("speechTranslationMode") == "simultaneous")
    check(integratedRuntime.isNull("textCredentials"))
    check(integratedRuntime.getJSONObject("credentials").getString("apiKey") == "synthetic-integrated-key")
    check(SettingsStore.translationConfiguration(context, TextTranslationProvider.DEEPL).apiKey.isEmpty()); assertDrafts()
    check(SettingsStore.activateProfile(context, id)); check(SettingsStore.originalTextOnly(context))
    val before = profileDisk().readCatalog()
    val invalid = ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to "unsaved-replacement"), "wss://unsaved.example.invalid", "unsaved")
    check(SettingsStore.saveProfile(context, id, "Invalid", invalid, deepL.copy(apiKey = "")) == null)
    check(profileDisk().readCatalog() == before)
    val preserved = ServiceConfiguration(ServiceProvider.DASHSCOPE, speech.credentials, "wss://legacy.example.invalid", "qwen3.5-livetranslate-flash-realtime")
    for (model in listOf("lite", "flash", "plus")) {
        save(TranslationConfiguration(provider = TextTranslationProvider.BUILTIN), model, preserved)
        check(SettingsStore.qwenMtModel(context) == model)
        check(SettingsStore.profileConfiguration(context, id).model == preserved.model); assertDrafts()
    }
    val beforeInvalidModel = profileDisk().readCatalog()
    check(SettingsStore.saveProfile(context, id, "Invalid", preserved, qwenMtModel = "") == null)
    check(profileDisk().readCatalog() == beforeInvalidModel)
    SettingsStore.setSourceLang(context, "yue"); SettingsStore.setTargetLang(context, "zh")
    val beforeRuntime = profileDisk().readCatalog()
    val effective = SettingsStore.runtimeConfiguration(context)
    check(effective.getString("sourceLanguage") != "yue")
    check(effective.getString("qwenMtModel") == "plus")
    check(SettingsStore.sourceLang(context) == "yue" && profileDisk().readCatalog() == beforeRuntime)
    check(SettingsStore.deleteProfile(context, integratedId))
    check(!MimiService.isRunning)
}

private fun checkFixtureAllowed(context: Context) {
    check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) { "Storage fixtures require a debug application" }
    check(!MimiService.isRunning && !MimiService.isStopping) { "Stop the session before storage fixtures" }
}

private fun settingsPreferences(): SharedPreferences = SettingsStore::class.java.getDeclaredField("prefs").run {
    isAccessible = true; get(SettingsStore) as SharedPreferences
}

private fun profileDisk(): AndroidProfilePersistence {
    val repository = SettingsStore::class.java.getDeclaredField("profileRepository").run { isAccessible = true; get(SettingsStore) }
    return ServiceProfileRepository::class.java.getDeclaredField("disk").run { isAccessible = true; get(repository) as AndroidProfilePersistence }
}

private fun copyPreferences(preferences: SharedPreferences): Map<String, Any?> = preferences.all.mapValues { (_, value) ->
    if (value is Set<*>) value.toSet() else value
}

private fun restorePreferences(preferences: SharedPreferences, original: Map<String, Any?>, removeExtras: Boolean) {
    val restore = preferences.edit()
    if (removeExtras) preferences.all.keys.filterNot(original::containsKey).forEach(restore::remove)
    original.forEach { (key, value) ->
        when (value) {
            is String -> restore.putString(key, value)
            is Boolean -> restore.putBoolean(key, value)
            is Int -> restore.putInt(key, value)
            is Long -> restore.putLong(key, value)
            is Float -> restore.putFloat(key, value)
            is Set<*> -> restore.putStringSet(key, value.map { check(it is String); it }.toSet())
            null -> restore.remove(key)
            else -> error("Unsupported fixture preference type")
        }
    }
    check(restore.commit()) { "Storage fixture restore did not reach disk" }
}

/** Private bounded in-memory snapshot, never printed or exported; restore secrets before their catalog. */
internal class ProfileSettingsSnapshot(private val context: Context) : AutoCloseable {
    private val preferences: SharedPreferences
    private val disk: AndroidProfilePersistence
    private val credentialPreferences: SharedPreferences
    private val originalPreferences: Map<String, Any?>
    private val originalCredentials: Map<String, Any?>
    private val originalCatalog: String
    init {
        checkFixtureAllowed(context)
        SettingsStore.profiles(context)
        check(SettingsStore.flushPendingWritesForTests(context))
        preferences = settingsPreferences(); disk = profileDisk()
        credentialPreferences = AndroidProfilePersistence::class.java.getDeclaredField("secrets").run {
            isAccessible = true; get(disk) as SharedPreferences
        }
        originalPreferences = copyPreferences(preferences)
        originalCredentials = copyPreferences(credentialPreferences)
        originalCatalog = checkNotNull(disk.readCatalog())
    }
    override fun close() {
        checkFixtureAllowed(context)
        // Keep test revisions until the original catalog is durable, even if restoration fails.
        restorePreferences(credentialPreferences, originalCredentials, removeExtras = false)
        check(disk.writeCatalog(originalCatalog)) { "Profile catalog restoration failed" }
        restorePreferences(credentialPreferences, originalCredentials, removeExtras = true)
        restorePreferences(preferences, originalPreferences, removeExtras = true)
        check(disk.readCatalog() == originalCatalog && copyPreferences(credentialPreferences) == originalCredentials &&
            copyPreferences(preferences) == originalPreferences) { "Storage fixture restore mismatch" }
    }
}

/** Includes inactive profiles, all translator drafts, exact active ID and unrelated typed preferences. */
internal fun withTextTranslationSettingsSnapshot(context: Context, action: () -> Unit) {
    ProfileSettingsSnapshot(context).use { action() }
}
