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
import android.widget.ImageView
import app.yuxino.mimi.android.AppearanceControls
import app.yuxino.mimi.android.CaptionDisplayMode
import app.yuxino.mimi.android.serviceProviderIcon
import app.yuxino.mimi.android.translationProviderIcon
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
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
    private var draggingOverlay = false
    private var resizeWidthPercent: Int? = null
    private var resizeHeightPercent: Int? = null
    private val ownUiUpdate = Runnable { renderBus() }
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
    private var interfaceLocaleTags = ""
    private val interfaceLanguageListener: () -> Unit = { mainHandler.post { refreshInterfaceLanguage() } }
    private var stopObservingAppearance: (() -> Unit)? = null
    private var appearanceDialog: AlertDialog? = null
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
        ownUiChanged = {
            mainHandler.removeCallbacks(ownUiUpdate)
            if (ownUiVisible) {
                if (!previewOverOwnUi) {
                    appearanceDialog?.dismiss()
                    immersiveHelp.dismiss()
                }
                renderBus()
            } else mainHandler.postDelayed(ownUiUpdate, 120)
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
            renderBus()
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
            previewOverOwnUi = intent.getBooleanExtra("preview_over_own_ui", false)
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
            renderBus()
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
        appearanceDialog?.dismiss()
        appearanceDialog = null
        ++generation
        finishingSession = false
        isStopping = false
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
        isStopping = true
        stateListeners.forEach { it() }
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
        ownUiChanged = null
        mainHandler.removeCallbacks(ownUiUpdate)
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
        appearanceDialog?.dismiss()
        immersiveHelp = ImmersiveModeHelp(ContextThemeWrapper(InterfaceLanguage.context(this), R.style.Theme_Mimi), overlayWindow = true)
        fun label(tag: String, text: Int?, description: Int) {
            val control = overlayView?.findViewWithTag<View>(tag)
                ?: immersiveExitView?.takeIf { it.tag == tag } ?: return
            if (text != null && control is TextView) control.text = interfaceString(text)
            control.contentDescription = interfaceString(description)
            control.tooltipText = control.contentDescription
        }
        label("enter-immersive", R.string.overlay_enter_immersive, R.string.overlay_enter_immersive)
        label("overlay-font", null, R.string.caption_style_controls)
        updateOverlayFontSize()
        label("exit-immersive", null, R.string.overlay_exit_immersive)
        label("overlay-drag", null, R.string.overlay_drag)
        label("overlay-resize", null, R.string.overlay_resize)
        label("overlay-route", null, R.string.overlay_language_description)
        overlayView?.findViewWithTag<TextView>("overlay-route")?.text =
            if (sessionOriginalOnly) languageName(sessionSourceLanguage)
            else "${languageName(sessionSourceLanguage)} → ${languageName(sessionTargetLanguage)}"
        overlayView?.findViewWithTag<View>("overlay-providers")?.let { providers ->
            providers.contentDescription = overlayProviderLabel()
            overlayView?.findViewWithTag<TextView>("overlay-model")?.text = providers.contentDescription
        }
        expandedView?.requestLayout()
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
        overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(this))
        val compact = app.yuxino.mimi.android.CompactCaptionView(this)
        val status = compact.status; val source = compact.source; val translation = compact.translation
        compact.applyStyle(app.yuxino.mimi.android.CaptionFrameStyle.saved(this, sessionOriginalOnly), immersiveSession)
        val panel = buildExpandedPanel()
        val root = FrameLayout(this).apply {
            tag = "mimi-overlay"
            addView(compact, FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.WRAP_CONTENT, Gravity.CENTER,
            ))
            compact.visibility = if (expanded) View.GONE else View.VISIBLE
            panel.visibility = if (expanded) View.VISIBLE else View.GONE
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

        fun attachDrag(control: View) {
            var initialY = 0
            var initialX = 0
            var touchY = 0f
            var touchX = 0f
            var dragged = false
            control.setOnTouchListener { _, event ->
                if (immersiveSession) return@setOnTouchListener false
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        initialY = params.y; initialX = params.x
                        touchY = event.rawY; touchX = event.rawX
                        dragged = false; draggingOverlay = false
                        true
                    }
                    MotionEvent.ACTION_MOVE -> {
                        if (kotlin.math.abs(event.rawY - touchY) > dp(8) || kotlin.math.abs(event.rawX - touchX) > dp(8)) dragged = true
                        if (dragged) {
                            draggingOverlay = true
                            val (width, height) = overlayAvailableSize()
                            params.y = OverlayPlacement.clampY((initialY - (event.rawY - touchY)).toInt(), height, root.height)
                            params.x = OverlayPlacement.clampX((initialX + event.rawX - touchX).toInt(), width, root.width)
                            wm.updateViewLayout(root, params)
                        }
                        true
                    }
                    MotionEvent.ACTION_UP -> {
                        draggingOverlay = false
                        if (dragged) {
                            try {
                                val density = resources.displayMetrics.density
                                SettingsStore.setOverlayPosition(this, true,
                                    kotlin.math.round(params.x / density).toInt(), kotlin.math.round(params.y / density).toInt())
                                overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(this))
                            } catch (_: RuntimeException) { Toast.makeText(InterfaceLanguage.context(this), R.string.service_save_failed, Toast.LENGTH_LONG).show() }
                            updateOverlayGeometry()
                        } else control.performClick()
                        true
                    }
                    MotionEvent.ACTION_CANCEL -> { draggingOverlay = false; updateOverlayGeometry(); true }
                    else -> false
                }
            }
        }
        attachDrag(panel.findViewWithTag("overlay-drag"))
        var resizeStartWidth = 0
        var resizeStartHeight = 0
        var resizeStartX = 0f
        var resizeStartY = 0f
        var resizeStartOffsetX = 0
        var resizeStartOffsetY = 0
        panel.findViewWithTag<View>("overlay-resize").setOnTouchListener { control, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    resizeStartWidth = root.width; resizeStartHeight = root.height
                    resizeStartX = event.rawX; resizeStartY = event.rawY
                    resizeStartOffsetX = params.x; resizeStartOffsetY = params.y
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    val deltaX = event.rawX - resizeStartX
                    val deltaY = event.rawY - resizeStartY
                    if (kotlin.math.abs(deltaX) > dp(8) || kotlin.math.abs(deltaY) > dp(8)) {
                        val (width, height) = overlayAvailableSize()
                        draggingOverlay = true
                        resizeWidthPercent = ((resizeStartWidth + deltaX) * 100 / width).toInt().coerceIn(55, 100)
                        resizeHeightPercent = ((resizeStartHeight + deltaY) * 100 / height).toInt().coerceIn(20, 70)
                        // Keep the top-left corner fixed while the touched corner follows the finger.
                        params.x = resizeStartOffsetX + ((captionWindowWidth(true) - resizeStartWidth) / 2)
                        params.y = resizeStartOffsetY - (expandedPanelHeight() - resizeStartHeight)
                        updateOverlayGeometry(); expandedView?.requestLayout()
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    val width = resizeWidthPercent
                    val height = resizeHeightPercent
                    if (width == null || height == null) control.performClick()
                    else try {
                        val density = resources.displayMetrics.density
                        SettingsStore.setOverlayBounds(this, width, height,
                            kotlin.math.round(params.x / density).toInt(), kotlin.math.round(params.y / density).toInt())
                        overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(this))
                    } catch (_: RuntimeException) {
                        Toast.makeText(InterfaceLanguage.context(this), R.string.service_save_failed, Toast.LENGTH_LONG).show()
                    }
                    resizeWidthPercent = null; resizeHeightPercent = null; draggingOverlay = false
                    updateOverlayGeometry(); true
                }
                MotionEvent.ACTION_CANCEL -> {
                    resizeWidthPercent = null; resizeHeightPercent = null; draggingOverlay = false
                    updateOverlayGeometry(); true
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
        applyOverlaySurface(compact, panel)
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
            val caption = (if (expanded) listOf(expandedTranslationView, expandedSourceView)
                else listOf(translationView, sourceView)).firstOrNull { it?.isShown == true && !it.text.isNullOrBlank() }
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
        updateOverlayFontSize()
        renderBus()
    }

    private fun showImmersiveExitControl(wm: WindowManager) {
        if (immersiveExitView != null) return
        val exit = FrameLayout(this).apply {
            tag = "exit-immersive"
            contentDescription = interfaceString(R.string.overlay_exit_immersive)
            isClickable = true; isFocusable = true
            addView(ImageView(this@MimiService).apply {
                setImageResource(R.drawable.ic_overlay_close)
                setPadding(dp(5), dp(5), dp(5), dp(5))
                background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(0x99000000.toInt()) }
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }, FrameLayout.LayoutParams(dp(24), dp(24), Gravity.CENTER))
            minimumWidth = dp(48); minimumHeight = dp(48)
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
            immersiveHelp.requestEnable(onConfirmed = {
                runCatching { applyImmersiveMode(true) }.onFailure {
                    Toast.makeText(InterfaceLanguage.context(this), R.string.service_save_failed, Toast.LENGTH_LONG).show()
                }
            })
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
        val route = if (sessionOriginalOnly) languageName(sessionSourceLanguage)
            else "${languageName(sessionSourceLanguage)} → ${languageName(sessionTargetLanguage)}"
        val panel = app.yuxino.mimi.android.CaptionPanelView(this, route, overlayProviderLabel(), app.yuxino.mimi.android.captionProviderMarks(this),
            app.yuxino.mimi.android.CaptionPanelActions(
                home = { startActivity(Intent(this, app.yuxino.mimi.android.MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) },
                style = { showAppearanceControls() },
                immerse = { setImmersiveMode(true) },
                services = { startActivity(Intent(this, app.yuxino.mimi.android.MainActivity::class.java)
                    .putExtra("settings_section", "services")
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)) },
            ))
        panel.applyStyle(app.yuxino.mimi.android.CaptionFrameStyle.saved(this, sessionOriginalOnly))
        expandedStatusView = panel.status; expandedSourceView = panel.source
        expandedTranslationView = panel.translation; historyView = panel.history
        transcriptScrollView = panel.transcriptScroll
        panel.findViewWithTag<View>("overlay-actions").addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> updateOverlayGeometry() }
        panel.status.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> updateOverlayGeometry() }
        return panel
    }

    private var previewHistoryEnabled = false
    private var previewOverOwnUi = false

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

    private fun languageName(code: String): String = app.yuxino.mimi.android.languageDisplayName(InterfaceLanguage.context(this), code)

    private fun showAppearanceControls() {
        if (appearanceDialog != null) return
        val themed = ContextThemeWrapper(InterfaceLanguage.context(this), R.style.Theme_Mimi)
        val controls = AppearanceControls(themed, floating = true) { enabled -> setImmersiveMode(enabled) }
        controls.setPadding(dp(20), dp(8), dp(20), dp(16))
        val scroll = app.yuxino.mimi.android.ControlScrollView(themed).apply {
            tag = "overlay-appearance-scroll"
            addView(controls)
        }
        val maxBodyHeight = (overlayAvailableSize().second * .58f - dp(72)).toInt().coerceAtLeast(dp(96))
        val body = object : FrameLayout(themed) {
            override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
                val available = if (MeasureSpec.getMode(heightMeasureSpec) == MeasureSpec.UNSPECIFIED) maxBodyHeight
                    else minOf(maxBodyHeight, MeasureSpec.getSize(heightMeasureSpec))
                super.onMeasure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(available, MeasureSpec.AT_MOST))
            }
        }.apply { addView(scroll, FrameLayout.LayoutParams(-1, -1)) }
        val surface = GradientDrawable().apply { cornerRadius = dp(24).toFloat() }
        val dialog = MaterialAlertDialogBuilder(themed).setBackground(surface)
            // Material's centered-dialog insets waste the caption space above
            // this bottom editor, especially with a large system font.
            .setBackgroundInsetStart(0).setBackgroundInsetEnd(0)
            .setBackgroundInsetTop(0).setBackgroundInsetBottom(0)
            .setView(body).setPositiveButton(R.string.guide_finish, null).create()
        dialog.setCanceledOnTouchOutside(false)
        dialog.setOnDismissListener {
            if (appearanceDialog === dialog) {
                appearanceDialog = null
                updateEditorPresentation()
                updateOverlayGeometry()
            }
        }
        appearanceDialog = dialog
        try {
            val window = checkNotNull(dialog.window)
            window.setType(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY)
            dialog.show()
            window.clearFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND)
            window.addFlags(WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL)
            window.setGravity(Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL)
            window.attributes = window.attributes.apply { y = dp(12) }
            window.setLayout(minOf(overlayAvailableSize().first - dp(24), dp(480)), WindowManager.LayoutParams.WRAP_CONTENT)
            window.decorView.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
                if (appearanceDialog === dialog) mainHandler.post { updateOverlayGeometry() }
            }
            surface.setColor(ContextCompat.getColor(dialog.context, R.color.mimi_surface))
            updateEditorPresentation()
            updateOverlayGeometry()
        } catch (_: RuntimeException) {
            appearanceDialog = null
            dialog.dismiss()
            updateEditorPresentation()
            updateOverlayGeometry()
            Toast.makeText(themed, R.string.overlay_font_failed, Toast.LENGTH_LONG).show()
        }
    }

    private fun updateEditorPresentation() {
        // The live caption window is the preview. Hide its duplicate actions
        // while the bottom editor is open, then restore them in the same window.
        val visibility = if (appearanceDialog?.isShowing == true) View.GONE else View.VISIBLE
        expandedView?.findViewWithTag<View>("overlay-toolbar")?.visibility = visibility
        expandedView?.findViewWithTag<View>("overlay-actions")?.visibility = visibility
    }

    private fun editorOffset(): Int {
        val editor = appearanceDialog?.takeIf { it.isShowing }?.window?.decorView ?: return 0
        if (editor.height <= 0) return 0
        val origin = IntArray(2)
        editor.getLocationOnScreen(origin)
        val wm = windowManager ?: return 0
        val safeBottom = if (Build.VERSION.SDK_INT >= 30) {
            val metrics = wm.currentWindowMetrics
            metrics.bounds.bottom - metrics.windowInsets.getInsetsIgnoringVisibility(
                WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout()).bottom
        } else {
            val frame = Rect(); overlayView?.getWindowVisibleDisplayFrame(frame); frame.bottom
        }
        return (safeBottom - origin[1] + dp(12)).coerceAtLeast(0)
    }

    private fun applyAppearance() {
        val root = overlayView ?: return
        if (overlayPlacement.setImmersive(SettingsStore.immersiveSubtitles(this))) {
            draggingOverlay = false
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
        val compact = compactView
        val panel = expandedView
        if (compact != null && panel != null) applyOverlaySurface(compact, panel)
        translationView?.setTextColor(SettingsStore.translationColor(this))
        expandedTranslationView?.setTextColor(SettingsStore.translationColor(this))
        overlayPlacement.setPreferredOffsetDp(SettingsStore.overlayYOffset(this))
        overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(this))
        updateOverlayGeometry()
        expandedView?.requestLayout()
        renderBus()
    }

    /** Initial paint and live updates share the same renderer as the preview. */
    private fun applyOverlaySurface(compact: View, panel: View) {
        val style = app.yuxino.mimi.android.CaptionFrameStyle.saved(this, sessionOriginalOnly)
        app.yuxino.mimi.android.CaptionSurface.apply(compact, style, immersiveSession)
        app.yuxino.mimi.android.CaptionSurface.apply(panel, style, false)
    }

    private fun updateOverlayFontSize() {
        val style = app.yuxino.mimi.android.CaptionFrameStyle.saved(this, sessionOriginalOnly)
        (compactView as? app.yuxino.mimi.android.CompactCaptionView)?.applyStyle(style, immersiveSession)
        (expandedView as? app.yuxino.mimi.android.CaptionPanelView)?.applyStyle(style)
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

    private fun updateOverlayGeometry() {
        val root = overlayView ?: return
        val params = overlayParams ?: return
        val (width, height) = overlayAvailableSize()
        val padding = dp(if (immersiveSession) 3 else 14) * 2
        val windowWidth = captionWindowWidth(expanded)
        val textWidth = (windowWidth - padding).coerceAtLeast(1)
        listOf(statusView, sourceView, translationView).forEach {
            if (it != null && it.maxWidth != textWidth) it.maxWidth = textWidth
        }
        val desiredWidth = windowWidth
        val desiredHeight = if (expanded) expandedPanelHeight() else WindowManager.LayoutParams.WRAP_CONTENT
        val captionHeight = compactView?.height ?: 0
        val preferredY = when {
            draggingOverlay -> OverlayPlacement.clampY(params.y, height, if (expanded) desiredHeight else captionHeight)
            expanded -> overlayPlacement.expandedY(resources.displayMetrics.density, height, desiredHeight)
            else -> overlayPlacement.compactY(resources.displayMetrics.density, height, captionHeight)
        }
        val desiredY = OverlayPlacement.clampY(maxOf(preferredY, editorOffset()), height,
            if (expanded) desiredHeight else captionHeight)
        val desiredX = OverlayPlacement.clampX(if (draggingOverlay) params.x else
            (SettingsStore.overlayXOffset(this, expanded) * resources.displayMetrics.density).toInt(), width, desiredWidth)
        if (params.width != desiredWidth || params.height != desiredHeight || params.y != desiredY || params.x != desiredX) {
            params.x = desiredX
            params.width = desiredWidth
            params.height = desiredHeight
            params.y = desiredY
            windowManager?.updateViewLayout(root, params)
        }
        immersiveExitParams?.let { exitParams ->
            val y = exitControlY()
            val x = maxOf(dp(4), (width - root.width) / 2 - params.x)
            if (exitParams.y != y || exitParams.x != x) {
                exitParams.y = y
                exitParams.x = x
                immersiveExitView?.let { windowManager?.updateViewLayout(it, exitParams) }
            }
        }
    }

    private fun captionWindowWidth(expanded: Boolean): Int = OverlayPlacement.windowWidth(
        overlayAvailableSize().first, resizeWidthPercent ?: SettingsStore.overlayWidthPercent(this), dp(if (expanded) 176 else 160), dp(8))

    private fun expandedPanelHeight(): Int {
        val availableHeight = overlayAvailableSize().second
        val panel = expandedView
        val editing = appearanceDialog?.isShowing == true
        val chrome = if (editing) dp(20) else
            (panel?.paddingTop ?: dp(10)) + (panel?.paddingBottom ?: dp(10)) + dp(8) +
                (panel?.findViewWithTag<View>("overlay-toolbar")?.height?.takeIf { it > 0 } ?: dp(48)) +
                (panel?.findViewWithTag<View>("overlay-actions")?.height?.takeIf { it > 0 } ?: dp(48))
        val lineHeight = expandedTranslationView?.paint?.fontSpacing?.toInt() ?: dp(24)
        val limit = if (editing) (availableHeight - editorOffset()).coerceAtLeast(1) else availableHeight
        return app.yuxino.mimi.android.captionPanelHeight(availableHeight,
            resizeHeightPercent ?: SettingsStore.overlayHeightPercent(this), chrome, lineHeight, resources.displayMetrics.density, limit)
    }

    private fun hideOverlay() {
        appearanceDialog?.dismiss()
        appearanceDialog = null
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
        draggingOverlay = false
        statusView = null
        historyView = null
        sourceView = null
        translationView = null
        expandedStatusView = null
        expandedSourceView = null
        expandedTranslationView = null
        transcriptScrollView = null
    }

    private fun renderBus() {
        val maxHistory = if (previewMode) (if (previewHistoryEnabled) 3 else 0)
            else SettingsStore.historyLines(this)
        val mode = if (sessionOriginalOnly) CaptionDisplayMode.ORIGINAL else SettingsStore.displayMode(this)
        val content = app.yuxino.mimi.android.CaptionFrameContent.live(this, mode, maxHistory,
            captureObservation?.state, isRunning, isStopping)
        val statusLine = content.status
        (compactView as? app.yuxino.mimi.android.CompactCaptionView)?.let {
            content.applyRows(it.source, it.translation, it.status, mode)
            it.source.setTextColor(if (mode == CaptionDisplayMode.ORIGINAL) SettingsStore.translationColor(this) else 0xFFE7E7E7.toInt())
        }
        (expandedView as? app.yuxino.mimi.android.CaptionPanelView)?.let {
            it.showContent(content, mode, expanded)
            it.source.setTextColor(if (mode == CaptionDisplayMode.ORIGINAL) SettingsStore.translationColor(this) else 0xFFD5D5D5.toInt())
        }
        updateOverlayGeometry()
        val hasCaption = listOf(sourceView, translationView).any {
            it?.visibility == View.VISIBLE && !it.text.isNullOrBlank()
        }
        val coveredByOwnUi = ownUiVisible && !previewOverOwnUi
        overlayView?.visibility = if (!coveredByOwnUi && (expanded || hasCaption || statusLine.isNotBlank()))
            View.VISIBLE else View.GONE
        immersiveExitView?.visibility = if (coveredByOwnUi) View.GONE else View.VISIBLE
    }

    private fun interfaceString(@androidx.annotation.StringRes id: Int): String =
        InterfaceLanguage.context(this).getString(id)

    private fun overlayProviderLabel(): String = app.yuxino.mimi.android.overlayServiceLabel(InterfaceLanguage.context(this))

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
        private val ownUiOwners = CopyOnWriteArraySet<android.app.Activity>()
        private var ownUiChanged: (() -> Unit)? = null
        private val ownUiVisible: Boolean get() = ownUiOwners.isNotEmpty()
        internal fun setOwnUiVisible(owner: android.app.Activity, visible: Boolean) {
            val changed = if (visible) ownUiOwners.add(owner) else ownUiOwners.remove(owner)
            if (changed) ownUiChanged?.invoke()
        }
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
        @Volatile var isStopping: Boolean = false
            private set
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
