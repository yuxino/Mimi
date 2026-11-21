package app.yuxino.mimi.android.shortcuts

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.provider.Settings
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.localization.InterfaceLanguage

/** One ID replaces idle Start with an explicit, dismissible continuation after unlocking. */
internal class CaptionShortcutNotification(private val context: Context) {
    private val manager = context.getSystemService(NotificationManager::class.java)
    private var postedLocale: String? = null
    private var postedNotice = CaptionShortcutNotice.NONE
    private var needsInitialCleanup = true

    fun refresh(notice: CaptionShortcutNotice): Boolean {
        if (notice == CaptionShortcutNotice.NONE || MimiService.isStarting || MimiService.isRunning || MimiService.isStopping || !canPost(context)) {
            if (needsInitialCleanup || postedLocale != null) manager.cancel(ID)
            needsInitialCleanup = false
            postedLocale = null
            postedNotice = CaptionShortcutNotice.NONE
            return false
        }
        val localized = InterfaceLanguage.context(context)
        val locale = localized.resources.configuration.locales.toLanguageTags()
        if (postedLocale == locale && postedNotice == notice) return true
        manager.createNotificationChannel(NotificationChannel(CHANNEL,
            localized.getString(R.string.shortcut_notification_label), NotificationManager.IMPORTANCE_LOW))
        val continuing = notice == CaptionShortcutNotice.CONTINUE
        val start = if (continuing) CaptionShortcutActions.continuePendingIntent(context) else CaptionShortcutActions.startPendingIntent(context)
        val dismissed = PendingIntent.getBroadcast(context, 2404, Intent(ACTION_DISMISSED).setPackage(context.packageName),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_mimi)
            .setContentTitle(localized.getString(if (continuing) R.string.shortcut_captions_paused else R.string.shortcut_captions_off))
            .apply {
                if (continuing) {
                    val reason = localized.getString(R.string.shortcut_resume_after_unlock)
                    setContentText(reason)
                    setStyle(Notification.BigTextStyle().bigText(reason))
                }
            }
            .setContentIntent(start)
            .setDeleteIntent(dismissed)
            .addAction(Notification.Action.Builder(null, localized.getString(if (continuing) R.string.shortcut_resume_captions else R.string.start_capture), start).build())
            .setOngoing(!continuing)
            .setAutoCancel(continuing)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setVisibility(Notification.VISIBILITY_SECRET)
            .build()
        manager.notify(ID, notification)
        postedLocale = locale
        postedNotice = notice
        needsInitialCleanup = false
        return true
    }

    companion object {
        private const val CHANNEL = "caption_shortcuts"
        private const val ID = 42
        const val ACTION_DISMISSED = "app.yuxino.mimi.android.action.DISMISS_CAPTION_SHORTCUT"

        fun canPost(context: Context): Boolean {
            val manager = context.getSystemService(NotificationManager::class.java)
            return manager.areNotificationsEnabled() &&
                manager.getNotificationChannel(CHANNEL)?.importance != NotificationManager.IMPORTANCE_NONE
        }

        fun settingsIntent(context: Context): Intent {
            val manager = context.getSystemService(NotificationManager::class.java)
            val channelDisabled = manager.areNotificationsEnabled() &&
                manager.getNotificationChannel(CHANNEL)?.importance == NotificationManager.IMPORTANCE_NONE
            return Intent(if (channelDisabled) Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS
                else Settings.ACTION_APP_NOTIFICATION_SETTINGS).apply {
                putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
                if (channelDisabled) putExtra(Settings.EXTRA_CHANNEL_ID, CHANNEL)
            }
        }
    }
}
