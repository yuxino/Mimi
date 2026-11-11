package app.yuxino.mimi.android

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.provider.validateTranslationConfiguration
import app.yuxino.mimi.android.provider.buildRuntimeConfiguration
import app.yuxino.mimi.android.capture.CaptureInput
import org.json.JSONObject

/** App-private settings. Service metadata and encrypted, profile-scoped credentials are separate. */
object SettingsStore {
    private const val FILE = "mimi_secure"

    private const val KEY_SOURCE_LANG = "source_lang"
    private const val KEY_TARGET_LANG = "target_lang"
    private const val KEY_FONT_SIZE = "font_size"
    private const val KEY_OVERLAY_OPACITY = "overlay_opacity"
    private const val KEY_OVERLAY_BG_ALPHA = "overlay_bg_alpha"
    private const val KEY_IMMERSIVE_SUBTITLES = "immersive_subtitles"
    private const val KEY_OVERLAY_ADJUSTABLE = "overlay_adjustable"
    private const val KEY_HISTORY_LINES = "history_lines"
    private const val KEY_CAPTURE_INPUT = "capture_input"
    private const val KEY_RETAIN_SESSION_HISTORY = "retain_session_history"
    private const val KEY_SUBTITLE_DIVIDERS = "subtitle_dividers"
    private const val KEY_SUBTITLE_TIMESTAMPS = "subtitle_timestamps"
    private const val KEY_HOTWORDS = "hotwords"
    private const val KEY_TRANSLATION_COLOR = "translation_color"
    private const val KEY_OVERLAY_X = "overlay_x"
    private const val KEY_OVERLAY_Y = "overlay_y"
    private const val KEY_DISPLAY_MODE = "caption_display_mode"
    private const val KEY_OVERLAY_WIDTH = "overlay_width_percent"
    private const val KEY_OVERLAY_HEIGHT = "overlay_height_percent"
    private const val KEY_CAPTION_ALIGNMENT = "caption_alignment"
    private const val KEY_EXPANDED_Y = "expanded_overlay_y"
    private const val KEY_EXPANDED_X = "expanded_overlay_x"

    const val PROVIDER_DASHSCOPE = "dashscope"
    const val PROVIDER_OPENAI = "openai"
    const val DEFAULT_OVERLAY_HEIGHT = 17

    @Volatile private var prefs: SharedPreferences? = null

    private fun get(context: Context): SharedPreferences {
        return prefs ?: synchronized(this) {
            prefs ?: run {
                val freshInstall = isFreshCaptionInstall(
                    java.io.File(context.applicationInfo.dataDir), context.filesDir,
                    context.getSharedPreferences("first_run", Context.MODE_PRIVATE).contains("seen"),
                )
                val masterKey = MasterKey.Builder(context)
                    .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                    .build()
                EncryptedSharedPreferences.create(
                    context, FILE, masterKey,
                    EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                    EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
                ).also {
                    if (freshInstall && !it.contains(KEY_HISTORY_LINES)) {
                        check(it.edit().putInt(KEY_HISTORY_LINES, 3).commit()) { "appearance_save_failed" }
                    }
                    prefs = it
                }
            }
        }
    }

    /** Instrumentation terminates the process immediately; wait for its restore writes first. */
    internal fun flushPendingWritesForTests(context: Context): Boolean = get(context).edit().commit()

