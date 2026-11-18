package app.yuxino.mimi.android.captions

import android.content.Context
import android.view.View
import android.widget.TextView
import app.yuxino.mimi.android.InterfaceLanguage
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.SettingsStore
import app.yuxino.mimi.android.capture.CaptureHealth
import app.yuxino.mimi.android.capture.CaptureInput
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.SubtitleBus
import app.yuxino.mimi.android.runtimeFeedbackResource

/** Native presentation of the bounded shared subtitle state; never retains a transcript. */
internal data class CaptionFrameContent(
    val original: String,
    val translated: String,
    val hidden: Boolean = false,
    val status: String = "",
    val history: String = "",
    val historyLimit: Int = 0,
    val confirmed: Boolean = false,
    val timestamp: String = "",
) {
    fun applyRows(source: TextView, translation: TextView, statusView: TextView, mode: CaptionDisplayMode) {
        source.setCaptionText(original); translation.setCaptionText(translated); statusView.setCaptionText(status)
        source.visibility = if (!hidden && mode.sourceVisible && original.isNotBlank()) View.VISIBLE else View.GONE
        translation.visibility = if (!hidden && mode.translationVisible && translated.isNotBlank()) View.VISIBLE else View.GONE
        statusView.visibility = if (status.isBlank()) View.GONE else View.VISIBLE
    }

    companion object {
        fun live(context: Context, mode: CaptionDisplayMode, historyLimit: Int,
                 captureState: CaptureHealth.State?, running: Boolean, stopping: Boolean): CaptionFrameContent {
            val states = SubtitleBus.displaySnapshots().map { it.forReading(mode) }
            val multiSource = states.size > 1
            val localized = InterfaceLanguage.context(context)
            fun sourceName(id: String) = localized.getString(if (id == "microphone")
                R.string.capture_input_microphone else R.string.capture_input_system)
            val visible = states.any { !it.hidden && (mode.sourceVisible && it.source.isNotBlank() ||
                mode.translationVisible && it.translation.isNotBlank()) }
            val runtime = states.firstNotNullOfOrNull { state ->
                (runtimeFeedbackResource(state.feedback)?.let(localized::getString) ?: state.status).takeIf { it.isNotBlank() }
            }.orEmpty()
            val status = when {
                captureState == CaptureHealth.State.NO_PCM -> when (MimiService.captureInput) {
                    CaptureInput.MICROPHONE -> localized.getString(R.string.microphone_no_sound_hint)
                    CaptureInput.BOTH -> MimiService.captureObservations.entries
                        .filter { it.value.state == CaptureHealth.State.NO_PCM }
                        .joinToString("\n") { "${sourceName(it.key.id)} · ${localized.getString(R.string.capture_no_pcm)}" }
                        .ifBlank { localized.getString(R.string.capture_both_source_hint) }
                    CaptureInput.SYSTEM -> localized.getString(R.string.capture_no_sound_hint)
                }
                runtime.isNotBlank() -> runtime
                running && !stopping && !visible -> localized.getString(R.string.overlay_listening_empty)
                else -> ""
            }
            val showTime = SettingsStore.subtitleTimestamps(context)
            fun text(translation: Boolean) = states.filterNot { it.hidden }.mapNotNull { state ->
                val value = if (translation) state.translation else state.source
                value.takeIf { it.isNotBlank() }?.let {
                    if (!multiSource) it else listOfNotNull(sourceName(state.audioSource),
                        captionTime(state.createdAt).takeIf { showTime && it.isNotBlank() }, it).joinToString("\n")
                }
            }.joinToString("\n\n")
            val history = if (historyLimit > 0) SubtitleBus.historySnapshotWithTime().takeLast(historyLimit)
                .filterNot { timed -> states.any { state -> state.createdAt != null &&
                    state.createdAt == timed.createdAt && state.audioSource == timed.pair.audioSource } }
                .joinToString("\n\n") { timed ->
                    val pair = timed.pair
                    listOfNotNull(sourceName(pair.audioSource).takeIf { multiSource },
                        captionTime(timed.createdAt).takeIf { showTime && it.isNotBlank() },
                        pair.source.takeIf { mode.sourceVisible && it.isNotBlank() },
                        pair.translation.takeIf { mode.translationVisible && it.isNotBlank() }).joinToString("\n")
                } else ""
            return CaptionFrameContent(text(false), text(true), states.all { it.hidden }, status, history, historyLimit,
                states.any { it.source.isNotBlank() || it.translation.isNotBlank() } &&
                    states.filter { it.source.isNotBlank() || it.translation.isNotBlank() }.all { it.confirmed },
                if (showTime && !multiSource) captionTime(states.firstOrNull()?.createdAt) else "")
        }
    }
}

/** Original reading follows recognition independently of translation. Translated reading
 * keeps the shared complete pair while waiting, never mixing sentence owners. */
internal fun SubtitleBus.DisplaySnapshot.forReading(mode: CaptionDisplayMode): SubtitleBus.DisplaySnapshot {
    if (hidden) return this
    if (mode == CaptionDisplayMode.ORIGINAL) {
        val current = copy(source = currentSource.text,
            confirmed = currentSource.confirmed, createdAt = currentSource.createdAt)
        if (currentSource.text.isNotBlank()) return current
        // ASR may publish an empty opening draft between utterances. Keep the
        // shared completed original until actual new words arrive, just as the
        // translated modes keep their completed pair while MT is pending.
        val retained = retainedConfirmed?.takeIf { it.pair.source.isNotBlank() }
            ?: return if (source.isNotBlank()) this else current
        return copy(source = retained.pair.source, confirmed = true, createdAt = retained.createdAt)
    }
    if (!mode.translationVisible || translation.isNotBlank()) return this
    val retained = retainedConfirmed?.takeIf { it.pair.translation.isNotBlank() } ?: return this
    return copy(source = retained.pair.source, translation = retained.pair.translation,
        confirmed = true, createdAt = retained.createdAt)
}

/** Unchanged runtime/health publications must not reset a TextView's native viewport. */
internal fun TextView.setCaptionText(value: String) {
    if (text.toString() != value) text = value
}
