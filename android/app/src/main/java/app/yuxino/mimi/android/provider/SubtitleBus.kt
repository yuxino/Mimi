package app.yuxino.mimi.android.provider

import org.json.JSONObject

/** Native UI facade over the exact Rust reducer used by desktop. */
object SubtitleBus {
    data class Pair(val source: String, val translation: String)
    private val lock = Any()
    private val listeners = mutableListOf<Listener>()
    private var state: String? = null
    @Volatile private var snapshot = JSONObject()
    private var historyLimit = 0
    private var sourceId = 0L
    private var confirmationId = 0L
    private var draftId: Long? = null
    private var originalOnly = false
    private var runtimeOwned = false
    private var atomicPreview = false

    val sourceDraft: String get() = line("source", false)
    val sourceFinal: String get() = line("source", true)
    val translationDraft: String get() = if (originalOnly) "" else line("translation", false)
    val translationFinal: String get() = if (originalOnly) "" else line("translation", true)
    /** Shared realtime projection advances drafts without mixing sentence owners. */
    private fun realtimeText(role: String): String? = if (atomicPreview) null else snapshot.optJSONObject("realtimePreview")?.getJSONObject(role)?.getString("text")
    val displaySource: String get() = if (originalOnly) sourceDraft.ifEmpty { sourceFinal }
        else realtimeText("source") ?: snapshot.optJSONObject("displayPair")?.optString("source") ?: sourceDraft.ifEmpty { sourceFinal }
    val displayTranslation: String get() = if (originalOnly) "" else
        realtimeText("translation") ?: snapshot.optJSONObject("displayPair")?.optString("translation") ?: translationDraft.ifEmpty { translationFinal }
    val displayPairFinal: Boolean get() = (atomicPreview || snapshot.optJSONObject("realtimePreview") == null) && snapshot.optBoolean("displayPairFinal")
    @Volatile var statusLine = ""
    @Volatile var liveHidden = false
    @Volatile var detectedSourceLanguage: String? = null
    const val MAX_HISTORY = 6

    interface Listener { fun onSubtitleChanged() }
    fun addListener(listener: Listener) = synchronized(lock) { listeners.add(listener); Unit }
    fun removeListener(listener: Listener) = synchronized(lock) { listeners.remove(listener); Unit }
    private fun notifyListeners() {
        val copy = synchronized(lock) { listeners.toList() }
        copy.forEach { try { it.onSubtitleChanged() } catch (_: Exception) { } }
    }
    private fun line(role: String, final: Boolean): String {
        val value = snapshot.optJSONObject(role) ?: return ""
        return if (value.optBoolean("isFinal") == final) value.optString("text") else ""
    }
    private fun exchange(operation: JSONObject) {
        val response = SharedSubtitleCore.exchange(JSONObject().put("state", state).put("operation", operation))
        state = response.getString("state")
        snapshot = response.getJSONObject("snapshot")
    }
    private fun ensureCreated() {
        if (state == null) exchange(JSONObject().put("type", "create").put("history_limit", historyLimit))
    }
    fun onCoreEvent(event: JSONObject, language: String? = null) {
        synchronized(lock) {
            ensureCreated()
            exchange(JSONObject().put("type", "apply").put("event", event))
            val identified = event.optString("type") == "identified_final_pair" || event.optString("type") == "utterance_text"
            val ownsSource = !identified || snapshot.optJSONObject("source")?.optString("utteranceId") == event.optString("utterance_id")
            if (ownsSource) detectedSourceLanguage = language?.trim()?.lowercase()?.takeIf { it.length in 2..64 } ?: detectedSourceLanguage
            liveHidden = false
        }
        notifyListeners()
    }
    /** The shared session controller has already reduced the provider event. */
    fun onRuntimeSnapshot(value: JSONObject, sourceOnly: Boolean) {
        synchronized(lock) {
            runtimeOwned = true
            atomicPreview = value.optBoolean("atomicPreview")
            snapshot = value.getJSONObject("snapshot")
            originalOnly = sourceOnly
            detectedSourceLanguage = value.optString("detectedLanguage").takeUnless { it.isBlank() || it == "null" }
            liveHidden = false
        }
        notifyListeners()
    }
    private fun textEvent(type: String, text: String) = JSONObject().put("type", type).put("text", text)
    fun onSourceDraft(text: String, language: String? = null) = onCoreEvent(textEvent("source_draft", text), language)
    fun onSourceFinal(text: String, language: String? = null) = onCoreEvent(textEvent("source_final", text), language)
    fun onTranslationDraft(text: String) = onCoreEvent(textEvent("translation_draft", text))
    fun onTranslationFinal(text: String) = onCoreEvent(textEvent("translation_final", text))
    fun onFinalPair(source: String, translation: String, language: String? = null) = onCoreEvent(
        JSONObject().put("type", "final_pair").put("source", source).put("translation", translation), language)
    fun onIdentifiedFinalPair(id: String, source: String, translation: String, language: String? = null) = onCoreEvent(
        JSONObject().put("type", "identified_final_pair").put("utterance_id", id).put("source", source).put("translation", translation), language)

