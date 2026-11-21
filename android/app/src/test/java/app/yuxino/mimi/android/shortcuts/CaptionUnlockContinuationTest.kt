package app.yuxino.mimi.android.shortcuts

import app.yuxino.mimi.android.capture.CaptureInput
import org.junit.Assert.*
import org.junit.Test

class CaptionUnlockContinuationTest {
    private fun CaptionUnlockContinuation.lockStop(input: CaptureInput = CaptureInput.SYSTEM, active: Boolean = true,
            preview: Boolean = false, stopping: Boolean = false, permissionsValid: Boolean = true, locked: Boolean = true) =
        interrupted(input, active, preview, stopping, permissionsValid, locked)

    private fun CaptionUnlockContinuation.notice(unlocked: Boolean = true, busy: Boolean = false,
            keepIdleNotification: Boolean = true, canPost: Boolean = true) = notice(keepIdleNotification, canPost, busy, unlocked)

    @Test fun realSystemLockInterruptionOffersOnlyAfterUnlockAndAfterCaptureHasStopped() {
        val continuation = CaptionUnlockContinuation()
        continuation.lockStop()
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice(unlocked = false))
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice(busy = true))
        assertFalse(continuation.consume())
        assertEquals(CaptionShortcutNotice.CONTINUE, continuation.notice())
        continuation.offered()
        assertTrue(continuation.consume())
        assertFalse(continuation.consume())
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice())
    }

    @Test fun permissionLossOtherErrorsPreviewAndUserStopNeverOfferContinuation() {
        val excluded = listOf<(CaptionUnlockContinuation) -> Unit>(
            { it.lockStop(active = false) }, { it.lockStop(preview = true) },
            { it.lockStop(stopping = true) }, { it.lockStop(permissionsValid = false) },
            { it.lockStop(locked = false) }, { it.lockStop(input = CaptureInput.MICROPHONE) },
            { it.lockStop(input = CaptureInput.BOTH) },
        )
        for (interrupt in excluded) {
            val continuation = CaptionUnlockContinuation()
            interrupt(continuation)
            assertEquals(CaptionShortcutNotice.IDLE, continuation.notice())
            assertFalse(continuation.consume())
        }
    }

    @Test fun healthCheckAndProjectionCallbackRaceCannotRearmADismissedPrompt() {
        val continuation = CaptionUnlockContinuation()
        continuation.lockStop()
        continuation.lockStop()
        continuation.offered()
        continuation.dismiss()
        continuation.lockStop()
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice())
        assertFalse(continuation.consume())
        continuation.reset()
        continuation.lockStop()
        assertEquals(CaptionShortcutNotice.CONTINUE, continuation.notice())
    }

    @Test fun systemNotificationDenialAndLockScreenNeverExposeAContinuePrompt() {
        val continuation = CaptionUnlockContinuation()
        continuation.lockStop()
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice(canPost = false))
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice(unlocked = false))
        continuation.reset()
        assertEquals(CaptionShortcutNotice.IDLE, continuation.notice())
    }

    @Test fun oneTimeContinuationDoesNotRequireOrEnableThePersistentIdleNotification() {
        val continuation = CaptionUnlockContinuation()
        assertFalse(continuation.pending)
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice(keepIdleNotification = false))
        continuation.lockStop()
        assertTrue(continuation.pending)
        assertEquals(CaptionShortcutNotice.CONTINUE, continuation.notice(keepIdleNotification = false))
        continuation.offered()
        continuation.dismiss()
        assertFalse(continuation.pending)
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice(keepIdleNotification = false))
        continuation.reset()
        assertEquals(CaptionShortcutNotice.NONE, continuation.notice(keepIdleNotification = false))
    }

    @Test fun normalStopNewStartAndProcessRecreationDiscardAnOldContinuation() {
        val continuation = CaptionUnlockContinuation()
        continuation.lockStop()
        continuation.offered()
        continuation.reset()
        assertFalse(continuation.consume())
        assertEquals(CaptionShortcutNotice.IDLE, continuation.notice())
        assertEquals(CaptionShortcutNotice.IDLE, CaptionUnlockContinuation().notice())
    }
}
