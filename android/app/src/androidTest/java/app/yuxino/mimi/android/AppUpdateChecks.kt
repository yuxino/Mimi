package app.yuxino.mimi.android

import android.app.Activity
import android.app.Instrumentation
import android.app.LocaleManager
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.os.Bundle
import android.os.Build
import android.os.LocaleList
import android.os.SystemClock
import android.view.View
import android.view.ViewTreeObserver
import android.widget.LinearLayout
import android.widget.RadioGroup
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import app.yuxino.mimi.android.capture.MimiService
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Exercises the real native control with held results; never contacts GitHub or a provider. */
internal class AppUpdateChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private var stage = "preconditions"

    private class HeldChecker : AppUpdateChecker {
        class Request(val completed: (AppUpdateResult) -> Unit) { var cancelled = false }
        val requests = mutableListOf<Request>()
        override fun check(installedVersion: String, completed: (AppUpdateResult) -> Unit): AppUpdateCall {
            check(AppVersion.parse(installedVersion) != null)
            val request = Request(completed)
            requests += request
            return AppUpdateCall { request.cancelled = true }
        }
    }

    fun run(arguments: Bundle?) {
        val prefs = context.getSharedPreferences("first_run", 0)
        val hadSeen = prefs.contains("seen")
        val wasSeen = prefs.getBoolean("seen", false)
        val priorTheme = AppCompatDelegate.getDefaultNightMode()
        val localeManager = if (Build.VERSION.SDK_INT >= 33) context.getSystemService(LocaleManager::class.java) else null
        val priorLocales = if (Build.VERSION.SDK_INT >= 33) LocaleListCompat.wrap(checkNotNull(localeManager).applicationLocales)
            else AppCompatDelegate.getApplicationLocales()
        val theme = arguments?.getString("theme") ?: "light"
        val locale = arguments?.getString("locale") ?: "en"
        var home: MainActivity? = null
        var failure: String? = null
        var openedUrl: String? = null
        val monitor = object : Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: Intent): Instrumentation.ActivityResult? {
                if (intent.action != Intent.ACTION_VIEW) return null
                openedUrl = intent.dataString
                return Instrumentation.ActivityResult(Activity.RESULT_CANCELED, null)
            }
        }
        try {
            check(context.packageName.endsWith(".dev") && context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!MimiService.isStarting && !MimiService.isRunning && !MimiService.isStopping)
            check(locale in InterfaceLanguage.tags.drop(1) && theme in listOf("light", "dark"))
            check(prefs.edit().putBoolean("seen", true).commit())
            onUi {
                AppCompatDelegate.setDefaultNightMode(if (theme == "dark") AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO)
                if (Build.VERSION.SDK_INT >= 33) checkNotNull(localeManager).applicationLocales = LocaleList.forLanguageTags(locale)
                else AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(locale))
            }
            val activity = test.startActivitySync(Intent(context, MainActivity::class.java)
                .putExtra("settings_section", "general").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
            home = activity
            test.waitForIdleSync()
            val checker = HeldChecker()
            stage = "native_entry"
            onUi {
                check(InterfaceLanguage.indexOf(activity.resources.configuration.locales[0]) == InterfaceLanguage.tags.indexOf(locale))
                check(field<TextView>(activity, "app-update-version").text.isNotBlank())
                check(field<View>(activity, "app-update-check").isEnabled)
                check(field<View>(activity, "app-update-download").visibility == View.GONE)
                // Keep MainActivity's navigation and lifecycle wiring; replace only the request boundary.
                val original = field<AppUpdateControls>(activity, "app-updates")
                val parent = original.parent as LinearLayout
                val index = parent.indexOfChild(original)
                val controls = AppUpdateControls(activity, checker)
                parent.removeView(original)
                parent.addView(controls, index, original.layoutParams)
                MainActivity::class.java.getDeclaredField("appUpdates").run { isAccessible = true; set(activity, controls) }
                controls.setPageVisible(true)
            }
            stage = "busy_and_navigation"
            onUi {
                field<View>(activity, "app-update-check").performClick()
                field<View>(activity, "app-update-check").performClick()
                check(checker.requests.size == 1)
                check(!field<View>(activity, "app-update-check").isEnabled)
                check(field<TextView>(activity, "app-update-check").text.toString() == activity.getString(R.string.app_update_checking))
                activity.findViewById<RadioGroup>(R.id.settings_tabs).check(R.id.tab_caption)
                check(checker.requests[0].cancelled)
                activity.findViewById<RadioGroup>(R.id.settings_tabs).check(R.id.tab_general)
                field<View>(activity, "app-update-check").performClick()
                check(checker.requests.size == 2)
            }
            val release = AppRelease(checkNotNull(AppVersion.parse("99.0.0")),
                "https://github.com/yuxino/Mimi/releases/download/v99.0.0/mimi_99.0.0_android.apk")
            checker.requests[0].completed(AppUpdateResult.Available(release))
            test.waitForIdleSync()
            onUi {
                check(!field<View>(activity, "app-update-check").isEnabled)
                check(field<View>(activity, "app-update-download").visibility == View.GONE)
            }
            stage = "failure_and_retry"
            checker.requests[1].completed(AppUpdateResult.Failed)
            test.waitForIdleSync()
            onUi {
                check(field<View>(activity, "app-update-check").isEnabled)
                check(field<TextView>(activity, "app-update-status").text.toString() == activity.getString(R.string.app_update_failed))
            }
            capture(activity, "failure-$locale-$theme")
            onUi { field<View>(activity, "app-update-check").performClick() }
            checker.requests[2].completed(AppUpdateResult.Available(release))
            test.waitForIdleSync()
            stage = "available_and_handoff"
            onUi {
                check(field<View>(activity, "app-update-download").visibility == View.VISIBLE)
                check(field<TextView>(activity, "app-update-status").text.toString() == activity.getString(R.string.app_update_available, "99.0.0"))
            }
            capture(activity, "available-$locale-$theme")
            test.addMonitor(monitor)
            onUi { check(field<View>(activity, "app-update-download").performClick()) }
            check(openedUrl == release.downloadUrl)
            test.removeMonitor(monitor)
            stage = "up_to_date"
            onUi { field<View>(activity, "app-update-check").performClick() }
            checker.requests[3].completed(AppUpdateResult.UpToDate)
            test.waitForIdleSync()
            onUi {
                check(field<View>(activity, "app-update-check").isEnabled)
                check(field<View>(activity, "app-update-status").visibility == View.GONE)
                check(field<View>(activity, "app-update-download").visibility == View.GONE)
            }
            stage = "blur_suppresses_late_notice"
            var dialog: androidx.appcompat.app.AlertDialog? = null
            val lostFocus = CountDownLatch(1)
            val focusListener = ViewTreeObserver.OnWindowFocusChangeListener { focused ->
                if (!focused) lostFocus.countDown()
            }
            onUi {
                activity.window.decorView.viewTreeObserver.addOnWindowFocusChangeListener(focusListener)
                field<View>(activity, "app-update-check").performClick()
                dialog = com.google.android.material.dialog.MaterialAlertDialogBuilder(activity)
                    .setMessage("Synthetic focus fixture")
                    .setPositiveButton(android.R.string.ok, null).show()
            }
            check(lostFocus.await(3, TimeUnit.SECONDS))
            test.waitForIdleSync()
            onUi {
                activity.window.decorView.viewTreeObserver.removeOnWindowFocusChangeListener(focusListener)
                check(!activity.hasWindowFocus())
            }
            checker.requests[4].completed(AppUpdateResult.UpToDate)
            test.waitForIdleSync()
            onUi {
                val controls = field<AppUpdateControls>(activity, "app-updates")
                check(AppUpdateControls::class.java.getDeclaredField("toast").run {
                    isAccessible = true; get(controls) == null
                })
                dialog?.dismiss()
            }
            test.waitForIdleSync()
            val destroyed = CountDownLatch(1)
            onUi {
                activity.lifecycle.addObserver(LifecycleEventObserver { _, event ->
                    if (event == Lifecycle.Event.ON_DESTROY) destroyed.countDown()
                })
                activity.findViewById<RadioGroup>(R.id.settings_tabs).check(R.id.tab_caption)
                activity.findViewById<RadioGroup>(R.id.settings_tabs).check(R.id.tab_general)
                field<View>(activity, "app-update-check").performClick()
                activity.finish()
            }
            stage = "close_lifecycle"
            check(destroyed.await(3, TimeUnit.SECONDS))
            test.waitForIdleSync()
            stage = "close_cancels_request"
            check(checker.requests[5].cancelled)
            checker.requests[5].completed(AppUpdateResult.Available(release))
            test.waitForIdleSync()
        } catch (_: Throwable) {
            failure = stage
        } finally {
            test.removeMonitor(monitor)
            onUi {
                home?.takeUnless { it.isFinishing }?.finish()
                if (Build.VERSION.SDK_INT >= 33) checkNotNull(localeManager).applicationLocales = LocaleList.forLanguageTags(priorLocales.toLanguageTags())
                else AppCompatDelegate.setApplicationLocales(priorLocales)
                AppCompatDelegate.setDefaultNightMode(priorTheme)
            }
            val editor = prefs.edit()
            if (hadSeen) editor.putBoolean("seen", wasSeen) else editor.remove("seen")
            if (!editor.commit()) failure = "restore_first_run"
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", if (failure == null)
                "App update UI passed ($locale/$theme): native entry, busy state, duplicate click, navigation cancellation, stale result, failure/retry, available/current states, browser intent, blur suppression and close cancellation. Synthetic update results only; no provider requests, download, install, capture or credential changes.\n"
            else "App update UI failed: stage=$failure.\n")
        })
    }

    private fun capture(activity: Activity, name: String) {
        test.waitForIdleSync()
        val painted = CountDownLatch(1)
        onUi {
            activity.window.decorView.viewTreeObserver.registerFrameCommitCallback { painted.countDown() }
            activity.window.decorView.invalidate()
        }
        check(painted.await(3, TimeUnit.SECONDS))
        // A committed render frame must also reach the compositor before its screenshot.
        SystemClock.sleep(100)
        val screenshot = checkNotNull(test.uiAutomation.takeScreenshot())
        try {
            val directory = File(context.getExternalFilesDir(null), "update-check").apply { mkdirs() }
            File(directory, "$name.png").outputStream().use { check(screenshot.compress(Bitmap.CompressFormat.PNG, 100, it)) }
        } finally { screenshot.recycle() }
    }

    private inline fun <reified T : View> field(activity: Activity, tag: String): T =
        checkNotNull(activity.findViewById<View>(android.R.id.content).findViewWithTag<T>(tag))
    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync {
            try { action() } catch (error: Throwable) { failure = error }
        }
        failure?.let { throw it }
    }
}
