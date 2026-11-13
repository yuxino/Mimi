package app.yuxino.mimi.android

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.google.android.material.button.MaterialButton

internal class AppUpdateControls(
    private val activity: AppCompatActivity,
    private val checker: AppUpdateChecker = GitHubAppUpdateChecker(),
) : LinearLayout(activity) {
    private val callbacks = Handler(Looper.getMainLooper())
    private var pageVisible = false
    private var generation = 0L
    private var noticeGeneration = 0L
    private var pending: AppUpdateCall? = null
    private var checking = false
    private var release: AppRelease? = null
    private var toast: Toast? = null
    @Suppress("DEPRECATION")
    private val installedVersion = activity.packageManager.getPackageInfo(activity.packageName, 0).versionName.orEmpty()
    private val status = ServiceSettingsUi.label(activity, "", 16f).apply {
        tag = "app-update-status"
        visibility = View.GONE
        accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }
    private val check = MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
        tag = "app-update-check"
        setText(R.string.app_update_check)
        isAllCaps = false
        setOnClickListener { checkForUpdate() }
    }
    private val download = MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
        tag = "app-update-download"
        setText(R.string.app_update_download)
        isAllCaps = false
        visibility = View.GONE
        setOnClickListener { openDownload() }
    }

    init {
        orientation = VERTICAL
        tag = "app-updates"
        addView(ServiceSettingsUi.label(activity, activity.getString(R.string.app_update_version, installedVersion), 16f)
            .apply { tag = "app-update-version" }, LayoutParams(-1, -2))
        addView(check, LayoutParams(-2, -2).apply { gravity = Gravity.END })
        addView(status, LayoutParams(-1, -2))
        addView(download, LayoutParams(-2, -2).apply { gravity = Gravity.END })
        activity.lifecycle.addObserver(LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_STOP, Lifecycle.Event.ON_DESTROY -> cancel()
                Lifecycle.Event.ON_START -> refreshActions()
                else -> Unit
            }
        })
        refreshActions()
    }

    fun setPageVisible(visible: Boolean) {
        pageVisible = visible
        if (!visible) cancel()
        refreshActions()
    }

    fun clearNotice() { noticeGeneration++; toast?.cancel(); toast = null }

    private fun showNotice(message: Int) {
        clearNotice()
        if (active() && activity.hasWindowFocus()) {
            toast = Toast.makeText(activity, message, Toast.LENGTH_SHORT).also { it.show() }
        }
    }

    private fun active(): Boolean = pageVisible && activity.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)

    private fun refreshActions() {
        check.isEnabled = active() && !checking
        check.setText(if (checking) R.string.app_update_checking else R.string.app_update_check)
        download.isEnabled = active() && !checking
    }

    private fun cancel() {
        generation++
        pending?.cancel()
        pending = null
        checking = false
        callbacks.removeCallbacksAndMessages(null)
        clearNotice()
        refreshActions()
    }

    private fun checkForUpdate() {
        if (!active() || checking) return
        clearNotice()
        checking = true
        release = null
        status.visibility = View.GONE
        download.visibility = View.GONE
        refreshActions()
        val owner = ++generation
        val noticeOwner = noticeGeneration
        pending = checker.check(installedVersion) { result ->
            callbacks.post {
                if (owner != generation || !active()) return@post
                pending = null
                checking = false
                when (result) {
                    AppUpdateResult.UpToDate -> {
                        if (noticeOwner == noticeGeneration) showNotice(R.string.app_update_current)
                    }
                    is AppUpdateResult.Available -> {
                        release = result.release
                        status.text = activity.getString(R.string.app_update_available, result.release.version.toString())
                        status.visibility = View.VISIBLE
                        download.visibility = View.VISIBLE
                    }
                    AppUpdateResult.Failed -> {
                        status.setText(R.string.app_update_failed)
                        status.visibility = View.VISIBLE
                    }
                }
                refreshActions()
            }
        }
    }

    private fun openDownload() {
        if (!active() || checking) return
        val update = release ?: return
        try {
            activity.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(update.downloadUrl)))
        } catch (_: ActivityNotFoundException) {
            showNotice(R.string.app_update_open_failed)
        } catch (_: SecurityException) {
            showNotice(R.string.app_update_open_failed)
        }
    }
}
