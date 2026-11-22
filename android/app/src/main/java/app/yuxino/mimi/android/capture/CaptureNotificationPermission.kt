package app.yuxino.mimi.android.capture

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

internal object CaptureNotificationPermission {
    private const val HANDLED = "capture-notification-permission-handled"

    private fun preferences(context: Context) = context.getSharedPreferences("first_run", Context.MODE_PRIVATE)

    /** Opening the notification setting explicitly also settles the optional capture-start prompt. */
    fun markHandled(context: Context) {
        preferences(context).edit().putBoolean(HANDLED, true).apply()
    }

    fun takeFirstOffer(activity: Activity, fromMainScreen: Boolean): Boolean {
        if (Build.VERSION.SDK_INT < 33 || !fromMainScreen || preferences(activity).getBoolean(HANDLED, false)) return false
        // Remember the first in-app start before the dialog. Swiping it away leaves the Android grant unchanged.
        markHandled(activity)
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED ||
            ActivityCompat.shouldShowRequestPermissionRationale(activity, Manifest.permission.POST_NOTIFICATIONS)) return false

        // An older version could already have asked but did not save that choice.
        val installed = activity.packageManager.getPackageInfo(activity.packageName, PackageManager.PackageInfoFlags.of(0))
        return installed.firstInstallTime == installed.lastUpdateTime
    }
}
