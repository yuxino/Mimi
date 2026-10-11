package app.yuxino.mimi.android

import android.content.Context
import android.view.View
import android.widget.TextView
import app.yuxino.mimi.android.capture.CaptureHealth
import app.yuxino.mimi.android.provider.SubtitleBus

/** Native presentation of the bounded shared subtitle state; never retains a transcript. */
internal data class CaptionFrameContent(
    val original: String,
    val translated: String,
    val hidden: Boolean = false,
    val status: String = "",
    val history: String = "",
    val historyLimit: Int = 0,
    val confirmed: Boolean = false,
) {
    fun applyRows(source: TextView, translation: TextView, statusView: TextView, mode: CaptionDisplayMode) {
        source.text = original; translation.text = translated; statusView.text = status
        source.visibility = if (!hidden && mode.sourceVisible && original.isNotBlank()) View.VISIBLE else View.GONE
        translation.visibility = if (!hidden && mode.translationVisible && translated.isNotBlank()) View.VISIBLE else View.GONE
        statusView.visibility = if (status.isBlank()) View.GONE else View.VISIBLE
    }

    companion object {
        fun live(context: Context, mode: CaptionDisplayMode, historyLimit: Int,
                 captureState: CaptureHealth.State?, running: Boolean, stopping: Boolean): CaptionFrameContent {
            val bus = SubtitleBus
            val visible = !bus.liveHidden && (mode.sourceVisible && bus.displaySource.isNotBlank() ||
                mode.translationVisible && bus.displayTranslation.isNotBlank())
            val localized = InterfaceLanguage.context(context)
            val runtime = runtimeFeedbackResource(bus.runtimeFeedback)?.let(localized::getString) ?: bus.statusLine
            val status = when {
                captureState == CaptureHealth.State.NO_PCM -> localized.getString(R.string.capture_no_sound_hint)
                runtime.isNotBlank() -> runtime
                running && !stopping && !visible -> localized.getString(R.string.overlay_listening_empty)
                else -> ""
            }
            val history = if (historyLimit > 0) bus.historySnapshot().takeLast(historyLimit).joinToString("\n\n") { pair ->
                listOfNotNull(pair.source.takeIf { mode.sourceVisible && it.isNotBlank() },
                    pair.translation.takeIf { mode.translationVisible && it.isNotBlank() }).joinToString("\n")
            } else ""
            return CaptionFrameContent(bus.displaySource, bus.displayTranslation, bus.liveHidden, status, history, historyLimit, bus.displayPairFinal)
        }
    }
}
