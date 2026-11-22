package app.yuxino.mimi.android.capture

import android.Manifest
import android.app.Activity
import android.app.Instrumentation
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import app.yuxino.mimi.android.MainActivity

/** Uses the current Android grant and install history. The caller prepares the requested device state. */
internal class CaptureNotificationPermissionChecks(private val test: Instrumentation) {
    fun run(scenario: String) {
        val result = Bundle()
        val context = test.targetContext
        val history = context.getSharedPreferences("first_run", Context.MODE_PRIVATE)
        val key = "capture-notification-permission-handled"
        val original = history.all[key] as Boolean?
        var activity: MainActivity? = null
        try {
            check(Build.VERSION.SDK_INT >= 33)
            val home = test.startActivitySync(Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            activity = home
            val pkg = context.packageManager.getPackageInfo(context.packageName, PackageManager.PackageInfoFlags.of(0))
            val granted = context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
            val denied = home.shouldShowRequestPermissionRationale(Manifest.permission.POST_NOTIFICATIONS)
            when (scenario) {
                "fresh" -> check(!granted && !denied && pkg.firstInstallTime == pkg.lastUpdateTime)
                "denied" -> check(!granted && denied)
                "granted" -> check(granted)
                "upgrade" -> check(!granted && pkg.firstInstallTime != pkg.lastUpdateTime)
                else -> error("Unknown notification offer scenario")
            }
            check(history.edit().remove(key).commit())
            onUi {
                check(!CaptureNotificationPermission.takeFirstOffer(home, fromMainScreen = false))
                check(!history.contains(key)) { "A shortcut consumed the in-app offer" }
                check(CaptureNotificationPermission.takeFirstOffer(home, fromMainScreen = true) == (scenario == "fresh"))
                check(history.getBoolean(key, false))
                check(!CaptureNotificationPermission.takeFirstOffer(home, fromMainScreen = true)) { "Repeated the offer" }
                check(history.edit().remove(key).commit())
                CaptureNotificationPermission.markHandled(home)
                check(!CaptureNotificationPermission.takeFirstOffer(home, fromMainScreen = true)) { "Ignored the user's settings choice" }
            }
            result.putString("result", "passed")
        } catch (failure: Throwable) {
            result.putString("failure", failure.toString())
        } finally {
            val restore = history.edit()
            if (original == null) restore.remove(key) else restore.putBoolean(key, original)
            if (!restore.commit()) result.putString("failure", "Could not restore notification offer history")
            activity?.let { home -> test.runOnMainSync { home.finish() } }
        }
        test.finish(if (result.containsKey("failure")) Activity.RESULT_CANCELED else Activity.RESULT_OK, result)
    }

    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
}
