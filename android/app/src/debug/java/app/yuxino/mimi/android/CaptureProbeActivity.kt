package app.yuxino.mimi.android

import android.app.Activity
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import android.widget.TextView
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.capture.MimiService

/** Debug-only consent host: never opens settings or reads service credentials. */
class CaptureProbeActivity : Activity() {
    @Volatile var requests = 0
        private set
    @Volatile var consentResult: Int? = null
        private set

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(TextView(this).apply { text = "Android system playback capture test" })
    }

    fun requestProjection() {
        requests++
        consentResult = null
        @Suppress("DEPRECATION")
        startActivityForResult(getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent(), 1)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != 1) return
        consentResult = resultCode
        if (resultCode == RESULT_OK && data != null) {
            ContextCompat.startForegroundService(this, MimiService.startIntent(this, resultCode, data)
                .setAction(MimiService.ACTION_CAPTURE_TEST))
        }
    }
}
