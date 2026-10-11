package app.yuxino.mimi.android

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.provider.validateTranslationConfiguration
import app.yuxino.mimi.android.provider.buildRuntimeConfiguration
import app.yuxino.mimi.android.provider.normalizeQwenMTModel
import org.json.JSONObject

/** EncryptedSharedPreferences-backed settings (API key stays in Android Keystore-backed storage). */
object SettingsStore {
    private const val FILE = "mimi_secure"

    private const val KEY_PROVIDER = "provider"
    private const val KEY_API_KEY = "api_key"
    private const val KEY_SOURCE_LANG = "source_lang"
    private const val KEY_TARGET_LANG = "target_lang"
    private const val KEY_FONT_SIZE = "font_size"
    private const val KEY_BASE_URL_PREFIX = "base_url_"
    private const val KEY_MODEL_PREFIX = "model_"
    private const val KEY_QWEN_MT_MODEL = "qwen_mt_model_dashscope"
    private const val KEY_OVERLAY_OPACITY = "overlay_opacity"
    private const val KEY_OVERLAY_BG_ALPHA = "overlay_bg_alpha"
    private const val KEY_IMMERSIVE_SUBTITLES = "immersive_subtitles"
    private const val KEY_HISTORY_LINES = "history_lines"
    private const val KEY_HOTWORDS = "hotwords"
    private const val KEY_TRANSLATION_COLOR = "translation_color"
    private const val KEY_OVERLAY_X = "overlay_x"
    private const val KEY_OVERLAY_Y = "overlay_y"
    private const val KEY_OVERLAY_SAVED = "overlay_saved"
    private const val KEY_DISPLAY_MODE = "caption_display_mode"
    private const val KEY_OVERLAY_WIDTH = "overlay_width_percent"
    private const val KEY_OVERLAY_HEIGHT = "overlay_height_percent"
    private const val KEY_CAPTION_ALIGNMENT = "caption_alignment"
    private const val KEY_EXPANDED_Y = "expanded_overlay_y"
    private const val KEY_EXPANDED_X = "expanded_overlay_x"

    const val PROVIDER_DASHSCOPE = "dashscope"
    const val PROVIDER_OPENAI = "openai"

    @Volatile private var prefs: SharedPreferences? = null

