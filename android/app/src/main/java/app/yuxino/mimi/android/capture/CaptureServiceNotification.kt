package app.yuxino.mimi.android.capture

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import app.yuxino.mimi.android.MainActivity
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.localization.InterfaceLanguage

/** Foreground-service channel, type bitmask and localized ongoing notification for capture. */
internal object CaptureServiceNotification {
    const val CHANNEL_ID = "capture"
    const val NOTIFICATION_ID = 41

    fun foregroundServiceType(input: CaptureInput): Int =
        (if (input.needsProjection) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION else 0) or
            (if (input.usesMicrophone && Build.VERSION.SDK_INT >= 30) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0)

    fun createChannel(context: Context) {
        val localized = InterfaceLanguage.context(context)
        val channel = NotificationChannel(
            CHANNEL_ID,
            localized.getString(R.string.notification_channel),
            NotificationManager.IMPORTANCE_LOW,
        )
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    fun build(context: Context, input: CaptureInput): Notification {
        val localized = InterfaceLanguage.context(context)
        val stopIntent = PendingIntent.getService(
            context,
            1,
            Intent(context, MimiService::class.java).setAction(MimiService.ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val mainActivityIntent = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val textRes = when (input) {
            CaptureInput.MICROPHONE -> R.string.notification_microphone_text
            CaptureInput.BOTH -> R.string.capture_both_source_hint
            CaptureInput.SYSTEM -> R.string.notification_text
        }
        return Notification.Builder(context, CHANNEL_ID)
            .apply {
                if (Build.VERSION.SDK_INT >= 31) setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
            }
            .setSmallIcon(R.drawable.ic_mimi)
            .setContentTitle(localized.getString(R.string.notification_title))
            .setContentText(localized.getString(textRes))
            .setContentIntent(mainActivityIntent)
            .addAction(
                Notification.Action.Builder(
                    null,
                    localized.getString(R.string.stop_action),
                    stopIntent,
                ).build(),
            )
            .setOngoing(true)
            .build()
    }

    fun refresh(context: Context, input: CaptureInput) {
        createChannel(context)
        context.getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, build(context, input))
    }
}
