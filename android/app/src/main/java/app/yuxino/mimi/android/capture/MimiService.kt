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
import android.graphics.Rect
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
import android.view.WindowInsets
import android.widget.TextView
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Toast
import android.widget.SeekBar
import androidx.appcompat.app.AlertDialog
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.atomic.AtomicBoolean
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.SettingsStore
import app.yuxino.mimi.android.ImmersiveModeHelp
import app.yuxino.mimi.android.InterfaceLanguage
import app.yuxino.mimi.android.provider.EngineListener
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
    private var finishingSession = false
    private var projectionCallback: MediaProjection.Callback? = null
    private var engine: ProviderEngine? = null
    private lateinit var immersiveHelp: ImmersiveModeHelp

    private var windowManager: WindowManager? = null
    private var overlayView: View? = null
    private var overlayParams: WindowManager.LayoutParams? = null
    private var immersiveExitView: View? = null
    private var immersiveExitParams: WindowManager.LayoutParams? = null
    private var compactView: View? = null
    private var expandedView: View? = null
    private val overlayPlacement = OverlayPlacement()
    private val expanded get() = overlayPlacement.isExpanded
    private val immersiveSession get() = overlayPlacement.isImmersive
    private var draggingCompact = false
    private var previewMode = false
    private var sessionSourceLanguage = "auto"
    private var sessionTargetLanguage = "zh"
    private var sessionOriginalOnly = false
    private var statusView: TextView? = null
    private var historyView: TextView? = null
    private var sourceView: TextView? = null
    private var translationView: TextView? = null
    private var expandedStatusView: TextView? = null
    private var expandedSourceView: TextView? = null
    private var expandedTranslationView: TextView? = null
    private var transcriptScrollView: ScrollView? = null
    private var relayoutExpandedHeader: (() -> Unit)? = null
    private var scrollToCurrentOnLayout = false
    private var interfaceLocaleTags = ""
    private val interfaceLanguageListener: () -> Unit = { mainHandler.post { refreshInterfaceLanguage() } }
    private var stopObservingAppearance: (() -> Unit)? = null
    private var fontDialog: AlertDialog? = null
    private val appearanceUpdate = Runnable { applyAppearance() }

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
        immersiveHelp = ImmersiveModeHelp(ContextThemeWrapper(InterfaceLanguage.context(this), R.style.Theme_Mimi), overlayWindow = true)
        interfaceLocaleTags = InterfaceLanguage.context(this).resources.configuration.locales.toLanguageTags()
        InterfaceLanguage.addListener(interfaceLanguageListener)
        stopObservingAppearance = SettingsStore.observeAppearance(this) {
            mainHandler.removeCallbacks(appearanceUpdate)
            mainHandler.post(appearanceUpdate)
        }
        createChannel()
        SubtitleBus.addListener(busListener)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            finishSession()
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_APPLY_APPEARANCE) {
            applyAppearance()
            if (overlayView == null && !isRunning) stopSelf()
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
            Toast.makeText(InterfaceLanguage.context(this), R.string.capture_failed, Toast.LENGTH_LONG).show()
            stopEverything()
        }
        return START_NOT_STICKY
    }

    private fun startAsForeground() {
        startForeground(
            NOTIFICATION_ID, notification(),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION,
        )
    }

    private fun notification(): Notification {
        val stopIntent = PendingIntent.getService(
            this, 1,
            Intent(this, MimiService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_mimi)
            .setContentTitle(interfaceString(R.string.notification_title))
            .setContentText(interfaceString(R.string.notification_text))
            .setContentIntent(mainActivityIntent())
            .addAction(
                Notification.Action.Builder(
                    null, interfaceString(R.string.stop_action), stopIntent,
                ).build(),
            )
            .setOngoing(true)
            .build()
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
        sessionOriginalOnly = false
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
            val configuration = SettingsStore.runtimeConfiguration(this)
            sessionSourceLanguage = configuration.getString("sourceLanguage")
            sessionTargetLanguage = configuration.getString("targetLanguage")
            activeModelNames = app.yuxino.mimi.android.runtimeModelNames(configuration)
            sessionOriginalOnly = configuration.getString("targetLanguage") == "original"
            val listener = object : EngineListener {
                override fun onRuntimeSnapshot(state: org.json.JSONObject, originalOnly: Boolean) = dispatch {
                    SubtitleBus.onRuntimeSnapshot(state, originalOnly)
                    if (SubtitleBus.displayPairFinal) scheduleAutoHide() else cancelAutoHide()
                }
                override fun onSessionReady() = Unit
                override fun onSourceDraft(text: String, language: String?) = Unit
                override fun onSourceFinal(text: String, language: String?) = Unit
                override fun onTranslationDraft(text: String) = Unit
                override fun onTranslationFinal(text: String) = Unit
                override fun onError(code: String, message: String) = dispatch {
                    Toast.makeText(InterfaceLanguage.context(this@MimiService), providerErrorMessageResource(code), Toast.LENGTH_LONG).show()
                    stopEverything()
                }
                override fun onClosed() = dispatch { if (!finishingSession) stopEverything() }
                override fun onLog(message: String) = Unit
            }
            engine = app.yuxino.mimi.android.provider.SharedRuntimeEngine(configuration, listener)
            engine?.start("", sessionSourceLanguage, configuration.getString("targetLanguage"))

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
                        Toast.makeText(InterfaceLanguage.context(this), R.string.capture_failed, Toast.LENGTH_LONG).show()
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
        fontDialog?.dismiss()
        fontDialog = null
        ++generation
        finishingSession = false
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
        activeModelNames = emptyList()
        setRunning(false)
    }

    /** User stop drains accepted work; revocation/errors/destruction still abort. */
    private fun finishSession() {
        if (finishingSession) return
        if (engine == null || previewMode) { stopEverything(); return }
        finishingSession = true
        val owner = generation
        capturing?.set(false)
        try { audioRecord?.stop() } catch (_: Exception) { }
        mainHandler.removeCallbacks(healthTick)
        cancelAutoHide()
        val policy = app.yuxino.mimi.android.provider.NativeRuntimeConfiguration.policy
        // Cover audio drain, bounded provider finish, disconnect and publication;
        // the provider-only timeout can expire before Rust publishes its tail.
        mainHandler.postDelayed({ if (generation == owner && finishingSession) stopEverything() }, policy.getLong("sessionFinishTimeoutMs"))
        engine?.finish {
            mainHandler.post {
                if (generation != owner || !finishingSession) return@post
                val publishAndClose = {
                    mainHandler.post {
                        if (generation == owner && finishingSession) {
                            renderBus()
                            // Let the native snapshot publication run before its overlay is retired.
                            mainHandler.post { if (generation == owner && finishingSession) stopEverything() }
                        }
                    }
                    Unit
                }
                publishAndClose()
            }
        }
    }

    private fun stopEverything() {
        releaseSession()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        InterfaceLanguage.removeListener(interfaceLanguageListener)
        stopObservingAppearance?.invoke()
        stopObservingAppearance = null
        SubtitleBus.removeListener(busListener)
        releaseSession()
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        mainHandler.post {
            refreshInterfaceLanguage()
            applyAppearance()
        }
    }

    private fun refreshInterfaceLanguage() {
        val localeTags = InterfaceLanguage.context(this).resources.configuration.locales.toLanguageTags()
        if (localeTags == interfaceLocaleTags) return
        interfaceLocaleTags = localeTags
        immersiveHelp.dismiss()
        fontDialog?.dismiss()
        immersiveHelp = ImmersiveModeHelp(ContextThemeWrapper(InterfaceLanguage.context(this), R.style.Theme_Mimi), overlayWindow = true)
        fun label(tag: String, text: Int?, description: Int) {
            val control = (overlayView?.findViewWithTag<View>(tag)
                ?: immersiveExitView?.takeIf { it.tag == tag }) as? TextView ?: return
            if (text != null) control.text = interfaceString(text)
            control.contentDescription = interfaceString(description)
        }
        label("collapse-overlay", R.string.overlay_collapse, R.string.overlay_collapse_description)
        label("enter-immersive", R.string.overlay_enter_immersive, R.string.overlay_enter_immersive)
        label("overlay-font", null, R.string.overlay_font_description)
        updateOverlayFontSize()
        label("exit-immersive", R.string.overlay_exit_short, R.string.overlay_exit_immersive)
        label("overlay-route", null, R.string.overlay_language_description)
        overlayView?.findViewWithTag<TextView>("overlay-route")?.text =
            if (sessionOriginalOnly) languageName(sessionSourceLanguage)
            else "${languageName(sessionSourceLanguage)} → ${languageName(sessionTargetLanguage)}"
        relayoutExpandedHeader?.invoke()
        renderBus()
        if (isRunning) {
            createChannel()
            getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification())
        }
    }

    /** The actual floating window: a small live line that opens a bounded reading panel. */
    private fun showOverlay() {
        if (overlayView != null) return
        val displayManager = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        @Suppress("DEPRECATION")
        val overlayContext = if (Build.VERSION.SDK_INT >= 30) {
            createDisplayContext(displayManager.defaultDisplay)
                .createWindowContext(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY, null)
        } else this
        val wm = overlayContext.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        windowManager = wm

        overlayPlacement.reset(SettingsStore.overlayYOffset(this), SettingsStore.immersiveSubtitles(this))
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
            textSize = 14f
            maxWidth = (resources.displayMetrics.widthPixels * 0.88f).toInt()
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
        val panel = buildExpandedPanel()
        val root = FrameLayout(this).apply {
            tag = "mimi-overlay"
            addView(compact, FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.CENTER,
            ))
            panel.visibility = View.GONE
            addView(panel, FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT,
            ))
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
            y = overlayPlacement.compactY(resources.displayMetrics.density, overlayAvailableSize().second, 0)
            fitOverlayInsets(this)
            // Android 12+ passes touches through an untrusted overlay only when
            // its window opacity stays at or below the system threshold (0.8).
            if (immersiveSession) alpha = 0.8f
        }

        var initialY = 0
        var initialTouchY = 0f
        var dragged = false
        compact.setOnClickListener { showExpandedOverlay() }
        compact.setOnTouchListener { _, event ->
            if (immersiveSession) return@setOnTouchListener false
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    initialY = params.y
                    initialTouchY = event.rawY
                    dragged = false
                    draggingCompact = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    if (kotlin.math.abs(event.rawY - initialTouchY) > dp(8)) dragged = true
                    if (dragged) {
                        draggingCompact = true
                        params.y = OverlayPlacement.clampY(
                            (initialY - (event.rawY - initialTouchY)).toInt(), overlayAvailableSize().second, root.height,
                        )
                        wm.updateViewLayout(root, params)
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    draggingCompact = false
                    if (dragged) {
                        val offsetDp = kotlin.math.round(params.y / resources.displayMetrics.density).toInt()
                        overlayPlacement.setPreferredOffsetDp(offsetDp)
                        SettingsStore.setOverlayYOffset(this, offsetDp)
                    } else compact.performClick()
                    true
                }
                MotionEvent.ACTION_CANCEL -> {
                    draggingCompact = false
                    updateOverlayGeometry()
                    true
                }
                else -> false
            }
        }

        root.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
            if (overlayView === root) updateOverlayGeometry()
        }
        root.setOnApplyWindowInsetsListener { _, insets ->
            mainHandler.post { if (overlayView === root) updateOverlayGeometry() }
            insets
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
            val caption = when {
                sessionOriginalOnly && expanded -> expandedSourceView
                sessionOriginalOnly -> sourceView
                expanded -> expandedTranslationView
                else -> translationView
            }
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
        if (immersiveExitView != null) return
        val exit = panelButton(interfaceString(R.string.overlay_exit_short)).apply {
            tag = "exit-immersive"
            contentDescription = interfaceString(R.string.overlay_exit_immersive)
            minWidth = dp(48)
            minHeight = dp(48)
            alpha = 0.68f
            setOnClickListener { setImmersiveMode(false) }
            addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
                if (immersiveExitView === this) updateOverlayGeometry()
            }
        }
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT, WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT,
        ).apply {
            // A tiny touchable companion keeps the caption itself touch-through.
            // WindowManager fits both windows to the same safe display frame.
            gravity = Gravity.BOTTOM or Gravity.RIGHT
            x = dp(4)
            y = exitControlY()
            fitOverlayInsets(this)
        }
        wm.addView(exit, params)
        immersiveExitView = exit
        immersiveExitParams = params
        updateOverlayGeometry()
    }

    private fun exitControlY(): Int = OverlayPlacement.exitY(
        overlayParams?.y ?: 0, compactView?.height ?: 0, overlayAvailableSize().second,
        immersiveExitView?.height?.takeIf { it > 0 } ?: dp(48), dp(6),
    )

    private fun setImmersiveMode(enabled: Boolean) {
        if (immersiveSession == enabled) return
        if (enabled) {
            immersiveHelp.requestEnable(onConfirmed = { applyImmersiveMode(true) })
        } else {
            immersiveHelp.dismiss()
            runCatching { applyImmersiveMode(false) }.onFailure {
                Toast.makeText(InterfaceLanguage.context(this), R.string.service_save_failed, Toast.LENGTH_LONG).show()
            }
        }
    }

    private fun applyImmersiveMode(enabled: Boolean) {
        SettingsStore.setImmersiveSubtitles(this, enabled)
        applyAppearance()
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
        val collapse = panelButton(interfaceString(R.string.overlay_collapse)).apply {
            tag = "collapse-overlay"
            minWidth = dp(58)
            contentDescription = interfaceString(R.string.overlay_collapse_description)
            setOnClickListener { collapseOverlay() }
        }
        val font = panelButton(interfaceString(R.string.overlay_font_size, SettingsStore.fontSize(this))).apply {
            tag = "overlay-font"
            minWidth = dp(42)
            contentDescription = interfaceString(R.string.overlay_font_description)
            setOnClickListener { showFontSizeControl() }
        }
        val immersive = panelButton(interfaceString(R.string.overlay_enter_immersive)).apply {
            tag = "enter-immersive"
            minWidth = dp(58)
            contentDescription = interfaceString(R.string.overlay_enter_immersive)
            setOnClickListener { setImmersiveMode(true) }
        }
        val route = panelButton(
            if (sessionOriginalOnly) languageName(sessionSourceLanguage)
            else "${languageName(sessionSourceLanguage)} → ${languageName(sessionTargetLanguage)}",
        ).apply {
            tag = "overlay-route"
            contentDescription = interfaceString(R.string.overlay_language_description)
            maxLines = 1
            ellipsize = android.text.TextUtils.TruncateAt.END
            setOnClickListener {
                startActivity(Intent(this@MimiService, app.yuxino.mimi.android.MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        }
        relayoutExpandedHeader = {
            listOf(collapse, route, font, immersive).forEach {
                (it.parent as? android.view.ViewGroup)?.removeView(it)
                it.textSize = 13f
            }
            header.removeAllViews()
            header.orientation = LinearLayout.HORIZONTAL
            val actions = listOf(collapse, font, immersive)
            actions.forEach {
                it.measure(View.MeasureSpec.UNSPECIFIED, View.MeasureSpec.makeMeasureSpec(dp(38), View.MeasureSpec.EXACTLY))
            }
            val availableWidth = expandedPanelWidth() - panel.paddingLeft - panel.paddingRight
            val requiredWidth = actions.sumOf { it.measuredWidth } + dp(58 + 3 * 8)
            if (requiredWidth <= availableWidth) {
                header.addView(collapse, LinearLayout.LayoutParams(-2, dp(38)))
                header.addView(route, LinearLayout.LayoutParams(0, dp(38), 1f).apply { marginStart = dp(8) })
                header.addView(font, LinearLayout.LayoutParams(-2, dp(38)).apply { marginStart = dp(8) })
                header.addView(immersive, LinearLayout.LayoutParams(-2, dp(38)).apply { marginStart = dp(8) })
            } else {
                // Large system fonts need a second row, not smaller or clipped labels.
                header.orientation = LinearLayout.VERTICAL
                val actionsRow = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL }
                actionsRow.addView(collapse, LinearLayout.LayoutParams(-2, dp(38)))
                actionsRow.addView(View(this), LinearLayout.LayoutParams(0, 1, 1f))
                val separateImmersive = collapse.measuredWidth + immersive.measuredWidth + dp(8) > availableWidth
                if (!separateImmersive) {
                    actionsRow.addView(immersive, LinearLayout.LayoutParams(-2, dp(38)).apply { marginStart = dp(8) })
                }
                val preferencesRow = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL }
                preferencesRow.addView(route, LinearLayout.LayoutParams(0, dp(38), 1f))
                preferencesRow.addView(font, LinearLayout.LayoutParams(-2, dp(38)).apply { marginStart = dp(8) })
                header.addView(actionsRow, LinearLayout.LayoutParams(-1, -2))
                header.addView(preferencesRow, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
                if (separateImmersive) {
                    val modeRow = LinearLayout(this).apply { gravity = Gravity.END }
                    modeRow.addView(immersive, LinearLayout.LayoutParams(-2, dp(38)))
                    header.addView(modeRow, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(8) })
                }
            }
        }
        relayoutExpandedHeader?.invoke()
        panel.addView(header)
        if (activeModelNames.isNotEmpty()) panel.addView(TextView(this).apply {
            tag = "overlay-model"
            text = app.yuxino.mimi.android.modelNamesLabel(activeModelNames)
            textSize = 14f
            setTextColor(0xFFDDDDDD.toInt())
        }, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(10) })

        expandedStatusView = TextView(this).apply {
            setTextColor(0xFFB9B9B9.toInt())
            textSize = 14f
            visibility = View.GONE
        }
        panel.addView(expandedStatusView, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(10) })

        val scroll = ScrollView(this).apply {
            tag = "subtitle-transcript-scroll"
            isFillViewport = true
        }
        transcriptScrollView = scroll
        scroll.viewTreeObserver.addOnPreDrawListener {
            if (expanded && scrollToCurrentOnLayout) {
                scrollToCurrentOnLayout = false
                currentExpandedCaption()?.let { scroll.scrollTo(0, it.top) }
            }
            true
        }
        val transcript = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        historyView = TextView(this).apply {
            setTextColor(0xFFB9B9B9.toInt())
            textSize = (SettingsStore.fontSize(this@MimiService) - 1).coerceAtLeast(12).toFloat()
            setLineSpacing(dp(5).toFloat(), 1f)
        }
        transcript.addView(historyView)
        transcript.addView(View(this).apply { setBackgroundColor(0xFF555555.toInt()) },
            LinearLayout.LayoutParams(-1, dp(1)))
        expandedSourceView = TextView(this).apply {
            tag = "expanded-source"
            setTextColor(0xFFD5D5D5.toInt())
            textSize = SettingsStore.fontSize(this@MimiService).toFloat()
        }
        expandedTranslationView = TextView(this).apply {
            tag = "expanded-translation"
            setTextColor(SettingsStore.translationColor(this@MimiService))
            textSize = (SettingsStore.fontSize(this@MimiService) + 3).toFloat()
            setTypeface(typeface, Typeface.BOLD)
        }
        // The shared core bounds sentence content. The reading panel keeps its
        // fixed viewport while allowing the complete current pair to scroll.
        // Compact and immersive captions retain their two/three-line limits.
        transcript.addView(expandedSourceView, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(14) })
        transcript.addView(expandedTranslationView, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(6) })
        scroll.addView(transcript)
        panel.addView(scroll, LinearLayout.LayoutParams(-1, 0, 1f).apply { topMargin = dp(18) })
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
        setSingleLine(true)
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

    private fun languageName(code: String): String = app.yuxino.mimi.android.languageDisplayName(InterfaceLanguage.context(this), code)

    private fun showFontSizeControl() {
        if (fontDialog != null) return
        val themed = ContextThemeWrapper(InterfaceLanguage.context(this), R.style.Theme_Mimi)
        val content = LinearLayout(themed).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24), dp(12), dp(24), 0)
        }
        val value = TextView(themed).apply {
            tag = "overlay-font-value"
            setTextColor(ContextCompat.getColor(themed, R.color.mimi_text))
            textSize = 18f
            text = themed.getString(R.string.settings_font_value, SettingsStore.fontSize(this@MimiService))
        }
        val slider = SeekBar(themed).apply {
            tag = "overlay-font-slider"
            contentDescription = themed.getString(R.string.settings_font)
            min = 12; max = 24; progress = SettingsStore.fontSize(this@MimiService)
            progressTintList = ContextCompat.getColorStateList(themed, R.color.mimi_text)
            thumbTintList = progressTintList
            setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(seekBar: SeekBar?, progress: Int, fromUser: Boolean) {
                    if (!fromUser) return
                    try {
                        SettingsStore.setFontSize(this@MimiService, progress)
                        value.text = themed.getString(R.string.settings_font_value, progress)
                        applyAppearance()
                    } catch (_: RuntimeException) {
                        seekBar?.progress = SettingsStore.fontSize(this@MimiService)
                        Toast.makeText(themed, R.string.service_save_failed, Toast.LENGTH_LONG).show()
                    }
                }
                override fun onStartTrackingTouch(seekBar: SeekBar?) = Unit
                override fun onStopTrackingTouch(seekBar: SeekBar?) = Unit
            })
        }
        content.addView(value, LinearLayout.LayoutParams(-1, -2))
        content.addView(slider, LinearLayout.LayoutParams(-1, dp(48)))
        val surface = GradientDrawable().apply { cornerRadius = dp(28).toFloat() }
        val dialog = MaterialAlertDialogBuilder(themed).setBackground(surface).setTitle(R.string.settings_font)
            .setView(content).setPositiveButton(R.string.guide_finish, null).create()
        dialog.setOnDismissListener { if (fontDialog === dialog) fontDialog = null }
        fontDialog = dialog
        try {
            checkNotNull(dialog.window).setType(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY)
            dialog.show()
            surface.setColor(ContextCompat.getColor(dialog.context, R.color.mimi_surface))
        } catch (_: RuntimeException) {
            fontDialog = null
            dialog.dismiss()
            Toast.makeText(themed, R.string.overlay_font_failed, Toast.LENGTH_LONG).show()
        }
    }

    private fun applyAppearance() {
        val root = overlayView ?: return
        if (overlayPlacement.setImmersive(SettingsStore.immersiveSubtitles(this))) {
            draggingCompact = false
            val params = checkNotNull(overlayParams)
            params.flags = if (immersiveSession) params.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
                else params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
            params.alpha = if (immersiveSession) 0.8f else 1f
            // Update child presentation before WindowManager measures the new
            // compact frame or positions its companion restore control.
            compactView?.visibility = if (expanded) View.GONE else View.VISIBLE
            expandedView?.visibility = if (expanded) View.VISIBLE else View.GONE
            if (immersiveSession) {
                showImmersiveExitControl(checkNotNull(windowManager))
            } else {
                immersiveExitView?.let { windowManager?.removeView(it) }
                immersiveExitView = null
                immersiveExitParams = null
            }
            // Keep the same caption views, reading position and compact anchor.
            windowManager?.updateViewLayout(root, params)
        }
        updateOverlayFontSize()
        fontDialog?.window?.decorView?.findViewWithTag<TextView>("overlay-font-value")?.text =
            interfaceString(R.string.settings_font_value, SettingsStore.fontSize(this))
        fontDialog?.window?.decorView?.findViewWithTag<SeekBar>("overlay-font-slider")?.progress = SettingsStore.fontSize(this)
        compactView?.apply {
            alpha = if (immersiveSession) 1f else SettingsStore.overlayOpacity(this@MimiService) / 100f
            (background as? GradientDrawable)?.setColor(
                (if (immersiveSession) 0 else ((SettingsStore.overlayBgAlpha(this@MimiService) / 100.0) * 255).toInt()) shl 24 or 0x101010)
        }
        translationView?.setTextColor(SettingsStore.translationColor(this))
        expandedTranslationView?.setTextColor(SettingsStore.translationColor(this))
        overlayPlacement.setPreferredOffsetDp(SettingsStore.overlayYOffset(this))
        compactView?.setPadding(dp(if (immersiveSession) 3 else 14), dp(9), dp(if (immersiveSession) 3 else 14), dp(10))
        listOf(sourceView, translationView).forEach {
            it?.setShadowLayer(if (immersiveSession) dp(4).toFloat() else 0f, 0f, dp(1).toFloat(), Color.BLACK)
        }
        updateOverlayGeometry()
        relayoutExpandedHeader?.invoke()
        renderBus()
    }

    private fun updateOverlayFontSize() {
        val size = SettingsStore.fontSize(this).toFloat()
        sourceView?.textSize = size
        translationView?.textSize = size + 3
        expandedSourceView?.textSize = size
        expandedTranslationView?.textSize = size + 3
        historyView?.textSize = (size - 1).coerceAtLeast(12f)
        overlayView?.findViewWithTag<TextView>("overlay-font")?.text =
            interfaceString(R.string.overlay_font_size, size.toInt())
    }

    private fun showExpandedOverlay() {
        val root = overlayView ?: return
        val params = overlayParams ?: return
        val panel = expandedView ?: return
        if (!overlayPlacement.expand()) return
        compactView?.visibility = View.GONE
        panel.visibility = View.VISIBLE
        scrollToCurrentOnLayout = true
        params.width = expandedPanelWidth()
        params.height = expandedPanelHeight(0)
        params.gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
        params.y = 0
        windowManager?.updateViewLayout(root, params)
        renderBus()
    }

    /** WindowManager applies these insets once; y is relative to its safe frame. */
    private fun fitOverlayInsets(params: WindowManager.LayoutParams) {
        if (Build.VERSION.SDK_INT >= 30) {
            params.setFitInsetsTypes(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            params.setFitInsetsIgnoringVisibility(true)
        }
    }

    private fun overlayAvailableSize(): Pair<Int, Int> {
        val wm = windowManager
        if (Build.VERSION.SDK_INT >= 30 && wm != null) {
            val metrics = wm.currentWindowMetrics
            val insets = metrics.windowInsets.getInsetsIgnoringVisibility(
                WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout(),
            )
            return OverlayPlacement.availableExtent(metrics.bounds.width(), insets.left, insets.right) to
                OverlayPlacement.availableExtent(metrics.bounds.height(), insets.top, insets.bottom)
        }
        // API 29 keeps the platform's fitted overlay frame. Do not subtract
        // system bars again from the already-fitted visible display frame.
        val frame = Rect()
        overlayView?.getWindowVisibleDisplayFrame(frame)
        return if (!frame.isEmpty) frame.width() to frame.height()
            else resources.displayMetrics.widthPixels to resources.displayMetrics.heightPixels
    }

    private fun updateOverlayGeometry(historyCount: Int = SubtitleBus.historySnapshot().size) {
        val root = overlayView ?: return
        val params = overlayParams ?: return
        val (width, height) = overlayAvailableSize()
        val padding = dp(if (immersiveSession) 3 else 14) * 2
        val textWidth = minOf((width * 0.88f).toInt(), (width - padding).coerceAtLeast(1))
        listOf(statusView, sourceView, translationView).forEach {
            if (it != null && it.maxWidth != textWidth) it.maxWidth = textWidth
        }
        val desiredWidth = if (expanded) expandedPanelWidth() else WindowManager.LayoutParams.WRAP_CONTENT
        val desiredHeight = if (expanded) expandedPanelHeight(historyCount) else WindowManager.LayoutParams.WRAP_CONTENT
        val captionHeight = compactView?.height ?: 0
        val desiredY = when {
            expanded -> 0
            draggingCompact -> OverlayPlacement.clampY(params.y, height, captionHeight)
            else -> overlayPlacement.compactY(resources.displayMetrics.density, height, captionHeight)
        }
        if (params.width != desiredWidth || params.height != desiredHeight || params.y != desiredY) {
            params.width = desiredWidth
            params.height = desiredHeight
            params.y = desiredY
            windowManager?.updateViewLayout(root, params)
        }
        immersiveExitParams?.let { exitParams ->
            val y = exitControlY()
            val x = maxOf(dp(4), (width - root.width) / 2)
            if (exitParams.y != y || exitParams.x != x) {
                exitParams.y = y
                exitParams.x = x
                immersiveExitView?.let { windowManager?.updateViewLayout(it, exitParams) }
            }
        }
    }

    private fun expandedPanelWidth(): Int =
        (overlayAvailableSize().first * 0.92f).toInt().coerceAtMost(dp(560))

    private fun expandedPanelHeight(historyCount: Int): Int {
        val (availableWidth, availableHeight) = overlayAvailableSize()
        val landscape = availableWidth > availableHeight
        val maxFraction = if (landscape && historyCount == 0) 0.48f
            else if (landscape) 0.62f else 0.64f
        return dp(230 + historyCount * 76)
            .coerceAtMost((availableHeight * maxFraction).toInt())
    }

    private fun collapseOverlay() {
        val root = overlayView ?: return
        val params = overlayParams ?: return
        if (!overlayPlacement.collapse()) return
        expandedView?.visibility = View.GONE
        compactView?.visibility = View.VISIBLE
        params.width = WindowManager.LayoutParams.WRAP_CONTENT
        params.height = WindowManager.LayoutParams.WRAP_CONTENT
        params.gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
        params.y = overlayPlacement.compactY(resources.displayMetrics.density, overlayAvailableSize().second, compactView?.height ?: 0)
        windowManager?.updateViewLayout(root, params)
        renderBus()
    }

    private fun hideOverlay() {
        fontDialog?.dismiss()
        fontDialog = null
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
        overlayPlacement.reset(overlayPlacement.preferredOffsetDp, false)
        draggingCompact = false
        statusView = null
        historyView = null
        sourceView = null
        translationView = null
        expandedStatusView = null
        expandedSourceView = null
        expandedTranslationView = null
        transcriptScrollView = null
        relayoutExpandedHeader = null
        scrollToCurrentOnLayout = false
    }

    private fun currentExpandedCaption(): TextView? =
        expandedSourceView?.takeIf { it.visibility == View.VISIBLE && it.text.isNotEmpty() }
            ?: expandedTranslationView?.takeIf { it.visibility == View.VISIBLE && it.text.isNotEmpty() }

    private fun readingCurrentCaption(): Boolean {
        val scroll = transcriptScrollView ?: return true
        val current = currentExpandedCaption() ?: return true
        val maxScroll = ((scroll.getChildAt(0)?.height ?: 0) - scroll.height).coerceAtLeast(0)
        return scroll.scrollY >= minOf(current.top, maxScroll) - dp(8)
    }

    private fun renderBus() {
        val maxHistory = if (previewMode) (if (previewHistoryEnabled) 3 else 0)
            else SettingsStore.historyLines(this)
        val history = if (maxHistory > 0) SubtitleBus.historySnapshot().takeLast(maxHistory) else emptyList()
        val historyText = history.joinToString("\n\n") { pair ->
            if (pair.translation.isEmpty()) pair.source else "${pair.source}\n${pair.translation}"
        }
        // Keep a new completed sentence readable, but do not interrupt a reader
        // who scrolled up into history. History layout changes can move the same
        // current pair below the viewport too. Streaming drafts retain their position.
        val currentPairChanged = expandedSourceView?.text?.toString() != SubtitleBus.displaySource ||
            expandedTranslationView?.text?.toString() != SubtitleBus.displayTranslation
        val historyChanged = historyView?.text?.toString() != historyText
        if (expanded && (historyChanged || (currentPairChanged && SubtitleBus.displayPairFinal)) && readingCurrentCaption()) {
            scrollToCurrentOnLayout = true
        }
        val observation = captureObservation?.state
        val statusLine = if (observation == CaptureHealth.State.NO_PCM || observation == CaptureHealth.State.SILENT) {
            interfaceString(R.string.capture_no_sound_hint)
        } else app.yuxino.mimi.android.runtimeFeedbackResource(SubtitleBus.runtimeFeedback)
            ?.let(::interfaceString) ?: SubtitleBus.statusLine
        statusView?.apply {
            visibility = if (statusLine.isEmpty()) View.GONE else View.VISIBLE
            text = statusLine
        }
        expandedStatusView?.apply {
            visibility = if (statusLine.isEmpty()) View.GONE else View.VISIBLE
            text = statusLine
        }
        historyView?.apply {
            visibility = if (history.isEmpty()) View.GONE else View.VISIBLE
            maxLines = maxOf(maxHistory, 1) * 3
            text = historyText
        }
        // Translation sessions keep their established bilingual display. Original-only
        // sessions always show the recognized text regardless of its language.
        val liveVisible = !SubtitleBus.liveHidden
        val sourceIsEnglish =
            sessionSourceLanguage == "en" ||
                SubtitleBus.detectedSourceLanguage?.startsWith("en") == true
        sourceView?.apply {
            visibility = if (liveVisible && (sourceIsEnglish || sessionOriginalOnly)) View.VISIBLE else View.GONE
            text = SubtitleBus.displaySource
        }
        translationView?.apply {
            visibility = if (liveVisible && !sessionOriginalOnly) View.VISIBLE else View.GONE
            text = SubtitleBus.displayTranslation
        }
        expandedSourceView?.apply {
            visibility = if (liveVisible && (SubtitleBus.sourceDraft.isNotEmpty() || SubtitleBus.sourceFinal.isNotEmpty()))
                View.VISIBLE else View.GONE
            text = SubtitleBus.displaySource
        }
        expandedTranslationView?.apply {
            visibility = if (liveVisible && !sessionOriginalOnly) View.VISIBLE else View.GONE
            text = SubtitleBus.displayTranslation
        }
        updateOverlayGeometry(history.size)
        val hasCaption = listOf(sourceView, translationView).any {
            it?.visibility == View.VISIBLE && !it.text.isNullOrBlank()
        }
        overlayView?.visibility = if (expanded || hasCaption || statusLine.isNotBlank())
            View.VISIBLE else View.GONE
    }

    private fun interfaceString(@androidx.annotation.StringRes id: Int): String =
        InterfaceLanguage.context(this).getString(id)

    private fun interfaceString(@androidx.annotation.StringRes id: Int, value: Int): String =
        InterfaceLanguage.context(this).getString(id, value)

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density).toInt()

    private fun createChannel() {
        val channel = NotificationChannel(
            CHANNEL_ID,
            interfaceString(R.string.notification_channel),
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
        @Volatile var activeModelNames: List<String> = emptyList()
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