    private fun get(context: Context): SharedPreferences {
        return prefs ?: synchronized(this) {
            prefs ?: run {
                val masterKey = MasterKey.Builder(context)
                    .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                    .build()
                EncryptedSharedPreferences.create(
                    context, FILE, masterKey,
                    EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                    EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
                ).also { prefs = it }
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
            KEY_OVERLAY_HEIGHT, KEY_CAPTION_ALIGNMENT)
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key in keys) onChange()
        }
        preferences.registerOnSharedPreferenceChangeListener(listener)
        return { preferences.unregisterOnSharedPreferenceChangeListener(listener) }
    }

    fun provider(context: Context): String =
        get(context).getString(KEY_PROVIDER, PROVIDER_DASHSCOPE) ?: PROVIDER_DASHSCOPE

    fun setProvider(context: Context, value: String) =
        get(context).edit().putString(KEY_PROVIDER, value).apply()

    fun apiKey(context: Context, provider: String = provider(context)): String {
        val prefs = get(context)
        // Migrate the original shared key only to the provider it belonged to.
        synchronized(this) {
            if (prefs.contains(KEY_API_KEY)) {
                val owner = this.provider(context)
                val scopedKey = KEY_API_KEY + "_" + owner
                val editor = prefs.edit()
                if (!prefs.contains(scopedKey)) {
                    editor.putString(scopedKey, prefs.getString(KEY_API_KEY, ""))
                }
                editor.remove(KEY_API_KEY).apply()
            }
            return prefs.getString(KEY_API_KEY + "_" + provider, "") ?: ""
        }
    }

    fun setApiKey(context: Context, value: String, provider: String = provider(context)) {
        apiKey(context, provider) // Complete any legacy migration first.
        get(context).edit().putString(KEY_API_KEY + "_" + provider, value.trim()).apply()
    }

    fun configuration(context: Context, provider: app.yuxino.mimi.android.provider.ServiceProvider =
        app.yuxino.mimi.android.provider.ServiceProvider.fromId(provider(context))): app.yuxino.mimi.android.provider.ServiceConfiguration {
        val values = provider.fields.associate { field -> field.id to if (field.id == "apiKey")
            apiKey(context, provider.id) else get(context).getString("credential_${provider.id}_${field.id}", "").orEmpty() }
        return app.yuxino.mimi.android.provider.ServiceConfiguration(provider, values, baseUrl(context, provider.id), model(context, provider.id))
    }

    fun isConfigured(context: Context, provider: app.yuxino.mimi.android.provider.ServiceProvider =
        app.yuxino.mimi.android.provider.ServiceProvider.fromId(provider(context))): Boolean =
        provider.configured(configuration(context, provider).credentials)

    /** Save one complete profile atomically; callers preserve blank, write-only secret fields. */
    fun saveConfiguration(context: Context, config: app.yuxino.mimi.android.provider.ServiceConfiguration,
        translation: app.yuxino.mimi.android.provider.TranslationConfiguration? = null,
        translationEnabled: Boolean? = null, qwenMtModel: String? = null): Boolean {
        require(qwenMtModel == null || qwenMtModel in listOf("lite", "flash", "plus")) { "translation_model" }
        apiKey(context, config.provider.id) // Resolve legacy ownership before activating another provider.
        val editor = get(context).edit()
        config.provider.fields.forEach { field ->
            val key = if (field.id == "apiKey") "${KEY_API_KEY}_${config.provider.id}" else "credential_${config.provider.id}_${field.id}"
            editor.putString(key, config.value(field.id).trim())
        }
        editor.putString(KEY_BASE_URL_PREFIX + config.provider.id, config.endpoint.trim())
        editor.putString(KEY_MODEL_PREFIX + config.provider.id, config.model.trim())
        if (config.provider == app.yuxino.mimi.android.provider.ServiceProvider.DASHSCOPE && qwenMtModel != null) {
            editor.putString(KEY_QWEN_MT_MODEL, qwenMtModel)
        }
        if (config.provider == app.yuxino.mimi.android.provider.ServiceProvider.DASHSCOPE && translation != null) {
            val selected = if (translationEnabled == false) TextTranslationProvider.BUILTIN else translation.provider
            if (selected != TextTranslationProvider.BUILTIN) validateTranslationConfiguration(translation)
            // Copy the old ChatMock draft once, even when saving a different translator first.
            // Reads do not migrate or enable anything; Save and use is the only write boundary.
            val preferences = get(context)
            val openAIPrefix = translationPrefix(TextTranslationProvider.OPENAI_COMPATIBLE)
            if (!preferences.contains(openAIPrefix + "endpoint") && preferences.contains("chatmock_endpoint")) {
                writeTranslation(editor, translationConfiguration(context, TextTranslationProvider.OPENAI_COMPATIBLE))
            }
            if (translation.provider !in setOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) {
                writeTranslation(editor, translation)
            }
            editor.putString("text_translation_provider", selected.storageId)
            listOf("chatmock_enabled", "chatmock_endpoint", "chatmock_model", "chatmock_api_key", "chatmock_local_http")
                .forEach(editor::remove)
        }
        return editor.commit()
    }

    fun textTranslationProvider(context: Context): TextTranslationProvider {
        val preferences = get(context)
        val stored = preferences.getString("text_translation_provider", null)
        if (stored != null) return TextTranslationProvider.fromStorageId(stored)
        return if (preferences.getBoolean("chatmock_enabled", false)) TextTranslationProvider.OPENAI_COMPATIBLE
            else TextTranslationProvider.BUILTIN
    }

    fun originalTextOnly(context: Context): Boolean =
        provider(context) == PROVIDER_DASHSCOPE && textTranslationProvider(context) == TextTranslationProvider.NONE

    fun useChatMockTranslation(context: Context): Boolean =
        textTranslationProvider(context) == TextTranslationProvider.CHAT_MOCK

    /** A distinct text-model preference; legacy realtime model overrides remain untouched. */
    fun qwenMtModel(context: Context): String = normalizeQwenMTModel(get(context).getString(KEY_QWEN_MT_MODEL, null))

    /** Private, short-lived native session input. Never include it in UI snapshots or logs. */
    fun runtimeConfiguration(context: Context): JSONObject {
        val speech = configuration(context)
        val translation = if (speech.provider == app.yuxino.mimi.android.provider.ServiceProvider.DASHSCOPE)
            translationConfiguration(context) else TranslationConfiguration(provider = TextTranslationProvider.BUILTIN)
        // Normalize only this effective session input. Keep the saved language preference intact.
        val (source, target) = speech.provider.normalize(sourceLang(context), targetLang(context), translation.provider)
        return buildRuntimeConfiguration(speech, translation, source, target, qwenMtModel(context))
    }

    fun translationConfiguration(context: Context, provider: TextTranslationProvider = textTranslationProvider(context)): TranslationConfiguration {
        if (provider in setOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE)) {
            return TranslationConfiguration(provider = provider)
        }
        val preferences = get(context)
        val prefix = translationPrefix(provider)
        // The old combined entry could hold any compatible service. Keep it generic;
        // never infer ChatMock from its URL or copy its key into the new ChatMock entry.
        val legacy = provider == TextTranslationProvider.OPENAI_COMPATIBLE && !preferences.contains(prefix + "endpoint")
        return TranslationConfiguration(
            endpoint = preferences.getString(if (legacy) "chatmock_endpoint" else prefix + "endpoint", "").orEmpty(),
            model = preferences.getString(if (legacy) "chatmock_model" else prefix + "model", "").orEmpty(),
            apiKey = preferences.getString(if (legacy) "chatmock_api_key" else prefix + "api_key", "").orEmpty(),
            allowLocalHttp = preferences.getBoolean(if (legacy) "chatmock_local_http" else prefix + "local_http", false),
            provider = provider,
        )
    }

    private fun translationPrefix(provider: TextTranslationProvider) = "text_translation_${provider.storageId}_"

    private fun writeTranslation(editor: SharedPreferences.Editor, configuration: TranslationConfiguration) {
        val prefix = translationPrefix(configuration.provider)
        editor.putString(prefix + "endpoint", configuration.endpoint)
            .putString(prefix + "model", configuration.model)
            .putString(prefix + "api_key", configuration.apiKey)
            .putBoolean(prefix + "local_http", configuration.allowLocalHttp)
    }

    fun activateProvider(context: Context, provider: app.yuxino.mimi.android.provider.ServiceProvider): Boolean {
        if (!isConfigured(context, provider)) return false
        val (source, target) = provider.normalize(sourceLang(context), targetLang(context), textTranslationProvider(context))
        return get(context).edit().putString(KEY_PROVIDER, provider.id)
            .putString(KEY_SOURCE_LANG, source).putString(KEY_TARGET_LANG, target).commit()
    }

    /** Custom endpoint per provider; blank means the official endpoint. */
    fun baseUrl(context: Context, provider: String): String =
        get(context).getString(KEY_BASE_URL_PREFIX + provider, "") ?: ""

    fun setBaseUrl(context: Context, provider: String, value: String) =
        get(context).edit().putString(KEY_BASE_URL_PREFIX + provider, value.trim()).apply()

    /** Optional model override per provider; blank means the engine's default. */
    fun model(context: Context, provider: String): String =
        get(context).getString(KEY_MODEL_PREFIX + provider, "") ?: ""

    fun setModel(context: Context, provider: String, value: String) =
        get(context).edit().putString(KEY_MODEL_PREFIX + provider, value.trim()).apply()

    /** Overlay window opacity, 20..100 (percent). */
    fun overlayOpacity(context: Context): Int =
        get(context).getInt(KEY_OVERLAY_OPACITY, 100).coerceIn(20, 100)

    fun setOverlayOpacity(context: Context, value: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_OPACITY, value.coerceIn(20, 100)))

    /** History retention is opt-in; zero clears retained content immediately. */
    fun historyLines(context: Context): Int =
        get(context).getInt(KEY_HISTORY_LINES, 0).coerceIn(0, 6)

    fun setHistoryLines(context: Context, value: Int) {
        val limit = value.coerceIn(0, 6)
        writeAppearance(get(context).edit().putInt(KEY_HISTORY_LINES, limit))
        app.yuxino.mimi.android.provider.SubtitleBus.setHistoryLimit(limit)
    }

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
            .putInt(KEY_EXPANDED_Y, 0).putInt(KEY_EXPANDED_X, 0))

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
    fun overlayHeightPercent(context: Context): Int = get(context).getInt(KEY_OVERLAY_HEIGHT, 30).coerceIn(20, 70)
    fun setOverlayHeightPercent(context: Context, value: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_HEIGHT, value.coerceIn(20, 70)))
    fun setOverlayBounds(context: Context, width: Int, height: Int, x: Int, y: Int) =
        writeAppearance(get(context).edit().putInt(KEY_OVERLAY_WIDTH, width.coerceIn(55, 100))
            .putInt(KEY_OVERLAY_HEIGHT, height.coerceIn(20, 70))
            .putInt(KEY_EXPANDED_X, x.coerceIn(-2000, 2000)).putInt(KEY_EXPANDED_Y, y.coerceIn(0, 2000)))
    fun captionAlignment(context: Context): CaptionAlignment = CaptionAlignment.fromId(get(context).getString(KEY_CAPTION_ALIGNMENT, null))
    fun setCaptionAlignment(context: Context, alignment: CaptionAlignment) =
        writeAppearance(get(context).edit().putString(KEY_CAPTION_ALIGNMENT, alignment.id))
    fun expandedYOffset(context: Context): Int = get(context).getInt(KEY_EXPANDED_Y, 0).coerceIn(0, 2000)
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
