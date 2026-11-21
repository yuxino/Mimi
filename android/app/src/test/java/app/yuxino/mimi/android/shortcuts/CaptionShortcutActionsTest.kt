package app.yuxino.mimi.android.shortcuts

import org.junit.Assert.*
import org.junit.Test

class CaptionShortcutActionsTest {
    @Test fun onlyOurNonExportedEntryCanRequestAutomaticStart() {
        val app = "app.yuxino.mimi.android.dev"
        val action = CaptionShortcutActions.START_CAPTIONS
        val alias = CaptionShortcutActions.ENTRY_ACTIVITY
        assertTrue(trustedCaptionStart(app, action, app, alias))
        assertFalse(trustedCaptionStart(app, action, app, "app.yuxino.mimi.android.MainActivity"))
        assertFalse(trustedCaptionStart(app, action, "another.app", alias))
        assertFalse(trustedCaptionStart(app, "android.intent.action.MAIN", app, alias))
        assertFalse(trustedCaptionStart(app, action, null, null))
    }

    @Test fun pendingAndDrainingStatesNeverPresentAnOptimisticActiveTile() {
        assertEquals(CaptionShortcutState.INACTIVE, captionShortcutState(false, false, false))
        assertEquals(CaptionShortcutState.ACTIVE, captionShortcutState(true, false, false))
        assertEquals(CaptionShortcutState.STARTING, captionShortcutState(false, true, false))
        assertEquals(CaptionShortcutState.STOPPING, captionShortcutState(true, false, true))
    }

    @Test fun coldOrWarmRequestWaitsForVisibleUnlockedActivityAndIsConsumedOnce() {
        val route = CaptionShortcutStartRoute()
        route.request(busy = false)
        assertFalse(route.dispatch(visibleAndUnlocked = false, busy = false, running = false))
        assertTrue(route.dispatch(visibleAndUnlocked = true, busy = false, running = false))
        route.request(busy = false)
        assertFalse(route.dispatch(visibleAndUnlocked = true, busy = false, running = false))
    }

    @Test fun rotationDuringConsentDoesNotRepeatPermissionAndCancellationCanReturn() {
        val original = CaptionShortcutStartRoute()
        original.request(busy = false)
        assertTrue(original.dispatch(true, false, false))
        val restored = CaptionShortcutStartRoute(original.phase)
        assertFalse(restored.dispatch(true, false, false))
        restored.cancelled()
        assertTrue(restored.takeReturn(visibleAndUnlocked = true, permissionPending = false))
        assertFalse(restored.active)
        restored.request(busy = false)
        assertTrue(restored.dispatch(true, false, false))
    }

    @Test fun serviceMustAcknowledgeBeforeReturningAndNotificationPermissionMustSettle() {
        val route = CaptionShortcutStartRoute()
        route.request(false)
        assertTrue(route.dispatch(true, false, false))
        route.observeStarted(running = false, busy = true)
        assertFalse(route.takeReturn(true, false))
        route.observeStarted(running = true, busy = true)
        assertFalse(route.takeReturn(true, false))
        route.observeStarted(running = true, busy = false)
        assertFalse(route.takeReturn(true, permissionPending = true))
        assertTrue(route.takeReturn(true, permissionPending = false))
    }

    @Test fun staleStartWhileRunningNeverStopsOrRestartsTheSession() {
        val route = CaptionShortcutStartRoute()
        route.request(false)
        assertFalse(route.dispatch(true, false, running = true))
        assertTrue(route.takeReturn(true, false))
    }

    @Test fun busyRequestCannotStartLaterWhenTheOtherCaptureSettles() {
        val route = CaptionShortcutStartRoute()
        route.request(busy = true)
        assertFalse(route.active)
        assertFalse(route.dispatch(true, false, false))
    }
}
