package app.yuxino.mimi.android.shortcuts

import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import androidx.core.service.quicksettings.PendingIntentActivityWrapper
import app.yuxino.mimi.android.capture.MimiService

/** All external start controls enter the same visible Activity permission flow. */
object CaptionShortcutActions {
    const val START_CAPTIONS = "app.yuxino.mimi.android.action.START_CAPTIONS"
    const val ENTRY_ACTIVITY = "app.yuxino.mimi.android.shortcuts.CaptionShortcutEntry"
    private const val START_REQUEST = 2401
    private const val CONTINUE_REQUEST = 2403
    internal const val EXTRA_CONTINUE_SYSTEM_AUDIO = "continue_system_audio"

    fun startIntent(context: Context): Intent = Intent(START_CAPTIONS)
        .setComponent(ComponentName(context.packageName, ENTRY_ACTIVITY))
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)

    // This creates or updates the intent; only FLAG_NO_CREATE may return null.
    fun startPendingIntent(context: Context): PendingIntent = checkNotNull(startActivityWrapper(context).pendingIntent)

    internal fun startActivityWrapper(context: Context): PendingIntentActivityWrapper = PendingIntentActivityWrapper(
        context, START_REQUEST, startIntent(context), PendingIntent.FLAG_UPDATE_CURRENT, /* isMutable = */ false,
    )

    internal fun continuePendingIntent(context: Context): PendingIntent = PendingIntent.getActivity(context, CONTINUE_REQUEST,
        startIntent(context).putExtra(EXTRA_CONTINUE_SYSTEM_AUDIO, true), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)

    fun state(): CaptionShortcutState = captionShortcutState(MimiService.isRunning, MimiService.isStarting, MimiService.isStopping)

    internal fun isStartRequest(context: Context, intent: Intent): Boolean = trustedCaptionStart(
        context.packageName, intent.action, intent.component?.packageName, intent.component?.className,
    )
}

enum class CaptionShortcutState { INACTIVE, STARTING, ACTIVE, STOPPING }

internal fun captionShortcutState(running: Boolean, starting: Boolean, stopping: Boolean): CaptionShortcutState = when {
    stopping -> CaptionShortcutState.STOPPING
    starting -> CaptionShortcutState.STARTING
    running -> CaptionShortcutState.ACTIVE
    else -> CaptionShortcutState.INACTIVE
}

internal fun trustedCaptionStart(ownPackage: String, action: String?, componentPackage: String?, componentClass: String?): Boolean =
    action == CaptionShortcutActions.START_CAPTIONS && componentPackage == ownPackage && componentClass == CaptionShortcutActions.ENTRY_ACTIVITY

internal enum class CaptionShortcutPhase { IDLE, QUEUED, STARTING, RETURNING }

/** Saved Activity navigation intent, never a replacement for capture/service state. */
internal class CaptionShortcutStartRoute(var phase: CaptionShortcutPhase = CaptionShortcutPhase.IDLE) {
    val active: Boolean get() = phase != CaptionShortcutPhase.IDLE

    fun request(busy: Boolean) {
        if (!active && !busy) phase = CaptionShortcutPhase.QUEUED
    }

    /** A restored/duplicate request cannot present a second permission dialog. */
    fun dispatch(visibleAndUnlocked: Boolean, busy: Boolean, running: Boolean): Boolean {
        if (phase != CaptionShortcutPhase.QUEUED || !visibleAndUnlocked || busy) return false
        phase = if (running) CaptionShortcutPhase.RETURNING else CaptionShortcutPhase.STARTING
        return !running
    }

    fun observeStarted(running: Boolean, busy: Boolean) {
        if (phase == CaptionShortcutPhase.STARTING && running && !busy) phase = CaptionShortcutPhase.RETURNING
    }

    fun cancelled() {
        if (active) phase = CaptionShortcutPhase.RETURNING
    }

    fun takeReturn(visibleAndUnlocked: Boolean, permissionPending: Boolean): Boolean {
        if (phase != CaptionShortcutPhase.RETURNING || !visibleAndUnlocked || permissionPending) return false
        phase = CaptionShortcutPhase.IDLE
        return true
    }
}