    /** The returned identity travels with the HTTP request, including repeated text. */
    fun onUntranslatedSource(text: String, language: String?, final: Boolean): Long {
        val id = synchronized(lock) {
            originalOnly = false
            val current = draftId ?: (++sourceId).also { draftId = it }
            if (final) draftId = null
            current
        }
        onCoreEvent(JSONObject().put("type", "source_utterance_draft").put("utterance_id", id).put("text", text), language)
        return id
    }
    fun onTranslatedSource(source: String, language: String?, translation: String, sourceUtteranceId: Long? = null) {
        val id = synchronized(lock) { originalOnly = false; ++confirmationId }
        val ownedLanguage = synchronized(lock) { if (sourceUtteranceId == null || sourceUtteranceId >= sourceId) language else null }
        onCoreEvent(JSONObject().put("type", "confirmed_pair").put("utterance_id", id)
            .put("source_utterance_id", sourceUtteranceId ?: JSONObject.NULL)
            .put("source", source).put("translation", translation), ownedLanguage)
    }
    /** Original-only uses the same atomic confirmation as desktop; rendering hides the second lane. */
    fun onOriginalSource(source: String, language: String?) {
        synchronized(lock) { originalOnly = true; draftId = null }
        onFinalPair(source, source, language)
    }
    fun setHistoryLimit(limit: Int) {
        synchronized(lock) {
            historyLimit = limit.coerceIn(0, MAX_HISTORY)
            if (!runtimeOwned) {
                ensureCreated()
                exchange(JSONObject().put("type", "history_limit").put("limit", historyLimit))
            }
        }
        notifyListeners()
    }
    fun historySnapshot(): List<Pair> = synchronized(lock) {
        val rows = snapshot.optJSONArray("history") ?: return@synchronized emptyList()
        val count = minOf(rows.length(), historyLimit)
        List(count) { offset ->
            val index = rows.length() - count + offset
            val row = rows.getJSONObject(index)
            val source = row.getString("source")
            val translation = row.getString("translation")
            Pair(source, if (originalOnly && source == translation) "" else translation)
        }
    }
    fun onStatus(line: String) { statusLine = line; notifyListeners() }
    fun hideLive() {
        synchronized(lock) {
            // The shared current complete pair stays readable until replaced
            // or the session ends, even when saved history is disabled.
            if (snapshot.optJSONObject("displayPair") != null) return
            liveHidden = true
        }
        notifyListeners()
    }
    fun clear() {
        synchronized(lock) {
            state = null
            runtimeOwned = false
            atomicPreview = false
            snapshot = JSONObject()
            sourceId = 0; confirmationId = 0; draftId = null; originalOnly = false
            statusLine = ""; detectedSourceLanguage = null; liveHidden = false
        }
        notifyListeners()
    }
}