    /** Observe only appearance; never read credential values in UI refresh callbacks. */
    internal fun observeAppearance(context: Context, onChange: () -> Unit): () -> Unit {
        val preferences = get(context)
        val keys = setOf(KEY_FONT_SIZE, KEY_OVERLAY_OPACITY, KEY_OVERLAY_BG_ALPHA,
            KEY_IMMERSIVE_SUBTITLES, KEY_HISTORY_LINES, KEY_TRANSLATION_COLOR, KEY_OVERLAY_Y,
            KEY_OVERLAY_X, KEY_EXPANDED_Y, KEY_EXPANDED_X, KEY_DISPLAY_MODE, KEY_OVERLAY_WIDTH,
            KEY_OVERLAY_HEIGHT, KEY_CAPTION_ALIGNMENT, KEY_SUBTITLE_DIVIDERS, KEY_SUBTITLE_TIMESTAMPS)
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key in keys) onChange()
        }
        preferences.registerOnSharedPreferenceChangeListener(listener)
        return { preferences.unregisterOnSharedPreferenceChangeListener(listener) }
    }

    const val MAX_SERVICE_PROFILES = ServiceProfileRepository.LIMIT
    @Volatile private var profileRepository: ServiceProfileRepository? = null

    private fun repository(context: Context): ServiceProfileRepository = synchronized(this) {
        val repository = profileRepository ?: ServiceProfileRepository(AndroidProfilePersistence(context.applicationContext))
            .also { profileRepository = it }
        if (!repository.initialized()) {
            val legacy = LegacyServiceProfiles.read(get(context).all)
            check(repository.migrate(legacy.profiles, legacy.activeId)) { "profile_migration_failed" }
        }
        check(repository.migrateXaiAsrTranslations()) { "profile_migration_failed" }
        // Retry retirement after an interrupted migration. The catalog is already durable.
        val old = get(context)
        val keys = old.all.keys.filter(LegacyServiceProfiles::ownsKey)
        if (keys.isNotEmpty()) {
            repository.profiles().forEach { repository.contents(it.id) }
            val editor = old.edit()
            keys.forEach(editor::remove)
            check(editor.commit()) { "profile_migration_cleanup_failed" }
        }
        repository
    }

    fun profiles(context: Context): List<ServiceProfile> = repository(context).profiles()
    fun activeProfileId(context: Context): String? = repository(context).activeId()
    fun activeProfile(context: Context): ServiceProfile? {
        val repository = repository(context)
        return repository.profiles().firstOrNull { it.id == repository.activeId() }
    }
    fun profileConfiguration(context: Context, id: String): app.yuxino.mimi.android.provider.ServiceConfiguration =
        repository(context).contents(id).speech
    fun profileTextTranslationProvider(context: Context, id: String): TextTranslationProvider =
        repository(context).contents(id).withIndependentXaiTranslation().effectiveTranslation
    fun profileQwenMtModel(context: Context, id: String): String = repository(context).contents(id).qwenMtModel
    fun profileTranslationConfiguration(context: Context, id: String, provider: TextTranslationProvider? = null): TranslationConfiguration {
        val contents = repository(context).contents(id).withIndependentXaiTranslation()
        val selected = provider ?: contents.effectiveTranslation
        return contents.translations[selected] ?: TranslationConfiguration(provider = selected)
    }
    fun profileConfigured(context: Context, id: String): Boolean {
        val contents = repository(context).contents(id)
        val config = contents.speech
        if (!config.provider.configured(config.credentials, config.speechTranslationMode) ||
            config.usesAlibabaLiveTranslate && config.alibabaWorkspaceId.isBlank()) return false
        if (config.provider.supportsSpeechTranslationMode && config.usesIndependentTextTranslation &&
            !config.originalOnly(contents.effectiveTranslation) && targetLang(context) != "original") {
            return runCatching {
                val translation = contents.translations[contents.effectiveTranslation]
                    ?: TranslationConfiguration(provider = contents.effectiveTranslation)
                validateTranslationConfiguration(translation)
                true
            }.getOrDefault(false)
        }
        return true
    }

    private fun canEditProfiles() = !app.yuxino.mimi.android.capture.MimiService.isStarting &&
        !app.yuxino.mimi.android.capture.MimiService.isRunning &&
        !app.yuxino.mimi.android.capture.MimiService.isStopping

    @Synchronized fun saveProfile(context: Context, id: String?, name: String,
        config: app.yuxino.mimi.android.provider.ServiceConfiguration,
        translation: TranslationConfiguration? = null, translationEnabled: Boolean? = null,
        qwenMtModel: String? = null, activate: Boolean = false): String? {
        if (!canEditProfiles()) return null
        return try {
            require(qwenMtModel == null || qwenMtModel in listOf("lite", "flash", "plus")) { "translation_model" }
            require(config.alibabaRealtimeMode in listOf("asrMt", "liveTranslate")) { "speech_mode" }
            require(config.xaiSpeechMode in listOf("voice", "asrMt")) { "speech_mode" }
            require(config.speechTranslationMode in listOf("simultaneous", "asrMt") &&
                (config.speechTranslationMode == "simultaneous" || config.provider.supportsSpeechTranslationMode)) { "speech_mode" }
            val repository = repository(context)
            val savedPrior = id?.let(repository::contents)
            val prior = if (config.provider == app.yuxino.mimi.android.provider.ServiceProvider.XAI && config.usesIndependentTextTranslation)
                savedPrior?.withIndependentXaiTranslation() else savedPrior
            val selected = if (translationEnabled == false) TextTranslationProvider.BUILTIN else translation?.provider
                ?: prior?.selectedTranslation ?: if (config.provider == app.yuxino.mimi.android.provider.ServiceProvider.XAI && config.usesIndependentTextTranslation)
                    TextTranslationProvider.XAI else if (config.provider.supportsSpeechTranslationMode) TextTranslationProvider.OPENAI_COMPATIBLE
                    else TextTranslationProvider.BUILTIN
            if (config.usesIndependentTextTranslation && translation != null && selected !in setOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) {
                validateTranslationConfiguration(translation)
            }
            val drafts = prior?.translations.orEmpty().toMutableMap()
            if (translation != null && translation.provider !in setOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) drafts[translation.provider] = translation
            repository.save(id, name, ProfileContents(config, selected, drafts, qwenMtModel ?: prior?.qwenMtModel ?: "lite",
                prior?.xaiIndependentTranslationMigrated ?: true), activate)
        } catch (_: Exception) { null }
    }

    fun duplicateProfile(context: Context, id: String, name: String): String? =
        if (!canEditProfiles()) null else runCatching { repository(context).duplicate(id, name) }.getOrNull()
    fun renameProfile(context: Context, id: String, name: String): Boolean =
        canEditProfiles() && runCatching { repository(context).rename(id, name) }.getOrDefault(false)
    fun deleteProfile(context: Context, id: String): Boolean =
        canEditProfiles() && runCatching { repository(context).delete(id) }.getOrDefault(false)
    fun activateProfile(context: Context, id: String): Boolean =
        canEditProfiles() && runCatching { profileConfigured(context, id) && repository(context).activate(id) }.getOrDefault(false)

    private fun matchingProfile(context: Context, provider: app.yuxino.mimi.android.provider.ServiceProvider): ServiceProfile? =
        activeProfile(context)?.takeIf { it.provider == provider } ?: profiles(context).firstOrNull { it.provider == provider }

    fun provider(context: Context): String = activeProfile(context)?.provider?.id ?: PROVIDER_DASHSCOPE

    /** Compatibility for existing instrumentation; user-facing selection uses profile IDs. */
    fun setProvider(context: Context, value: String) {
        val profile = matchingProfile(context, app.yuxino.mimi.android.provider.ServiceProvider.fromId(value)) ?: return
        check(canEditProfiles() && repository(context).activate(profile.id)) { "profile_activation_failed" }
    }

    fun apiKey(context: Context, provider: String = provider(context)): String =
        configuration(context, app.yuxino.mimi.android.provider.ServiceProvider.fromId(provider)).value("apiKey")

    fun setApiKey(context: Context, value: String, provider: String = provider(context)) {
        val saved = configuration(context, app.yuxino.mimi.android.provider.ServiceProvider.fromId(provider))
        check(saveConfiguration(context, app.yuxino.mimi.android.provider.ServiceConfiguration(saved.provider,
            saved.credentials + ("apiKey" to value.trim()), saved.endpoint, saved.model, saved.alibabaRealtimeMode, saved.alibabaWorkspaceId, saved.xaiSpeechMode, saved.xaiTextModel, saved.speechTranslationMode))) { "profile_save_failed" }
    }

    fun configuration(context: Context, provider: app.yuxino.mimi.android.provider.ServiceProvider? = null): app.yuxino.mimi.android.provider.ServiceConfiguration {
        val profile = if (provider == null) activeProfile(context) else matchingProfile(context, provider)
        return profile?.let { profileConfiguration(context, it.id) }
            ?: app.yuxino.mimi.android.provider.ServiceConfiguration(provider ?: app.yuxino.mimi.android.provider.ServiceProvider.DASHSCOPE, emptyMap())
    }

    fun isConfigured(context: Context, provider: app.yuxino.mimi.android.provider.ServiceProvider? = null): Boolean {
        val profile = if (provider == null) activeProfile(context) else matchingProfile(context, provider)
        return profile != null && profileConfigured(context, profile.id)
    }

    /** Legacy provider-keyed entry point; never used to select a production session. */
    fun saveConfiguration(context: Context, config: app.yuxino.mimi.android.provider.ServiceConfiguration,
        translation: TranslationConfiguration? = null, translationEnabled: Boolean? = null, qwenMtModel: String? = null): Boolean {
        require(qwenMtModel == null || qwenMtModel in listOf("lite", "flash", "plus")) { "translation_model" }
        if (config.usesIndependentTextTranslation && translation != null && translationEnabled != false) validateTranslationConfiguration(translation)
        val profile = matchingProfile(context, config.provider)
        return saveProfile(context, profile?.id, profile?.name ?: config.provider.title, config, translation, translationEnabled, qwenMtModel) != null
    }

    fun textTranslationProvider(context: Context): TextTranslationProvider =
        activeProfileId(context)?.let { profileTextTranslationProvider(context, it) } ?: TextTranslationProvider.BUILTIN

    fun translationSelectionLocked(context: Context): Boolean =
        configuration(context).originalOnly(textTranslationProvider(context))

    fun originalTextOnly(context: Context): Boolean {
        val speech = configuration(context)
        return translationSelectionLocked(context) || speech.usesIndependentTextTranslation && targetLang(context) == "original"
    }

    fun useChatMockTranslation(context: Context): Boolean = textTranslationProvider(context) == TextTranslationProvider.CHAT_MOCK
    fun qwenMtModel(context: Context): String = activeProfileId(context)?.let { profileQwenMtModel(context, it) } ?: "lite"

    /** Private, short-lived native session input. Never include it in UI snapshots or logs. */
    fun runtimeConfiguration(context: Context): JSONObject {
        // One atomic profile read: a concurrent selection can never pair one profile
        // speech key with another profile translator or model.
        val contents = repository(context).activeContents() ?: throw IllegalStateException("profile_missing")
        val speech = contents.speech
        val translation = if (speech.usesIndependentTextTranslation)
            contents.translations[contents.effectiveTranslation] ?: TranslationConfiguration(provider = contents.effectiveTranslation)
            else TranslationConfiguration(provider = TextTranslationProvider.BUILTIN)
        val (source, target) = speech.provider.normalize(sourceLang(context), targetLang(context), translation.provider, speech.alibabaRealtimeMode, speech.xaiSpeechMode, speech.speechTranslationMode)
        return buildRuntimeConfiguration(speech, translation, source, target, contents.qwenMtModel)
    }

    fun translationConfiguration(context: Context, provider: TextTranslationProvider = textTranslationProvider(context)): TranslationConfiguration =
        activeProfileId(context)?.let { profileTranslationConfiguration(context, it, provider) } ?: TranslationConfiguration(provider = provider)

    fun activateProvider(context: Context, provider: app.yuxino.mimi.android.provider.ServiceProvider): Boolean =
        matchingProfile(context, provider)?.let { activateProfile(context, it.id) } ?: false

    fun baseUrl(context: Context, provider: String): String = configuration(context, app.yuxino.mimi.android.provider.ServiceProvider.fromId(provider)).endpoint
    fun setBaseUrl(context: Context, provider: String, value: String) {
        val saved = configuration(context, app.yuxino.mimi.android.provider.ServiceProvider.fromId(provider))
        check(saveConfiguration(context, app.yuxino.mimi.android.provider.ServiceConfiguration(saved.provider, saved.credentials,
            value.trim(), saved.model, saved.alibabaRealtimeMode, saved.alibabaWorkspaceId, saved.xaiSpeechMode, saved.xaiTextModel, saved.speechTranslationMode))) { "profile_save_failed" }
    }
    fun model(context: Context, provider: String): String = configuration(context, app.yuxino.mimi.android.provider.ServiceProvider.fromId(provider)).model
    fun setModel(context: Context, provider: String, value: String) {
        val saved = configuration(context, app.yuxino.mimi.android.provider.ServiceProvider.fromId(provider))
        check(saveConfiguration(context, app.yuxino.mimi.android.provider.ServiceConfiguration(saved.provider, saved.credentials,
            saved.endpoint, value.trim(), saved.alibabaRealtimeMode, saved.alibabaWorkspaceId, saved.xaiSpeechMode, saved.xaiTextModel, saved.speechTranslationMode))) { "profile_save_failed" }
    }

    /** Overlay window opacity, 20..100 (percent). */
    fun overlayOpacity(context: Context): Int =
        get(context).getInt(KEY_OVERLAY_OPACITY, 100).coerceIn(20, 100)

    fun setOverlayOpacity(context: Context, value: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_OPACITY, value.coerceIn(20, 100)))

    /** New installs start with three confirmed captions; upgrades keep the old zero fallback. */
    fun historyLines(context: Context): Int =
        get(context).getInt(KEY_HISTORY_LINES, 0).coerceIn(0, 6)

    fun setHistoryLines(context: Context, value: Int) {
        val limit = value.coerceIn(0, 6)
        writeAppearance(get(context).edit().putInt(KEY_HISTORY_LINES, limit))
        app.yuxino.mimi.android.provider.SubtitleBus.setHistoryLimit(limit)
    }

    fun captureInput(context: Context): CaptureInput = CaptureInput.entries.firstOrNull {
        it.id == get(context).getString(KEY_CAPTURE_INPUT, null)
    } ?: CaptureInput.SYSTEM

    fun setCaptureInput(context: Context, input: CaptureInput) {
        if (captureInput(context) == input) return
        check(canEditProfiles()) { "capture_input_change_requires_stop" }
        check(get(context).edit().putString(KEY_CAPTURE_INPUT, input.id).commit()) { "capture_input_save_failed" }
    }

    /** Durable caption saving is separate from the bounded in-memory overlay history. */
    fun retainSessionHistory(context: Context): Boolean = get(context).getBoolean(KEY_RETAIN_SESSION_HISTORY, false)

    fun setRetainSessionHistory(context: Context, enabled: Boolean) {
        if (retainSessionHistory(context) == enabled) return
        check(!enabled || canEditProfiles()) { "session_history_enable_requires_stop" }
        check(get(context).edit().putBoolean(KEY_RETAIN_SESSION_HISTORY, enabled).commit()) { "session_history_save_failed" }
    }

    internal fun observeSessionStorage(context: Context, onChange: () -> Unit): () -> Unit {
        val preferences = get(context)
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key == KEY_RETAIN_SESSION_HISTORY) onChange()
        }
        preferences.registerOnSharedPreferenceChangeListener(listener)
        return { preferences.unregisterOnSharedPreferenceChangeListener(listener) }
    }

    fun subtitleDividers(context: Context): Boolean = get(context).getBoolean(KEY_SUBTITLE_DIVIDERS, false)
    fun setSubtitleDividers(context: Context, value: Boolean) = writeAppearance(get(context).edit().putBoolean(KEY_SUBTITLE_DIVIDERS, value))
    fun subtitleTimestamps(context: Context): Boolean = get(context).getBoolean(KEY_SUBTITLE_TIMESTAMPS, false)
    fun setSubtitleTimestamps(context: Context, value: Boolean) = writeAppearance(get(context).edit().putBoolean(KEY_SUBTITLE_TIMESTAMPS, value))

    /**
     * Hotword pairs parsed from comma/newline-separated `source=translation` or bare
     * `term` entries; bare terms map to themselves. Only DashScope uses them.
     */
    fun hotwords(context: Context): Map<String, String> {
        val raw = get(context).getString(KEY_HOTWORDS, "") ?: ""
        return raw.split(',', '\n', '，')
            .mapNotNull { entry ->
                val item = entry.trim()
                if (item.isEmpty()) return@mapNotNull null
                val idx = item.indexOf('=')
                if (idx > 0) item.substring(0, idx).trim() to item.substring(idx + 1).trim()
                else item to item
            }
            .toMap()
    }

    fun hotwordsText(context: Context): String =
        get(context).getString(KEY_HOTWORDS, "") ?: ""

    fun setHotwords(context: Context, value: String) =
        get(context).edit().putString(KEY_HOTWORDS, value.trim()).apply()

    /** Pure black subtitle background, matching desktop: 0 (clear) .. 100 (solid). */
    fun overlayBgAlpha(context: Context): Int =
        get(context).getInt(KEY_OVERLAY_BG_ALPHA, 80).coerceIn(0, 100)

    fun setOverlayBgAlpha(context: Context, value: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_BG_ALPHA, value.coerceIn(0, 100)))

    /** Separate from immersive presentation: locking never changes caption geometry or style. */
    fun overlayAdjustable(context: Context): Boolean = get(context).getBoolean(KEY_OVERLAY_ADJUSTABLE, true)

    fun setOverlayAdjustable(context: Context, enabled: Boolean) {
        val preferences = get(context)
        val previous = overlayAdjustable(context)
        if (!preferences.edit().putBoolean(KEY_OVERLAY_ADJUSTABLE, enabled).commit()) {
            // SharedPreferences changes memory even when its disk write fails.
            // Restore that value too, so a later refresh cannot silently lock it.
            preferences.edit().putBoolean(KEY_OVERLAY_ADJUSTABLE, previous).apply()
            error("appearance_save_failed")
        }
    }

    /** Plain, touch-through subtitles; appearance updates the active views immediately. */
    fun immersiveSubtitles(context: Context): Boolean =
        get(context).getBoolean(KEY_IMMERSIVE_SUBTITLES, false)

    fun setImmersiveSubtitles(context: Context, enabled: Boolean) =
        writeAppearance(get(context).edit().putBoolean(KEY_IMMERSIVE_SUBTITLES, enabled))

    /** Last overlay vertical offset from the bottom edge, persisted. */
    fun overlayYOffset(context: Context): Int =
        get(context).getInt(KEY_OVERLAY_Y, 48).coerceIn(0, 2000)

    fun setOverlayYOffset(context: Context, y: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_Y, y.coerceIn(0, 2000)))

    /** Clears a persisted drag position so the card returns to bottom-center. */
    fun clearOverlayPosition(context: Context) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_Y, 48).putInt(KEY_OVERLAY_X, 0)
            .putInt(KEY_EXPANDED_Y, 48).putInt(KEY_EXPANDED_X, 0))

    private fun writeAppearance(editor: android.content.SharedPreferences.Editor) {
        check(editor.commit()) { "Appearance preference write failed" }
    }

    fun displayMode(context: Context): CaptionDisplayMode =
        CaptionDisplayMode.fromId(get(context).getString(KEY_DISPLAY_MODE, null))
    fun setDisplayMode(context: Context, mode: CaptionDisplayMode) =
        writeAppearance(get(context).edit().putString(KEY_DISPLAY_MODE, mode.id))
    fun overlayWidthPercent(context: Context): Int = get(context).getInt(KEY_OVERLAY_WIDTH, 92).coerceIn(55, 100)
    fun setOverlayWidthPercent(context: Context, value: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_WIDTH, value.coerceIn(55, 100)))
    fun overlayHeightPercent(context: Context): Int = get(context).getInt(KEY_OVERLAY_HEIGHT, DEFAULT_OVERLAY_HEIGHT).coerceIn(10, 70)
    fun setOverlayHeightPercent(context: Context, value: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_HEIGHT, value.coerceIn(10, 70)))
    fun setOverlayBounds(context: Context, width: Int, height: Int, x: Int, y: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_WIDTH, width.coerceIn(55, 100))
            .putInt(KEY_OVERLAY_HEIGHT, height.coerceIn(10, 70))
            .putInt(KEY_EXPANDED_X, x.coerceIn(-2000, 2000)).putInt(KEY_EXPANDED_Y, y.coerceIn(0, 2000)))
    fun captionAlignment(context: Context): CaptionAlignment = CaptionAlignment.fromId(get(context).getString(KEY_CAPTION_ALIGNMENT, null))
    fun setCaptionAlignment(context: Context, alignment: CaptionAlignment) =
        writeAppearance(get(context).edit().putString(KEY_CAPTION_ALIGNMENT, alignment.id))
    fun expandedYOffset(context: Context): Int = get(context).getInt(KEY_EXPANDED_Y, 48).coerceIn(0, 2000)
    fun overlayXOffset(context: Context, expanded: Boolean): Int =
        get(context).getInt(if (expanded) KEY_EXPANDED_X else KEY_OVERLAY_X, 0).coerceIn(-2000, 2000)
    fun setOverlayPosition(context: Context, expanded: Boolean, x: Int, y: Int) =
        writeAppearance(get(context).edit()
            .putInt(if (expanded) KEY_EXPANDED_X else KEY_OVERLAY_X, x.coerceIn(-2000, 2000))
            .putInt(if (expanded) KEY_EXPANDED_Y else KEY_OVERLAY_Y, y.coerceIn(0, 2000)))

    /** Preset translation colors, index-matched to the settings spinner. */
    val COLOR_PRESETS = listOf(0xFF4FD1C5, 0xFFFFFFFF, 0xFFFFD54F, 0xFF9AE66E, 0xFFF49AB5)

    fun translationColor(context: Context): Int {
        val index = get(context).getInt(KEY_TRANSLATION_COLOR, 1)
            .coerceIn(0, COLOR_PRESETS.size - 1)
        return COLOR_PRESETS[index].toInt()
    }

    fun translationColorIndex(context: Context): Int =
        get(context).getInt(KEY_TRANSLATION_COLOR, 1)
            .coerceIn(0, COLOR_PRESETS.size - 1)

    fun setTranslationColorIndex(context: Context, index: Int) =
        writeAppearance(get(context).edit()
            .putInt(KEY_TRANSLATION_COLOR, index.coerceIn(0, COLOR_PRESETS.size - 1)))

    fun sourceLang(context: Context): String =
        get(context).getString(KEY_SOURCE_LANG, "auto") ?: "auto"

    fun setSourceLang(context: Context, value: String) =
        get(context).edit().putString(KEY_SOURCE_LANG, value).apply()

    fun targetLang(context: Context): String =
        get(context).getString(KEY_TARGET_LANG, "zh") ?: "zh"

    fun setTargetLang(context: Context, value: String) =
        get(context).edit().putString(KEY_TARGET_LANG, value).apply()

    fun fontSize(context: Context): Int =
        get(context).getInt(KEY_FONT_SIZE, 16).coerceIn(12, 24)

    fun setFontSize(context: Context, value: Int) =
        writeAppearance(get(context).edit().putInt(KEY_FONT_SIZE, value.coerceIn(12, 24)))
}
