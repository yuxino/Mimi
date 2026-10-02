package app.yuxino.mimi.android.capture

import android.Manifest
import android.content.pm.PackageManager
import android.content.pm.ApplicationInfo
import androidx.core.content.ContextCompat
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.content.pm.ServiceInfo
import android.graphics.Color
import android.graphics.PixelFormat
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Handler
import android.os.IBinder
import android.os.Build
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.Gravity
import android.view.ContextThemeWrapper
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.TextView
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Toast
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.atomic.AtomicBoolean
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.SettingsStore
import app.yuxino.mimi.android.ImmersiveModeHelp
import app.yuxino.mimi.android.provider.DashScopeEngine
import app.yuxino.mimi.android.provider.EngineListener
import app.yuxino.mimi.android.provider.OpenAIRealtimeEngine
import app.yuxino.mimi.android.provider.ProviderEngine
import app.yuxino.mimi.android.provider.SubtitleBus
import app.yuxino.mimi.android.resample.StreamResampler
import kotlin.concurrent.thread

/**
 * Foreground service that owns the MediaProjection playback-capture loop and
 * the overlay subtitle window. One service, one lifecycle, one notification.
 */
class MimiService : Service() {

    private val mainHandler = Handler(Looper.getMainLooper())

    private var mediaProjection: MediaProjection? = null
    private var audioRecord: AudioRecord? = null
    private var captureThread: Thread? = null
    private var capturing: AtomicBoolean? = null
    private var health: CaptureHealth? = null
    private val healthTick = object : Runnable {
        override fun run() {
            if (health != null && (!android.provider.Settings.canDrawOverlays(this@MimiService) ||
                ContextCompat.checkSelfPermission(this@MimiService, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED ||
                getSystemService(android.app.KeyguardManager::class.java).isDeviceLocked ||
                !getSystemService(android.os.PowerManager::class.java).isInteractive)) {
                lastCaptureError = "capture.permission_or_lock_changed"
                stopEverything()
                return
            }
            health?.let {
                captureObservation = it.snapshot(SystemClock.elapsedRealtime())
                stateListeners.forEach { listener -> listener() }
                renderBus()
                mainHandler.postDelayed(this, 1_000)
            }
        }
    }
    private var generation = 0
    private var projectionCallback: MediaProjection.Callback? = null
    private var engine: ProviderEngine? = null
    private lateinit var immersiveHelp: ImmersiveModeHelp

    private var windowManager: WindowManager? = null
    private var overlayView: View? = null
    private var overlayParams: WindowManager.LayoutParams? = null
    private var immersiveExitView: View? = null
    private var immersiveExitParams: WindowManager.LayoutParams? = null
    private var immersiveExitYFraction = 0.42f
    private var compactView: View? = null
    private var expandedView: View? = null
    private var expanded = false
    private var compactYOffset = 0
    private var previewMode = false
    private var immersiveSession = false
    private var sessionSourceLanguage = "auto"
    private var statusView: TextView? = null
    private var historyView: TextView? = null
    private var sourceView: TextView? = null
    private var translationView: TextView? = null
    private var expandedStatusView: TextView? = null
    private var expandedSourceView: TextView? = null
    private var expandedTranslationView: TextView? = null

    private val busListener = object : SubtitleBus.Listener {
        override fun onSubtitleChanged() {
            mainHandler.post { renderBus() }
        }
    }

    /** Hides the live lines after the sentence-final lands and speech pauses. */
    private val autoHideRunnable = Runnable { SubtitleBus.hideLive() }

    /**
     * Fallback hide: providers may never send a sentence-final when background
     * music keeps their VAD from firing, so any gap in streaming deltas also
     * hides the card.
     */
    private val watchdogRunnable = Runnable { SubtitleBus.hideLive() }

    /**
     * Only a sentence-final starts the quick hide. Any streaming draft cancels
     * it, so event gaps inside one utterance can never blink the card.
     */
    private fun scheduleAutoHide() {
        mainHandler.removeCallbacks(autoHideRunnable)
        mainHandler.postDelayed(autoHideRunnable, AUTO_HIDE_MS)
    }

    private fun cancelAutoHide() {
        mainHandler.removeCallbacks(autoHideRunnable)
        // Keep the fallback watchdog armed across the whole session.
        mainHandler.removeCallbacks(watchdogRunnable)
        mainHandler.postDelayed(watchdogRunnable, WATCHDOG_MS)
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        immersiveHelp = ImmersiveModeHelp(ContextThemeWrapper(this, R.style.Theme_Mimi), overlayWindow = true)
        createChannel()
        SubtitleBus.addListener(busListener)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopEverything()
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_APPLY_APPEARANCE) {
            if (overlayView != null && immersiveSession != SettingsStore.immersiveSubtitles(this)) {
                rebuildOverlay()
            } else if (overlayView == null && !isRunning) stopSelf()
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_UI_PREVIEW_HISTORY && previewMode &&
            applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        ) {
            previewHistoryEnabled = true
            seedPreviewHistory()
            showExpandedOverlay()
            return START_NOT_STICKY
        }
        if (isRunning || overlayView != null) return START_NOT_STICKY
        val testEngine = if (intent?.action == ACTION_CAPTURE_TEST &&
            applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) captureEngineForTests else null
        if (intent?.action == ACTION_CAPTURE_TEST && testEngine == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        // Debug-only screenshot fixture: the real overlay with synthetic
        // subtitles and no MediaProjection, provider, or credential access.
        if (intent?.action == ACTION_UI_PREVIEW &&
            applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        ) {
            previewMode = true
            previewHistoryEnabled = false
            sessionSourceLanguage = "ja"
            SubtitleBus.clear()
            SubtitleBus.setHistoryLimit(0)
            SubtitleBus.onSourceDraft("もう少し歩いてみましょう。", "ja")
            SubtitleBus.onTranslationDraft("再往前走一会儿吧。")
            showOverlay()
            return START_NOT_STICKY
        }
        val resultCode = intent?.getIntExtra(EXTRA_RESULT_CODE, Int.MIN_VALUE) ?: Int.MIN_VALUE
        val resultData = intent?.compatGetParcelableExtra(EXTRA_RESULT_DATA)
        if (resultCode == Int.MIN_VALUE || resultData == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        // Official order on Android 14+: startForeground with the
        // mediaProjection type first — getMediaProjection() then requires the
        // FGS to already be running with that type, and startForeground's
        // token deadline is satisfied by calling it immediately after.
        val projectionManager =
            getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        try {
            lastCaptureError = null
            startAsForeground()
            val projection = checkNotNull(projectionManager.getMediaProjection(resultCode, resultData))
            mediaProjection = projection
            startCapture(projection, testEngine)
            setRunning(true)
        } catch (_: Exception) {
            lastCaptureError = "capture.start_failed"
            Log.w(TAG, "capture_start_failed")
            Toast.makeText(this, R.string.capture_failed, Toast.LENGTH_LONG).show()
            stopEverything()
        }
        return START_NOT_STICKY
    }

    private fun startAsForeground() {
        val stopIntent = PendingIntent.getService(
            this, 1,
            Intent(this, MimiService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_mimi)
            .setContentTitle(getString(R.string.notification_title))
            .setContentText(getString(R.string.notification_text))
            .setContentIntent(mainActivityIntent())
            .addAction(
                Notification.Action.Builder(
                    null, getString(R.string.stop_action), stopIntent,
                ).build(),
            )
            .setOngoing(true)
            .build()
        startForeground(
            NOTIFICATION_ID, notification,
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION,
        )
    }

    private fun mainActivityIntent(): PendingIntent =
        PendingIntent.getActivity(
            this, 0,
            Intent(this, app.yuxino.mimi.android.MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )

    private fun startCapture(projection: MediaProjection, testEngine: ProviderEngine? = null) {
        check(ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            "audio_permission_required"
        }
        check(android.provider.Settings.canDrawOverlays(this) &&
            getSystemService(android.os.PowerManager::class.java).isInteractive &&
            !getSystemService(android.app.KeyguardManager::class.java).isDeviceLocked) { "capture_permission_or_lock_changed" }
        val sessionGeneration = ++generation
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(SettingsStore.historyLines(this))
        val callback = object : MediaProjection.Callback() {
            override fun onStop() {
                if (generation == sessionGeneration) {
                    lastCaptureError = "capture.projection_stopped"
                    stopEverything()
                }
            }
        }
        projectionCallback = callback
        projection.registerCallback(callback, mainHandler)

        // Serialize provider callbacks with stop/start and ignore stale sessions.
        fun dispatch(action: () -> Unit) {
            mainHandler.post { if (generation == sessionGeneration) action() }
        }
        if (testEngine != null) {
            // Instrumentation exercises real playback capture without credentials or a provider connection.
            engine = testEngine
            sessionSourceLanguage = "auto"
            captureProjectionForTests = projection
        } else {
            val provider = SettingsStore.provider(this)
            val apiKey = SettingsStore.apiKey(this)
            val sourceLang = SettingsStore.sourceLang(this)
            sessionSourceLanguage = sourceLang
            val targetLang = SettingsStore.targetLang(this)
            val listener = object : EngineListener {
                override fun onSessionReady() = dispatch { Log.i(TAG, "session ready") }
                override fun onSourceDraft(text: String, language: String?) = dispatch {
                    cancelAutoHide()
                    SubtitleBus.onSourceDraft(text, language)
                }
                override fun onSourceFinal(text: String, language: String?) = dispatch {
                    cancelAutoHide()
                    SubtitleBus.onSourceFinal(text, language)
                }
                override fun onTranslationDraft(text: String) = dispatch {
                    cancelAutoHide()
                    SubtitleBus.onTranslationDraft(text)
                }
                override fun onTranslationFinal(text: String) = dispatch {
                    SubtitleBus.onTranslationFinal(text)
                    scheduleAutoHide()
                }
                override fun onError(code: String, message: String) = dispatch {
                    // Provider error bodies can echo user content or credentials.
                    Toast.makeText(this@MimiService, R.string.capture_failed, Toast.LENGTH_LONG).show()
                    stopEverything()
                }
                override fun onClosed() = dispatch { stopEverything() }
                override fun onLog(message: String) = Unit
            }
            engine = when (provider) {
                SettingsStore.PROVIDER_OPENAI -> OpenAIRealtimeEngine(listener)
                SettingsStore.PROVIDER_DASHSCOPE -> DashScopeEngine(listener)
                else -> app.yuxino.mimi.android.provider.StreamingServiceEngine(SettingsStore.configuration(this), listener)
            }
            engine?.setHotwords(SettingsStore.hotwords(this))
            engine?.start(
                apiKey, sourceLang, targetLang,
                SettingsStore.baseUrl(this, provider),
                SettingsStore.model(this, provider),
            )
        }

        // Playback capture at a fixed 48 kHz stereo float; the system resamples
        // whatever the apps actually play into this format for us.
        val captureConfig =
            AudioPlaybackCaptureConfiguration.Builder(projection)
                .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                .addMatchingUsage(AudioAttributes.USAGE_GAME)
                .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                .build()
        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
            .setSampleRate(CAPTURE_RATE_HZ)
            .setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
            .build()
        val minBuffer = AudioRecord.getMinBufferSize(
            CAPTURE_RATE_HZ, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_FLOAT,
        )
        check(minBuffer > 0) { "unsupported_capture_format" }
        val record = AudioRecord.Builder()
            .setAudioFormat(format)
            .setBufferSizeInBytes(minBuffer * 4)
            .setAudioPlaybackCaptureConfig(captureConfig)
            .build()
        audioRecord = record
        check(record.state == AudioRecord.STATE_INITIALIZED) { "capture_uninitialized" }

        val resampler = StreamResampler(CAPTURE_RATE_HZ, engine!!.sampleRateHz, 2)
        val readBuffer = FloatArray(CAPTURE_RATE_HZ / 10 * 2) // 100 ms of stereo

        val captureActive = AtomicBoolean(true)
        capturing = captureActive
        val sessionEngine = checkNotNull(engine)
        val sessionHealth = CaptureHealth(SystemClock.elapsedRealtime())
        synchronized(firstRunEvidence) { firstRunEvidence.reset() }
        health = sessionHealth
        captureObservation = sessionHealth.snapshot(SystemClock.elapsedRealtime())
        record.startRecording()
        check(record.recordingState == AudioRecord.RECORDSTATE_RECORDING) { "capture_not_started" }
        showOverlay()
        mainHandler.post(healthTick)
        captureThread = thread(name = "mimi-capture") {
            try {
                android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_URGENT_AUDIO)
                while (captureActive.get()) {
                    val read = record.read(readBuffer, 0, readBuffer.size, AudioRecord.READ_BLOCKING)
                    if (!captureActive.get()) break
                    check(read >= 0) { "capture_read_failed" }
                    if (read == 0) continue
                    val pcm = resampler.push(readBuffer.copyOf(read))
                    sessionHealth.observe(pcm, SystemClock.elapsedRealtime())
                    if (captureActive.get() && pcm.isNotEmpty()) {
                        sessionEngine.sendAudio(pcm)
                        synchronized(firstRunEvidence) { if (captureActive.get()) firstRunEvidence.submitted(pcm) }
                    }
                }
            } catch (_: Exception) {
                mainHandler.post {
                    if (generation == sessionGeneration) {
                        lastCaptureError = "capture.read_failed"
                        Toast.makeText(this, R.string.capture_failed, Toast.LENGTH_LONG).show()
                        stopEverything()
                    }
                }
            } finally {
                record.release()
            }
        }
    }

    private fun releaseSession() {
        immersiveHelp.dismiss()
        ++generation
        health = null
        captureObservation = null
        mainHandler.removeCallbacksAndMessages(null)
        capturing?.set(false)
        capturing = null
        // Stop blocking reads before waiting for the worker to finish.
        val record = audioRecord
        audioRecord = null
        try { record?.stop() } catch (_: Exception) { }
        val worker = captureThread
        captureThread = null
        if (worker != null) worker.join(600) else record?.release()
        synchronized(firstRunEvidence) { firstRunEvidence.reset() }
        engine?.stop()
        engine = null
        val projection = mediaProjection
        mediaProjection = null
        captureProjectionForTests = null
        projectionCallback?.let { projection?.unregisterCallback(it) }
        projectionCallback = null
        try { projection?.stop() } catch (_: Exception) { }
        SubtitleBus.clear()
        hideOverlay()
        previewMode = false
        setRunning(false)
    }

    private fun stopEverything() {
        releaseSession()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        SubtitleBus.removeListener(busListener)
        releaseSession()
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        mainHandler.post {
            if (expanded) {
                resizeExpandedOverlay(SubtitleBus.historySnapshot().size)
            }
            immersiveExitParams?.let { params ->
                params.y = exitControlY()
                immersiveExitView?.let { windowManager?.updateViewLayout(it, params) }
            }
        }
    }

    /** The actual floating window: a small live line that opens a bounded reading panel. */
    private fun showOverlay() {
        if (overlayView != null) return
        val wm = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        windowManager = wm

        immersiveSession = SettingsStore.immersiveSubtitles(this)
        expanded = false
        val bgAlpha = if (immersiveSession) 0 else SettingsStore.overlayBgAlpha(this)
        val compact = LinearLayout(this).apply {
            tag = "compact-subtitle"
            orientation = LinearLayout.VERTICAL
            val horizontalPadding = if (immersiveSession) 3 else 14
            setPadding(dp(horizontalPadding), dp(9), dp(horizontalPadding), dp(10))
            background = GradientDrawable().apply {
                cornerRadius = dp(12).toFloat()
                setColor(((bgAlpha / 100.0) * 255).toInt() shl 24 or 0x101010)
            }
            alpha = if (immersiveSession) 1f else SettingsStore.overlayOpacity(this@MimiService) / 100f
        }

        val status = TextView(this).apply {
            setTextColor(0xFFADADAD.toInt())
            textSize = 11f
            visibility = View.GONE
        }
        val source = TextView(this).apply {
            setTextColor(0xFFE7E7E7.toInt())
            textSize = SettingsStore.fontSize(this@MimiService).toFloat()
            maxLines = 2
            maxWidth = (resources.displayMetrics.widthPixels * 0.88f).toInt()
            if (immersiveSession) setShadowLayer(dp(4).toFloat(), 0f, dp(1).toFloat(), Color.BLACK)
        }
        val translation = TextView(this).apply {
            setTextColor(SettingsStore.translationColor(this@MimiService))
            textSize = (SettingsStore.fontSize(this@MimiService) + 3).toFloat()
            setTypeface(typeface, Typeface.BOLD)
            maxLines = 3
            if (immersiveSession) setShadowLayer(dp(4).toFloat(), 0f, dp(1).toFloat(), Color.BLACK)
            maxWidth = (resources.displayMetrics.widthPixels * 0.88f).toInt()
        }

        compact.addView(status)
        compact.addView(source)
        compact.addView(translation)
        val panel = if (immersiveSession) null else buildExpandedPanel()
        val root = FrameLayout(this).apply {
            tag = "mimi-overlay"
            addView(compact, FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.CENTER,
            ))
            panel?.let {
                it.visibility = View.GONE
                addView(it, FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT,
                ))
            }
        }

        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
                or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL
                or (if (immersiveSession) WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE else 0),
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
            y = dp(SettingsStore.overlayYOffset(this@MimiService))
            // Android 12+ passes touches through an untrusted overlay only when
            // its window opacity stays at or below the system threshold (0.8).
            if (immersiveSession) alpha = 0.8f
        }
        compactYOffset = params.y

        var initialY = 0
        var initialTouchY = 0f
        var dragged = false
        if (!immersiveSession) compact.setOnClickListener { showExpandedOverlay() }
        if (!immersiveSession) compact.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    initialY = params.y
                    initialTouchY = event.rawY
                    dragged = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    if (kotlin.math.abs(event.rawY - initialTouchY) > dp(8)) dragged = true
                    if (dragged) {
                        params.y = (initialY - (event.rawY - initialTouchY)).toInt().coerceAtLeast(0)
                        wm.updateViewLayout(root, params)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (dragged) {
                        compactYOffset = params.y
                        SettingsStore.setOverlayYOffset(
                            this, (params.y / resources.displayMetrics.density).toInt(),
                        )
                    } else compact.performClick()
                    true
                }
                MotionEvent.ACTION_CANCEL -> true
                else -> false
            }
        }

        wm.addView(root, params)
        overlayView = root
        overlayParams = params
        compactView = compact
        expandedView = panel
        statusView = status
        sourceView = source
        translationView = translation
        // Observe the real overlay drawing once per overlay, without accumulating listeners.
        val renderGeneration = generation
        overlayView?.viewTreeObserver?.addOnDrawListener {
            val caption = if (expanded) expandedTranslationView else translationView
            val newlyComplete = synchronized(firstRunEvidence) {
                val wasComplete = firstRunEvidence.complete
                if (generation == renderGeneration && isRunning && caption != null) {
                    firstRunEvidence.rendered(!caption.text.isNullOrBlank(), caption.isShown &&
                        caption.width > 0 && caption.height > 0 && android.provider.Settings.canDrawOverlays(this), previewMode)
                }
                !wasComplete && firstRunEvidence.complete
            }
            if (newlyComplete) mainHandler.post {
                getSharedPreferences("first_run", 0).edit().putBoolean("completed", true).apply()
                stateListeners.forEach { it() }
            }
        }
        if (immersiveSession) showImmersiveExitControl(wm)
        renderBus()
    }

    private fun showImmersiveExitControl(wm: WindowManager) {
        val exit = panelButton(getString(R.string.overlay_exit_short)).apply {
            tag = "exit-immersive"
            contentDescription = getString(R.string.overlay_exit_immersive)
            alpha = 0.68f
            setOnClickListener { setImmersiveMode(false) }
        }
        val params = WindowManager.LayoutParams(
            dp(56), dp(40), WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or Gravity.END
            x = dp(4)
            y = exitControlY()
        }
        var initialY = 0
        var initialTouchY = 0f
        var dragged = false
        exit.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    initialY = params.y
                    initialTouchY = event.rawY
                    dragged = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    if (kotlin.math.abs(event.rawY - initialTouchY) > dp(8)) dragged = true
                    if (dragged) {
                        params.y = (initialY + event.rawY - initialTouchY).toInt()
                            .coerceIn(0, (resources.displayMetrics.heightPixels - dp(48)).coerceAtLeast(0))
                        wm.updateViewLayout(exit, params)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    if (dragged) {
                        immersiveExitYFraction = params.y.toFloat() /
                            resources.displayMetrics.heightPixels.coerceAtLeast(1)
                    } else exit.performClick()
                    true
                }
                MotionEvent.ACTION_CANCEL -> true
                else -> false
            }
        }
        wm.addView(exit, params)
        immersiveExitView = exit
        immersiveExitParams = params
    }

    private fun exitControlY(): Int =
        (resources.displayMetrics.heightPixels * immersiveExitYFraction).toInt()
            .coerceIn(0, (resources.displayMetrics.heightPixels - dp(48)).coerceAtLeast(0))

    private fun setImmersiveMode(enabled: Boolean) {
        if (immersiveSession == enabled) return
        if (enabled) {
            immersiveHelp.requestEnable(onConfirmed = { applyImmersiveMode(true) })
        } else {
            immersiveHelp.dismiss()
            runCatching { applyImmersiveMode(false) }
        }
    }

    private fun applyImmersiveMode(enabled: Boolean) {
        SettingsStore.setImmersiveSubtitles(this, enabled)
        rebuildOverlay()
    }

    private fun rebuildOverlay() {
        hideOverlay()
        showOverlay()
    }

    private fun buildExpandedPanel(): View {
        val panel = LinearLayout(this).apply {
            tag = "expanded-subtitles"
            orientation = LinearLayout.VERTICAL
            setPadding(dp(18), dp(14), dp(18), dp(18))
            background = GradientDrawable().apply {
                cornerRadius = dp(22).toFloat()
                setColor(0xD91B1B1B.toInt())
            }
        }
        val header = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL }
        val collapse = panelButton(getString(R.string.overlay_collapse)).apply {
            tag = "collapse-overlay"
            contentDescription = getString(R.string.overlay_collapse_description)
            setOnClickListener { collapseOverlay() }
        }
        val font = panelButton("Aa").apply {
            contentDescription = getString(R.string.overlay_font_description)
            setOnClickListener {
                val current = SettingsStore.fontSize(this@MimiService)
                SettingsStore.setFontSize(this@MimiService, if (current >= 22) 16 else current + 2)
                updateOverlayFontSize()
            }
        }
        val immersive = panelButton(getString(R.string.overlay_enter_immersive)).apply {
            tag = "enter-immersive"
            contentDescription = getString(R.string.overlay_enter_immersive)
            setOnClickListener { setImmersiveMode(true) }
        }
        val route = panelButton(
            "${languageName(sessionSourceLanguage)} → ${languageName(SettingsStore.targetLang(this))}",
        ).apply {
            contentDescription = getString(R.string.overlay_language_description)
            maxLines = 1
            ellipsize = android.text.TextUtils.TruncateAt.END
            setOnClickListener {
                startActivity(Intent(this@MimiService, app.yuxino.mimi.android.MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        }
        header.addView(collapse, LinearLayout.LayoutParams(dp(58), dp(38)))
        header.addView(route, LinearLayout.LayoutParams(0, dp(38), 1f).apply { marginStart = dp(8) })
        header.addView(font, LinearLayout.LayoutParams(dp(42), dp(38)).apply { marginStart = dp(8) })
        header.addView(immersive, LinearLayout.LayoutParams(dp(58), dp(38)).apply { marginStart = dp(8) })
        panel.addView(header)

        expandedStatusView = TextView(this).apply {
            setTextColor(0xFFB9B9B9.toInt())
            textSize = 12f
            visibility = View.GONE
        }
        panel.addView(expandedStatusView, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(10) })

        val scroll = ScrollView(this).apply { isFillViewport = true }
        val transcript = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        historyView = TextView(this).apply {
            setTextColor(0xFFB9B9B9.toInt())
            textSize = (SettingsStore.fontSize(this@MimiService) - 1).coerceAtLeast(12).toFloat()
            setLineSpacing(dp(5).toFloat(), 1f)
        }
        transcript.addView(historyView)
        scroll.addView(transcript)
        panel.addView(scroll, LinearLayout.LayoutParams(-1, 0, 1f).apply { topMargin = dp(18) })
        panel.addView(View(this).apply { setBackgroundColor(0xFF555555.toInt()) },
            LinearLayout.LayoutParams(-1, dp(1)))
        expandedSourceView = TextView(this).apply {
            setTextColor(0xFFD5D5D5.toInt())
            textSize = SettingsStore.fontSize(this@MimiService).toFloat()
            maxLines = 2
        }
        expandedTranslationView = TextView(this).apply {
            setTextColor(SettingsStore.translationColor(this@MimiService))
            textSize = (SettingsStore.fontSize(this@MimiService) + 3).toFloat()
            setTypeface(typeface, Typeface.BOLD)
            maxLines = 3
        }
        panel.addView(expandedSourceView, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(14) })
        panel.addView(expandedTranslationView, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(6) })
        return panel
    }

    private var previewHistoryEnabled = false

    private fun seedPreviewHistory() {
        SubtitleBus.setHistoryLimit(3)
        listOf(
            "少し待ってください。" to "请稍等一下。",
            "今日はいい天気ですね。" to "今天天气真好。",
            "次はどこへ行きますか？" to "接下来去哪里？",
        ).forEach { (source, translation) ->
            SubtitleBus.onSourceFinal(source, "ja")
            SubtitleBus.onTranslationFinal(translation)
        }
        SubtitleBus.onSourceDraft("もう少し歩いてみましょう。", "ja")
        SubtitleBus.onTranslationDraft("再往前走一会儿吧。")
    }

    private fun panelButton(label: String): TextView = TextView(this).apply {
        text = label
        textSize = 13f
        setTypeface(typeface, Typeface.BOLD)
        setTextColor(Color.WHITE)
        gravity = Gravity.CENTER
        setPadding(dp(8), 0, dp(8), 0)
        background = GradientDrawable().apply {
            cornerRadius = dp(24).toFloat()
            setColor(0xFF414141.toInt())
        }
        isClickable = true
        isFocusable = true
    }

    private fun languageName(code: String): String = getString(when (code) {
        "zh" -> R.string.lang_zh
        "en" -> R.string.lang_en
        "ja" -> R.string.lang_ja
        "ko" -> R.string.lang_ko
        else -> R.string.lang_auto
    })

    private fun updateOverlayFontSize() {
        val size = SettingsStore.fontSize(this).toFloat()
        sourceView?.textSize = size
        translationView?.textSize = size + 3
        expandedSourceView?.textSize = size
        expandedTranslationView?.textSize = size + 3
        historyView?.textSize = (size - 1).coerceAtLeast(12f)
    }

    private fun showExpandedOverlay() {
        val root = overlayView ?: return
        val params = overlayParams ?: return
        val panel = expandedView ?: return
        compactYOffset = params.y
        compactView?.visibility = View.GONE
        panel.visibility = View.VISIBLE
        expanded = true
        params.width = expandedPanelWidth()
        params.height = expandedPanelHeight(0)
        params.gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
        params.y = 0
        windowManager?.updateViewLayout(root, params)
        renderBus()
    }

    private fun resizeExpandedOverlay(historyCount: Int) {
        val root = overlayView ?: return
        val params = overlayParams ?: return
        val desiredWidth = expandedPanelWidth()
        val desiredHeight = expandedPanelHeight(historyCount)
        if (params.width == desiredWidth && params.height == desiredHeight) return
        params.width = desiredWidth
        params.height = desiredHeight
        windowManager?.updateViewLayout(root, params)
    }

    private fun expandedPanelWidth(): Int =
        (resources.displayMetrics.widthPixels * 0.92f).toInt().coerceAtMost(dp(560))

    private fun expandedPanelHeight(historyCount: Int): Int {
        val landscape = resources.displayMetrics.widthPixels > resources.displayMetrics.heightPixels
        val maxFraction = if (landscape && historyCount == 0) 0.48f
            else if (landscape) 0.62f else 0.64f
        return dp(230 + historyCount * 76)
            .coerceAtMost((resources.displayMetrics.heightPixels * maxFraction).toInt())
    }

    private fun collapseOverlay() {
        val root = overlayView ?: return
        val params = overlayParams ?: return
        expandedView?.visibility = View.GONE
        compactView?.visibility = View.VISIBLE
        expanded = false
        params.width = WindowManager.LayoutParams.WRAP_CONTENT
        params.height = WindowManager.LayoutParams.WRAP_CONTENT
        params.gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
        params.y = compactYOffset
        windowManager?.updateViewLayout(root, params)
        renderBus()
    }

    private fun hideOverlay() {
        try {
            immersiveExitView?.let { windowManager?.removeView(it) }
        } catch (_: Exception) {
        }
        immersiveExitView = null
        immersiveExitParams = null
        try {
            overlayView?.let { windowManager?.removeView(it) }
        } catch (_: Exception) {
        }
        overlayView = null
        overlayParams = null
        compactView = null
        expandedView = null
        expanded = false
        statusView = null
        historyView = null
        sourceView = null
        translationView = null
        expandedStatusView = null
        expandedSourceView = null
        expandedTranslationView = null
    }

    private fun renderBus() {
        val observation = captureObservation?.state
        val statusLine = if (observation == CaptureHealth.State.NO_PCM || observation == CaptureHealth.State.SILENT) {
            getString(R.string.capture_no_sound_hint)
        } else SubtitleBus.statusLine
        statusView?.apply {
            visibility = if (statusLine.isEmpty()) View.GONE else View.VISIBLE
            text = statusLine
        }
        expandedStatusView?.apply {
            visibility = if (statusLine.isEmpty()) View.GONE else View.VISIBLE
            text = statusLine
        }
        val maxHistory = if (previewMode) (if (previewHistoryEnabled) 3 else 0)
            else SettingsStore.historyLines(this)
        val history = if (maxHistory > 0) SubtitleBus.historySnapshot().takeLast(maxHistory) else emptyList()
        historyView?.apply {
            visibility = if (history.isEmpty()) View.GONE else View.VISIBLE
            maxLines = maxOf(maxHistory, 1) * 3
            text = history.joinToString("\n\n") { pair ->
                "${pair.source}\n${pair.translation}"
            }
        }
        // Live display: translation line always; the source line joins only
        // when the speech is English (detected or configured).
        val liveVisible = !SubtitleBus.liveHidden
        val sourceIsEnglish =
            sessionSourceLanguage == "en" ||
                SubtitleBus.detectedSourceLanguage?.startsWith("en") == true
        sourceView?.apply {
            visibility = if (liveVisible && sourceIsEnglish) View.VISIBLE else View.GONE
            text = when {
                SubtitleBus.sourceDraft.isNotEmpty() -> SubtitleBus.sourceDraft
                else -> SubtitleBus.sourceFinal
            }
        }
        translationView?.apply {
            visibility = if (liveVisible) View.VISIBLE else View.GONE
            text = when {
                SubtitleBus.translationDraft.isNotEmpty() -> SubtitleBus.translationDraft
                else -> SubtitleBus.translationFinal
            }
        }
        expandedSourceView?.apply {
            visibility = if (liveVisible && (SubtitleBus.sourceDraft.isNotEmpty() || SubtitleBus.sourceFinal.isNotEmpty()))
                View.VISIBLE else View.GONE
            text = SubtitleBus.sourceDraft.ifEmpty { SubtitleBus.sourceFinal }
        }
        expandedTranslationView?.apply {
            visibility = if (liveVisible) View.VISIBLE else View.GONE
            text = SubtitleBus.translationDraft.ifEmpty { SubtitleBus.translationFinal }
        }
        if (expanded) resizeExpandedOverlay(history.size)
        overlayView?.visibility = if (expanded || liveVisible || statusLine.isNotEmpty())
            View.VISIBLE else View.GONE
    }

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density).toInt()

    private fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.notification_channel),
            NotificationManager.IMPORTANCE_LOW,
        )
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    companion object {
        @Volatile private var captureEngineForTests: ProviderEngine? = null
        @Volatile internal var captureProjectionForTests: MediaProjection? = null
            private set

        /** The release app cannot replace its provider or expose its projection through this seam. */
        internal fun setCaptureEngineForTests(context: Context, value: ProviderEngine?) {
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!isRunning)
            captureEngineForTests = value
        }

        val firstRunEvidence = app.yuxino.mimi.android.FirstRunEvidence()
        private const val TAG = "MimiService"
        private const val CHANNEL_ID = "capture"
        private const val NOTIFICATION_ID = 41
        private const val CAPTURE_RATE_HZ = 48_000
        private const val AUTO_HIDE_MS = 600L
        private const val WATCHDOG_MS = 3_000L
        @Volatile var isRunning: Boolean = false
            private set
        @Volatile var captureObservation: CaptureHealth.Snapshot? = null
            private set
        @Volatile var lastCaptureError: String? = null
            private set
        private val stateListeners = CopyOnWriteArraySet<() -> Unit>()
        fun addStateListener(listener: () -> Unit) { stateListeners.add(listener) }
        fun removeStateListener(listener: () -> Unit) { stateListeners.remove(listener) }
        private fun setRunning(value: Boolean) {
            isRunning = value
            stateListeners.forEach { it() }
        }

        const val ACTION_STOP = "app.yuxino.mimi.android.action.STOP"
        const val ACTION_APPLY_APPEARANCE = "app.yuxino.mimi.android.action.APPLY_APPEARANCE"
        const val ACTION_UI_PREVIEW = "app.yuxino.mimi.android.action.UI_PREVIEW"
        internal const val ACTION_CAPTURE_TEST = "app.yuxino.mimi.android.action.CAPTURE_TEST"
        const val ACTION_UI_PREVIEW_HISTORY = "app.yuxino.mimi.android.action.UI_PREVIEW_HISTORY"
        const val EXTRA_RESULT_CODE = "result_code"
        const val EXTRA_RESULT_DATA = "result_data"

        fun startIntent(context: Context, resultCode: Int, resultData: Intent): Intent =
            Intent(context, MimiService::class.java)
                .putExtra(EXTRA_RESULT_CODE, resultCode)
                .putExtra(EXTRA_RESULT_DATA, resultData)

        fun stopIntent(context: Context): Intent =
            Intent(context, MimiService::class.java).setAction(ACTION_STOP)

        private fun Intent.compatGetParcelableExtra(key: String): Intent? =
            if (Build.VERSION.SDK_INT >= 33) {
                getParcelableExtra(key, Intent::class.java)
            } else {
                @Suppress("DEPRECATION")
                getParcelableExtra(key) as? Intent
            }
    }
}
