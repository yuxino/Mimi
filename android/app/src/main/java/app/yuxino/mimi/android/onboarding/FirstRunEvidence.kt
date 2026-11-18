package app.yuxino.mimi.android.onboarding

/** Sharing recovery is shown only for a stopped, invalidated capture session. */
internal fun projectionSharingEnded(running: Boolean, captureError: String?): Boolean =
    !running && captureError in setOf("capture.projection_stopped", "capture.permission_or_lock_changed")

/** Content-free proof, scoped to one capture session. Preview UI cannot complete setup. */
class FirstRunEvidence {
    var audioSubmitted = false
        private set
    var captionRendered = false
        private set
    val complete get() = audioSubmitted && captionRendered
    fun submitted(pcm: ByteArray) {
        if (pcm.isNotEmpty() && pcm.any { it != 0.toByte() }) audioSubmitted = true
    }
    fun rendered(nonempty: Boolean, visible: Boolean, preview: Boolean) {
        if (audioSubmitted && nonempty && visible && !preview) captionRendered = true
    }
    fun reset() { audioSubmitted = false; captionRendered = false }
}
