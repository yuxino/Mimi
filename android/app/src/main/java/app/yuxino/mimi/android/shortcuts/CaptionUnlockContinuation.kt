package app.yuxino.mimi.android.shortcuts

import app.yuxino.mimi.android.capture.CaptureInput

internal enum class CaptionShortcutNotice { NONE, IDLE, CONTINUE }

/** Process-local intent only: a reboot or process restart cannot resume capture. */
internal class CaptionUnlockContinuation {
    private enum class Phase { NONE, WAITING_FOR_UNLOCK, OFFERED, DISMISSED }
    private var phase = Phase.NONE
    val pending: Boolean get() = phase == Phase.WAITING_FOR_UNLOCK || phase == Phase.OFFERED

    fun interrupted(input: CaptureInput, active: Boolean, preview: Boolean, stopping: Boolean,
                    permissionsValid: Boolean, locked: Boolean) {
        if (input == CaptureInput.SYSTEM && active && !preview && !stopping && permissionsValid && locked && phase == Phase.NONE)
            phase = Phase.WAITING_FOR_UNLOCK
    }

    fun notice(keepIdleNotification: Boolean, canPost: Boolean, busy: Boolean, unlocked: Boolean): CaptionShortcutNotice {
        if (!canPost || busy) return CaptionShortcutNotice.NONE
        return when (phase) {
            Phase.NONE -> if (keepIdleNotification) CaptionShortcutNotice.IDLE else CaptionShortcutNotice.NONE
            Phase.WAITING_FOR_UNLOCK, Phase.OFFERED -> if (unlocked) CaptionShortcutNotice.CONTINUE else CaptionShortcutNotice.NONE
            Phase.DISMISSED -> CaptionShortcutNotice.NONE
        }
    }

    fun offered() {
        if (phase == Phase.WAITING_FOR_UNLOCK) phase = Phase.OFFERED
    }

    fun consume(): Boolean {
        if (phase != Phase.OFFERED) return false
        phase = Phase.DISMISSED
        return true
    }

    fun dismiss() { phase = Phase.DISMISSED }
    fun reset() { phase = Phase.NONE }
}
