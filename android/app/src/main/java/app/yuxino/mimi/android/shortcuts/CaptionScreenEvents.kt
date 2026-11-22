package app.yuxino.mimi.android.shortcuts

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import androidx.core.content.ContextCompat

/** System UI can send USER_PRESENT under its own UID. All three actions are protected by Android. */
internal fun Context.registerCaptionScreenEvents(receiver: BroadcastReceiver) {
    val screenEvents = IntentFilter().apply {
        addAction(Intent.ACTION_SCREEN_OFF)
        addAction(Intent.ACTION_SCREEN_ON)
        addAction(Intent.ACTION_USER_PRESENT)
    }
    ContextCompat.registerReceiver(this, receiver, screenEvents, ContextCompat.RECEIVER_EXPORTED)
}
