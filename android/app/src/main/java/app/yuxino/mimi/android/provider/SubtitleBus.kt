package app.yuxino.mimi.android.provider

import org.json.JSONObject
import org.json.JSONArray

/** Native UI facade over the exact Rust reducer used by desktop. */
object SubtitleBus {
    data class Pair(val source: String, val translation: String, val audioSource: String = "system")
    data class TimedPair(val pair: Pair, val createdAt: Long?)
    data class RuntimeFeedback(val connectionStatus: String = "idle", val recoveryReason: String? = null,
        val retryScheduled: Boolean = false, val translationTimedOut: Boolean = false)
    data class SourceReading(val text: String, val confirmed: Boolean, val createdAt: Long?)
    data class DisplaySnapshot(val source: String, val translation: String, val hidden: Boolean,
        val confirmed: Boolean, val createdAt: Long?, val history: List<TimedPair>,
        val status: String, val feedback: RuntimeFeedback, val retainedConfirmed: TimedPair? = null,
        val currentSource: SourceReading = SourceReading(source, confirmed, createdAt),
        val audioSource: String = "system")
    @Volatile var runtimeFeedback = RuntimeFeedback()
        private set
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
    private val runtimeSnapshots = linkedMapOf<String, JSONObject>()
    private var runtimeStopped = false

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
    val displayPairCreatedAt: Long? get() = if (displayPairFinal)
        snapshot.optLong("displayPairCreatedAt").takeIf { it > 0 } else null
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
    /** Each native handle has already reduced its own source. Rust combines their bounded presentation. */
    fun onRuntimeSnapshot(value: JSONObject, sourceOnly: Boolean, audioSource: String = "system") {
        require(audioSource == "system" || audioSource == "microphone")
        synchronized(lock) {
            val next = LinkedHashMap(runtimeSnapshots).apply { put(audioSource, value) }
            val merged = mergeRuntime(false, next)
            runtimeOwned = true
            runtimeStopped = false
            runtimeSnapshots.clear()
            runtimeSnapshots.putAll(next)
            originalOnly = sourceOnly
            publishRuntime(merged)
            liveHidden = false
        }
        notifyListeners()
    }
    private fun mergeRuntime(stopped: Boolean, values: Map<String, JSONObject> = runtimeSnapshots): JSONObject = NativeRuntimeConfiguration.exchange(JSONObject()
        .put("operation", "merge_snapshots")
        .put("snapshots", JSONArray().apply { values.forEach { (source, value) ->
            put(JSONObject().put("audioSource", source).put("state", value))
        } }).put("historyLimit", historyLimit).put("stopped", stopped))
    private fun publishRuntime(value: JSONObject) {
        atomicPreview = value.optBoolean("atomicPreview")
        snapshot = value.getJSONObject("snapshot")
        runtimeFeedback = feedback(value)
        detectedSourceLanguage = value.optString("detectedLanguage").takeUnless { it.isBlank() || it == "null" }
    }
    private fun feedback(value: JSONObject): RuntimeFeedback {
        val recovery = value.optJSONObject("translationRecovery")
        return RuntimeFeedback(value.optString("connectionStatus", "idle"),
            recovery?.optString("reason"), recovery?.optBoolean("retryScheduled") ?: false,
            value.optBoolean("isTranslationTimedOut"))
    }
    /** Stop retires drafts and activity through the same shared projection as desktop. */
    fun finishSnapshot() {
        synchronized(lock) {
            if (runtimeSnapshots.isEmpty() && snapshot.has("source")) {
                runtimeSnapshots["system"] = JSONObject().put("snapshot", snapshot).put("atomicPreview", atomicPreview)
            }
            if (runtimeSnapshots.isNotEmpty()) {
                runtimeOwned = true
                runtimeStopped = true
                publishRuntime(mergeRuntime(true))
                // Retain only shared stopped state: no stale live drafts or old
                // full native snapshots remain behind the bounded UI projection.
                runtimeSnapshots.clear()
                val tracks = snapshot.optJSONArray("tracks")
                if (tracks != null) for (index in 0 until tracks.length()) {
                    val track = tracks.getJSONObject(index)
                    runtimeSnapshots[track.getString("audioSource")] = JSONObject()
                        .put("snapshot", track).put("atomicPreview", atomicPreview)
                }
            }
            runtimeFeedback = RuntimeFeedback()
            statusLine = ""
            liveHidden = false
        }
        notifyListeners()
    }
    val hasRetainedContent: Boolean get() = synchronized(lock) {
        displaySnapshots().any { it.source.isNotBlank() || it.translation.isNotBlank() } || historySnapshotWithTime().isNotEmpty()
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
            } else publishRuntime(mergeRuntime(runtimeStopped))
        }
        notifyListeners()
    }
    fun historySnapshot(): List<Pair> = historySnapshotWithTime().map { it.pair }
    /** Read one published shared state; a provider thread can publish during UI rendering. */
    fun displaySnapshot(): DisplaySnapshot = synchronized(lock) { display(snapshot, snapshot.optJSONArray("tracks")?.optJSONObject(0)?.optString("audioSource") ?: "system", runtimeFeedback) }
    fun displaySnapshots(): List<DisplaySnapshot> = synchronized(lock) {
        val tracks = snapshot.optJSONArray("tracks")
        if (tracks == null || tracks.length() == 0) return@synchronized listOf(display(snapshot, "system", runtimeFeedback))
        List(tracks.length()) { index ->
            val track = tracks.getJSONObject(index)
            display(track, track.getString("audioSource"), feedback(track).copy(connectionStatus = runtimeFeedback.connectionStatus))
        }
    }
    private fun display(value: JSONObject, audioSource: String, activity: RuntimeFeedback): DisplaySnapshot {
        val display = value.optJSONObject("displayPair")
        val pair = display?.takeIf { value.optBoolean("displayPairFinal") }
        val rows = history(value, audioSource)
        // Retiring a newer preview can leave the last accepted pair only in this input's history.
        val historyFallback = if (display == null) rows.lastOrNull { it.pair.audioSource == audioSource } else null
        val retained = pair?.let {
            TimedPair(Pair(it.optString("source"), if (originalOnly) "" else it.optString("translation"), audioSource),
                value.optLong("displayPairCreatedAt").takeIf { time -> time > 0 })
        } ?: historyFallback
        val line = value.optJSONObject("source")
        val text = line?.optString("text").orEmpty()
        val sourceConfirmed = line?.optBoolean("isFinal") == true && pair != null &&
            text == pair.optString("source") && line.opt("utteranceId") == pair.opt("utteranceId")
        val preview = value.optJSONObject("realtimePreview")?.takeUnless { atomicPreview }
        val source = if (originalOnly) text else preview?.optJSONObject("source")?.optString("text")
            ?: display?.optString("source") ?: historyFallback?.pair?.source ?: text
        val translation = if (originalOnly) "" else preview?.optJSONObject("translation")?.optString("text")
            ?: display?.optString("translation") ?: historyFallback?.pair?.translation
            ?: value.optJSONObject("translation")?.optString("text").orEmpty()
        val confirmed = preview == null && retained != null
        return DisplaySnapshot(source, translation, liveHidden, confirmed,
            retained?.createdAt.takeIf { confirmed }, rows, statusLine, activity, retained,
            SourceReading(text, sourceConfirmed, retained?.createdAt.takeIf { sourceConfirmed }), audioSource)
    }
    fun historySnapshotWithTime(): List<TimedPair> = synchronized(lock) { history(snapshot, "system") }
    private fun history(value: JSONObject, audioSource: String): List<TimedPair> {
        val rows = value.optJSONArray("history") ?: return emptyList()
        val count = minOf(rows.length(), historyLimit)
        return List(count) { offset ->
            val row = rows.getJSONObject(rows.length() - count + offset)
            val source = row.getString("source")
            val translation = row.getString("translation")
            TimedPair(Pair(source, if (originalOnly && source == translation) "" else translation,
                row.optString("audioSource", audioSource)), row.optLong("createdAt").takeIf { it > 0 })
        }
    }
    fun onStatus(line: String) { statusLine = line; notifyListeners() }
    fun hideLive() {
        synchronized(lock) {
            // The shared current complete pair stays readable until replaced
            // or the session ends, even when saved history is disabled.
            if (snapshot.optJSONObject("displayPair") != null) return
            val tracks = snapshot.optJSONArray("tracks")
            if (tracks != null && (0 until tracks.length()).any { tracks.getJSONObject(it).optJSONObject("displayPair") != null }) return
            // Independent live lanes can already show a translation before the first final.
            // A gap in provider updates does not withdraw it or the last retained reading pair.
            if (displaySnapshots().any { it.translation.isNotBlank() || it.retainedConfirmed != null }) return
            liveHidden = true
        }
        notifyListeners()
    }
    fun clear() {
        synchronized(lock) {
            state = null
            runtimeOwned = false
            runtimeSnapshots.clear()
            runtimeStopped = false
            atomicPreview = false
            runtimeFeedback = RuntimeFeedback()
            snapshot = JSONObject()
            sourceId = 0; confirmationId = 0; draftId = null; originalOnly = false
            statusLine = ""; detectedSourceLanguage = null; liveHidden = false
        }
        notifyListeners()
    }
}
