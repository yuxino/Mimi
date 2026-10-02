package app.yuxino.mimi.android

import android.content.Context
import android.content.DialogInterface
import android.graphics.drawable.GradientDrawable
import android.provider.Settings
import android.view.WindowManager
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.core.content.ContextCompat
import com.google.android.material.dialog.MaterialAlertDialogBuilder

/** Each entry point owns one explanation and enables immersive mode only after acknowledgement. */
internal class ImmersiveModeHelp(private val context: Context, private val overlayWindow: Boolean = false) {
    private var pendingDialog: AlertDialog? = null

    fun requestEnable(onConfirmed: () -> Unit, onCancelled: () -> Unit = {}) {
        if (pendingDialog != null) return
        val prefs = runCatching { context.getSharedPreferences("first_run", Context.MODE_PRIVATE) }
            .getOrElse { runCatching(onCancelled); return }
        val seen = runCatching { prefs.getBoolean("immersive_seen", false) }
            .getOrElse { runCatching(onCancelled); return }
        if (seen) {
            runCatching(onConfirmed).onFailure { showSaveFailure(); runCatching(onCancelled) }
            return
        }

        var confirmed = false
        val surface = GradientDrawable().apply {
            cornerRadius = 28f * context.resources.displayMetrics.density
        }
        val next = runCatching {
            MaterialAlertDialogBuilder(context)
                .setBackground(surface)
                .setTitle(R.string.guide_immersive)
                .setMessage(R.string.guide_immersive_hint)
                .setPositiveButton(R.string.guide_understood, null)
                .setNegativeButton(android.R.string.cancel, null)
                .create()
        }.getOrElse { runCatching(onCancelled); return }
        next.setOnDismissListener {
            if (pendingDialog === next) {
                pendingDialog = null
                if (!confirmed) runCatching(onCancelled)
            }
        }
        pendingDialog = next
        try {
            if (overlayWindow) {
                check(Settings.canDrawOverlays(context))
                checkNotNull(next.window).setType(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY)
            }
            next.show()
            // AppCompat applies the dialog's night configuration during show(), including for a service host.
            surface.setColor(ContextCompat.getColor(next.context, R.color.mimi_surface))
            next.getButton(DialogInterface.BUTTON_POSITIVE).setOnClickListener {
                if (pendingDialog !== next) return@setOnClickListener
                confirmed = runCatching {
                    if (overlayWindow) check(Settings.canDrawOverlays(context))
                    // This records reading the help, independently of applying the appearance setting.
                    check(prefs.edit().putBoolean("immersive_seen", true).commit())
                    onConfirmed()
                }.onFailure { showSaveFailure() }.isSuccess
                next.dismiss()
            }
        } catch (_: Exception) {
            dismiss()
            runCatching(onCancelled)
        }
    }

    private fun showSaveFailure() {
        runCatching { Toast.makeText(context, R.string.service_save_failed, Toast.LENGTH_LONG).show() }
    }

    /** Host destruction must not invoke callbacks that update its old views. */
    fun dismiss() {
        val current = pendingDialog
        pendingDialog = null
        runCatching { current?.dismiss() }
    }
}
