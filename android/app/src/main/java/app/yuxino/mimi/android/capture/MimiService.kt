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
import android.media.MediaRecorder
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
import android.widget.ScrollView
import android.widget.Toast
import android.widget.ImageView
import com.google.android.material.materialswitch.MaterialSwitch
import app.yuxino.mimi.android.CaptionAdjustmentSwitch
import app.yuxino.mimi.android.CaptionDisplayMode
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.SettingsStore
import app.yuxino.mimi.android.ImmersiveModeHelp
import app.yuxino.mimi.android.InterfaceLanguage
import app.yuxino.mimi.android.provider.EngineListener
import app.yuxino.mimi.android.provider.ProviderEngine
import app.yuxino.mimi.android.provider.NativeRuntimeConfiguration
import org.json.JSONObject
import app.yuxino.mimi.android.provider.SubtitleBus
import app.yuxino.mimi.android.resample.StreamResampler
import kotlin.concurrent.thread

/**
 * Foreground service that owns independent selected audio-capture lanes and
 * the overlay subtitle window. One service, one lifecycle, one notification.
 */
class MimiService : Service() {

    private val mainHandler = Handler(Looper.getMainLooper())

    private var mediaProjection: MediaProjection? = null
    private class CaptureLane(val source: CaptureSource, val engine: ProviderEngine) {
        var record: AudioRecord? = null
        var worker: Thread? = null
        val capturing = AtomicBoolean(false)
        var health: CaptureHealth? = null
    }
    private val lanes = linkedMapOf<CaptureSource, CaptureLane>()
    private val healthTick = object : Runnable {
        override fun run() {
            if (lanes.isEmpty()) return
            if (!android.provider.Settings.canDrawOverlays(this@MimiService) ||
                ContextCompat.checkSelfPermission(this@MimiService, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED ||
                getSystemService(android.app.KeyguardManager::class.java).isDeviceLocked ||
                !getSystemService(android.os.PowerManager::class.java).isInteractive) {
                lastCaptureError = "capture.permission_or_lock_changed"
                stopEverything()
                return
            }
            updateCaptureObservations()
            stateListeners.forEach { listener -> listener() }
            renderBus()
            mainHandler.postDelayed(this, 1_000)
        }
    }
    private fun updateCaptureObservations() {
        val now = SystemClock.elapsedRealtime()
        captureObservations = lanes.mapNotNull { (source, lane) -> lane.health?.snapshot(now)?.let { source to it } }.toMap()
        captureObservation = captureObservationForSources(captureObservations.values)
    }
    private var testCaptureMode = false
    private var historyHandle: Long? = null
    private var stopObservingStorage: (() -> Unit)? = null
    private val historyStates = mutableMapOf<CaptureSource, Pair<Boolean, Boolean>>()
    private var generation = 0
    private var finishingSession = false
    private var projectionCallback: MediaProjection.Callback? = null
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
    private var overlayGeometryGeneration = 0
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
    private var adjustmentSwitch: MaterialSwitch? = null
    private var adjustmentParams: WindowManager.LayoutParams? = null
    private var adjustable = true
    private var syncingAdjustment = false
    private val appearanceUpdate = Runnable { applyAppearance() }
    private val captionUpdatePending = AtomicBoolean(false)
    private val captionUpdate = Runnable {
        captionUpdatePending.set(false)
        renderBus()
    }

    private val busListener = object : SubtitleBus.Listener {
        override fun onSubtitleChanged() {
            // The shared reducer already preserves final events. Paint the
            // latest bounded snapshot once instead of queuing redundant frames.
            if (captionUpdatePending.compareAndSet(false, true)) mainHandler.post(captionUpdate)
        }
    }

    /** Recheck idle visibility after the sentence finishes. */
    private val autoHideRunnable = Runnable { SubtitleBus.hideLive() }

    /**
     * An unfinished recognition-only draft can expire during a pause. The bus
     * keeps visible translations and confirmed pairs readable across this check.
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
        stopObservingStorage = SettingsStore.observeSessionStorage(this) {
            mainHandler.post {
                if (!SettingsStore.retainSessionHistory(this)) disableSessionHistory()
            }
        }
        ownUiChanged = {
            mainHandler.removeCallbacks(ownUiUpdate)
            if (ownUiVisible) {
                if (!previewOverOwnUi) {
                    immersiveHelp.dismiss()
                }
                renderBus()
            } else mainHandler.postDelayed(ownUiUpdate, 120)
        }
        createChannel()
        SubtitleBus.addListener(busListener)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val startRequest = intent?.getLongExtra(EXTRA_START_REQUEST, 0L) ?: 0L
        try {
            // Expired delivery must not begin unexpected capture after controls unlock.
            if (startRequest != 0L && !startPending(startRequest)) {
                if (!isStarting && !isRunning && lanes.isEmpty()) stopSelf(startId)
                return START_NOT_STICKY
            }
            if (startRequest != 0L && !startOwnerVisible(startRequest)) {
                lastCaptureError = "capture.start_failed"
                stopSelf(startId)
                return START_NOT_STICKY
            }
            return handleStartCommand(intent)
        } finally {
            if (startRequest != 0L) {
                if (startPending(startRequest) && !isRunning && lastCaptureError == null) lastCaptureError = "capture.start_failed"
                completeStartRequest(startRequest)
            }
        }
    }

    private fun handleStartCommand(intent: Intent?): Int {
        if (intent?.action == ACTION_STOP) {
            startOwner?.first?.let(::completeStartRequest)
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
        val testEngines = if (intent?.action == ACTION_CAPTURE_TEST &&
            applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) captureEnginesForTests else null
        if (intent?.action == ACTION_CAPTURE_TEST && testEngines == null) {
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
        // Actual capture is always delivered from a currently visible Activity.
        if ((intent?.getLongExtra(EXTRA_START_REQUEST, 0L) ?: 0L) == 0L) {
            stopSelf()
            return START_NOT_STICKY
        }
        val input = CaptureInput.request(intent?.getStringExtra(EXTRA_CAPTURE_INPUT))
        val resultCode = intent?.getIntExtra(EXTRA_RESULT_CODE, Int.MIN_VALUE) ?: Int.MIN_VALUE
        val resultData = intent?.compatGetParcelableExtra(EXTRA_RESULT_DATA)
        if (input == null || input.needsProjection && (resultCode != android.app.Activity.RESULT_OK || resultData == null) ||
            testEngines != null && testEngines.keys != input.sources.toSet()) {
            stopSelf()
            return START_NOT_STICKY
        }
        try {
            lastCaptureError = null
            captureInput = input
            // Start from the visible activity after all required user grants.
            // Projection creation requires its foreground type on Android 14+.
            startAsForeground(input)
            val projection = if (input.needsProjection) {
                val manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
                checkNotNull(manager.getMediaProjection(resultCode, checkNotNull(resultData)))
                    .also { mediaProjection = it }
            } else null
            startCapture(input, projection, testEngines)
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

    private fun startAsForeground(input: CaptureInput) {
        val type = (if (input.needsProjection) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION else 0) or
            (if (input.usesMicrophone && Build.VERSION.SDK_INT >= 30) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0)
        startForeground(NOTIFICATION_ID, notification(), type)
        captureForegroundTypesForTests = foregroundServiceType
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
            .setContentText(interfaceString(if (captureInput == CaptureInput.MICROPHONE) R.string.notification_microphone_text else if (captureInput == CaptureInput.BOTH) R.string.capture_both_source_hint else R.string.notification_text))
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

    private fun startCapture(input: CaptureInput, projection: MediaProjection?, testEngines: Map<CaptureSource, ProviderEngine>? = null) {
        check(ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            "audio_permission_required"
        }
        check(android.provider.Settings.canDrawOverlays(this) &&
            getSystemService(android.os.PowerManager::class.java).isInteractive &&
            !getSystemService(android.app.KeyguardManager::class.java).isDeviceLocked) { "capture_permission_or_lock_changed" }
        val sessionGeneration = ++generation
        testCaptureMode = testEngines != null
        sessionOriginalOnly = false
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(SettingsStore.historyLines(this))
        synchronized(firstRunEvidence) { firstRunEvidence.reset() }
        if (projection != null) {
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
        }
        captureProjectionForTests = if (testEngines != null) projection else null
        val configuration = if (testEngines == null) SettingsStore.runtimeConfiguration(this) else null
        sessionSourceLanguage = configuration?.getString("sourceLanguage") ?: "auto"
        sessionTargetLanguage = configuration?.getString("targetLanguage") ?: "original"
        sessionOriginalOnly = sessionTargetLanguage == "original"
        activeModelNames = configuration?.let { app.yuxino.mimi.android.runtimeModelNames(it) } ?: emptyList()
        historyStatusGeneration.incrementAndGet()
        historySaveError = false
        historyLimited = false
        historyStates.clear()
        if (testEngines == null && SettingsStore.retainSessionHistory(this)) {
            historyHandle = runCatching {
                NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "history_begin")
                    .put("directory", java.io.File(filesDir, "session-history").absolutePath)).getLong("historyHandle")
            }.getOrElse { historySaveError = true; null }
        }

        for (source in input.sources) {
            fun dispatch(action: () -> Unit) {
                mainHandler.post { if (generation == sessionGeneration) action() }
            }
            val sessionEngine = testEngines?.get(source) ?: run {
                val listener = object : EngineListener {
                    override fun onRuntimeSnapshot(state: org.json.JSONObject, originalOnly: Boolean) = dispatch {
                        val previousStorage = historySaveError to historyLimited
                        historyStates[source] = state.optBoolean("historySaveError") to state.optBoolean("historyLimited")
                        historySaveError = historySaveError || historyStates.values.any { it.first }
                        historyLimited = historyLimited || historyStates.values.any { it.second }
                        if (previousStorage != (historySaveError to historyLimited)) stateListeners.forEach { it() }
                        SubtitleBus.onRuntimeSnapshot(state, originalOnly, source.id)
                        if (SubtitleBus.displayPairFinal) scheduleAutoHide() else cancelAutoHide()
                    }
                    override fun onError(code: String) = dispatch {
                        Toast.makeText(InterfaceLanguage.context(this@MimiService), providerErrorMessageResource(code), Toast.LENGTH_LONG).show()
                        stopEverything()
                    }
                    override fun onClosed() = dispatch { if (!finishingSession) stopEverything() }
                }
                app.yuxino.mimi.android.provider.SharedRuntimeEngine(checkNotNull(configuration), listener, source.id, historyHandle)
            }
            val lane = CaptureLane(source, sessionEngine)
            lanes[source] = lane
            if (testEngines == null) sessionEngine.start()
            val channels = if (source == CaptureSource.SYSTEM) AudioFormat.CHANNEL_IN_STEREO else AudioFormat.CHANNEL_IN_MONO
            val encoding = if (source == CaptureSource.SYSTEM) AudioFormat.ENCODING_PCM_FLOAT else AudioFormat.ENCODING_PCM_16BIT
            val format = AudioFormat.Builder().setEncoding(encoding).setSampleRate(CAPTURE_RATE_HZ).setChannelMask(channels).build()
            val minBuffer = AudioRecord.getMinBufferSize(CAPTURE_RATE_HZ, channels, encoding)
            check(minBuffer > 0) { "unsupported_capture_format" }
            val builder = AudioRecord.Builder().setAudioFormat(format).setBufferSizeInBytes(minBuffer * 4)
            if (source == CaptureSource.SYSTEM) {
                builder.setAudioPlaybackCaptureConfig(AudioPlaybackCaptureConfiguration.Builder(checkNotNull(projection))
                    .addMatchingUsage(AudioAttributes.USAGE_MEDIA).addMatchingUsage(AudioAttributes.USAGE_GAME)
                    .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN).build())
            } else builder.setAudioSource(MediaRecorder.AudioSource.MIC)
            lane.record = builder.build()
            check(lane.record?.state == AudioRecord.STATE_INITIALIZED) { "capture_uninitialized" }
        }
        // Prepare every selected lane before admitting any audio. A failed lane
        // tears the whole session down; it cannot silently become single-source.
        for (lane in lanes.values) startCaptureLane(lane, sessionGeneration)
        updateCaptureObservations()
        showOverlay()
        mainHandler.post(healthTick)
    }

    private fun startCaptureLane(lane: CaptureLane, sessionGeneration: Int) {
        val record = checkNotNull(lane.record)
        val resampler = StreamResampler(CAPTURE_RATE_HZ, lane.engine.sampleRateHz, lane.source.channels)
        val floatBuffer = if (lane.source == CaptureSource.SYSTEM) FloatArray(CAPTURE_RATE_HZ / 10 * lane.source.channels) else null
        val shortBuffer = if (lane.source == CaptureSource.MICROPHONE) ShortArray(CAPTURE_RATE_HZ / 10) else null
        val sessionHealth = CaptureHealth(SystemClock.elapsedRealtime())
        lane.health = sessionHealth
        record.startRecording()
        check(record.recordingState == AudioRecord.RECORDSTATE_RECORDING) { "capture_not_started" }
        lane.capturing.set(true)
        lane.worker = thread(name = "mimi-capture-${lane.source.id}") {
            try {
                android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_URGENT_AUDIO)
                while (lane.capturing.get()) {
                    val read = if (floatBuffer != null) record.read(floatBuffer, 0, floatBuffer.size, AudioRecord.READ_BLOCKING)
                        else record.read(checkNotNull(shortBuffer), 0, shortBuffer.size, AudioRecord.READ_BLOCKING)
                    if (!lane.capturing.get()) break
                    check(read >= 0) { "capture_read_failed" }
                    if (read == 0) continue
                    val samples = floatBuffer?.copyOf(read) ?: FloatArray(read) { checkNotNull(shortBuffer)[it] / 32768f }
                    val pcm = resampler.push(samples)
                    sessionHealth.observe(pcm, SystemClock.elapsedRealtime())
                    if (lane.capturing.get() && pcm.isNotEmpty()) {
                        lane.engine.sendAudio(pcm)
                        synchronized(firstRunEvidence) { if (lane.capturing.get()) firstRunEvidence.submitted(pcm) }
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
            } finally { record.release() }
        }
    }

    private fun releaseSession() {
        immersiveHelp.dismiss()
        ++generation
        finishingSession = false
        isStopping = false
        captureObservation = null
        captureObservations = emptyMap()
        mainHandler.removeCallbacksAndMessages(null)
        captionUpdatePending.set(false)
        val retired = lanes.values.toList()
        lanes.clear()
        retired.forEach { lane ->
            lane.capturing.set(false)
            try { lane.record?.stop() } catch (_: Exception) { }
        }
        retired.forEach { lane ->
            try { if (lane.worker != null) lane.worker?.join(600) else lane.record?.release() } catch (_: Exception) { }
            try { lane.engine.stop() } catch (_: Exception) { Log.w(TAG, "capture_engine_stop_failed") }
        }
        historyHandle?.let { handle ->
            val operation = if (SettingsStore.retainSessionHistory(this)) "history_finish" else "history_disable"
            runCatching { NativeRuntimeConfiguration.exchange(JSONObject().put("operation", operation).put("historyHandle", handle)) }
                .onSuccess { response ->
                    historySaveError = historySaveError || response.optJSONObject("state")?.optBoolean("saveError") == true
                    if (response.optBoolean("pending")) observeHistoryFinalization(handle)
                }
                .onFailure { historySaveError = true }
        }
        historyHandle = null
        historyStates.clear()
        synchronized(firstRunEvidence) { firstRunEvidence.reset() }
        val projection = mediaProjection
        mediaProjection = null
        captureProjectionForTests = null
        captureForegroundTypesForTests = 0
        projectionCallback?.let { projection?.unregisterCallback(it) }
        projectionCallback = null
        try { projection?.stop() } catch (_: Exception) { }
        SubtitleBus.finishSnapshot()
        hideOverlay()
        previewMode = false
        testCaptureMode = false
        activeModelNames = emptyList()
        setRunning(false)
    }

    /** User stop drains accepted work; revocation/errors/destruction still abort. */
    private fun finishSession() {
        if (finishingSession) return
        if (lanes.isEmpty() || previewMode) { stopEverything(); return }
        finishingSession = true
        isStopping = true
        stateListeners.forEach { it() }
        val owner = generation
        lanes.values.forEach { lane ->
            lane.capturing.set(false)
            try { lane.record?.stop() } catch (_: Exception) { }
        }
        mainHandler.removeCallbacks(healthTick)
        cancelAutoHide()
        val policy = app.yuxino.mimi.android.provider.NativeRuntimeConfiguration.policy
        // Cover audio drain, bounded provider finish, disconnect and publication;
        // the provider-only timeout can expire before Rust publishes its tail.
        mainHandler.postDelayed({ if (generation == owner && finishingSession) stopEverything() }, policy.getLong("sessionFinishTimeoutMs"))
        var remaining = lanes.size
        lanes.values.forEach { lane ->
            val acknowledged = AtomicBoolean(false)
            lane.engine.finish {
                mainHandler.post {
                    if (generation != owner || !finishingSession || !acknowledged.compareAndSet(false, true)) return@post
                    remaining--
                    if (remaining == 0) {
                        // Snapshot publication is queued before each finish callback.
                        renderBus()
                        mainHandler.post { if (generation == owner && finishingSession) stopEverything() }
                    }
                }
            }
        }
    }

    private fun disableSessionHistory() {
        val handle = historyHandle ?: return
        runCatching { NativeRuntimeConfiguration.exchange(JSONObject().put("operation", "history_disable").put("historyHandle", handle)) }
            .onSuccess { response ->
                historySaveError = response.optJSONObject("state")?.optBoolean("saveError") == true
                historyLimited = false
                historyStates.clear()
            }.onFailure { historySaveError = true }
        stateListeners.forEach { it() }
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
        stopObservingStorage?.invoke()
        stopObservingStorage = null
        SubtitleBus.removeListener(busListener)
        releaseSession()
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        // A gesture measured in the previous screen's axes cannot be committed
        // after rotation. Reapply saved bounds and clamp only the visible frame.
        draggingOverlay = false
        resizeWidthPercent = null
        resizeHeightPercent = null
        overlayGeometryGeneration++
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
        immersiveHelp = ImmersiveModeHelp(ContextThemeWrapper(InterfaceLanguage.context(this), R.style.Theme_Mimi), overlayWindow = true)
        fun label(tag: String, text: Int?, description: Int) {
            val control = overlayView?.findViewWithTag<View>(tag)
                ?: immersiveExitView?.takeIf { it.tag == tag } ?: return
            if (text != null && control is TextView) control.text = interfaceString(text)
            control.contentDescription = interfaceString(description)
            control.tooltipText = control.contentDescription
        }
        updateOverlayFontSize()
        label("exit-immersive", null, R.string.overlay_exit_immersive)
        label("overlay-drag", null, R.string.overlay_drag)
        label("overlay-resize", null, R.string.overlay_resize)
        adjustmentSwitch?.contentDescription = interfaceString(R.string.overlay_adjust)
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

        adjustable = SettingsStore.overlayAdjustable(this)
        overlayPlacement.reset(SettingsStore.overlayYOffset(this), SettingsStore.immersiveSubtitles(this))
        overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(this))
        val compact = app.yuxino.mimi.android.CompactCaptionView(this)
        val status = compact.status; val source = compact.source; val translation = compact.translation
        compact.applyStyle(app.yuxino.mimi.android.CaptionFrameStyle.saved(this, sessionOriginalOnly), immersiveSession)
        val panel = buildExpandedPanel()
        val root = app.yuxino.mimi.android.CaptionOverlayHost(this).apply {
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
                or (if (immersiveSession || !adjustable) WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE else 0),
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
            y = overlayPlacement.compactY(resources.displayMetrics.density, overlayAvailableSize().second, 0)
            fitOverlayInsets(this)
            // Android 12+ passes touches through an untrusted overlay only when
            // its window opacity stays at or below the system threshold (0.8).
            // Android blocks touches through an obscuring overlay above .8.
            // Keep this cap in BOTH adjustment states so toggling never dims captions.
            alpha = 0.8f
        }

        fun attachDrag(control: View) {
            var initialY = 0
            var initialX = 0
            var touchY = 0f
            var touchX = 0f
            var dragged = false
            var gestureGeneration = -1
            control.setOnTouchListener { _, event ->
                if (immersiveSession || !adjustable) return@setOnTouchListener false
                if (event.actionMasked != MotionEvent.ACTION_DOWN && gestureGeneration != overlayGeometryGeneration)
                    return@setOnTouchListener true
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        gestureGeneration = overlayGeometryGeneration
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
                            updateOverlayGeometry()
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
        var resizeGestureGeneration = -1
        panel.findViewWithTag<View>("overlay-resize").setOnTouchListener { control, event ->
            if (immersiveSession || !adjustable) return@setOnTouchListener false
            if (event.actionMasked != MotionEvent.ACTION_DOWN && resizeGestureGeneration != overlayGeometryGeneration)
                return@setOnTouchListener true
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    resizeGestureGeneration = overlayGeometryGeneration
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
                        resizeHeightPercent = ((resizeStartHeight + deltaY) * 100 / height).toInt().coerceIn(10, 70)
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
        if (immersiveSession) showImmersiveExitControl(wm) else showAdjustmentSwitch(wm)
        (panel as app.yuxino.mimi.android.CaptionPanelView).setAdjustable(adjustable)
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
        0,
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
        val panel = app.yuxino.mimi.android.CaptionPanelView(this,
            app.yuxino.mimi.android.CaptionPanelActions(
                home = { startActivity(Intent(this, app.yuxino.mimi.android.MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) },
            ))
        panel.applyStyle(app.yuxino.mimi.android.CaptionFrameStyle.saved(this, sessionOriginalOnly))
        expandedStatusView = panel.status; expandedSourceView = panel.source
        expandedTranslationView = panel.translation; historyView = panel.history
        transcriptScrollView = panel.transcriptScroll
        panel.status.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> updateOverlayGeometry() }
        listOf(panel.source, panel.translation, panel.timestamp).forEach { row ->
            row.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
                mainHandler.post { if (expandedView === panel) updateOverlayGeometry() }
            }
        }
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

    /** A separate touchable window is essential: the locked caption receives no input. */
    private fun showAdjustmentSwitch(wm: WindowManager) {
        if (adjustmentSwitch != null) return
        val themed = ContextThemeWrapper(InterfaceLanguage.context(this), R.style.Theme_Mimi)
        // Paint belongs to CaptionPanelView so native and preview share one
        // surface. This fully inset window owns only input and accessibility.
        val control = CaptionAdjustmentSwitch(themed, paintsControl = false).apply {
            tag = "overlay-adjust"
            isChecked = adjustable
            setOnCheckedChangeListener { _, enabled ->
                if (!syncingAdjustment) {
                    val previous = adjustable
                    try {
                        SettingsStore.setOverlayAdjustable(this@MimiService, enabled)
                        applyAdjustment(enabled)
                    } catch (_: RuntimeException) {
                        if (adjustable != previous) runCatching {
                            SettingsStore.setOverlayAdjustable(this@MimiService, previous)
                            applyAdjustment(previous)
                        }
                        syncingAdjustment = true
                        isChecked = previous
                        syncingAdjustment = false
                        Toast.makeText(themed, R.string.service_save_failed, Toast.LENGTH_LONG).show()
                    }
                }
            }
            addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
                if (adjustmentSwitch === this) updateOverlayGeometry()
            }
        }
        val params = WindowManager.LayoutParams(dp(CaptionAdjustmentSwitch.TARGET_DP), dp(CaptionAdjustmentSwitch.TARGET_DP),
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT).apply {
            gravity = Gravity.BOTTOM or Gravity.RIGHT
            fitOverlayInsets(this)
        }
        wm.addView(control, params)
        adjustmentSwitch = control
        adjustmentParams = params
        updateOverlayGeometry()
    }

    private fun applyAdjustment(enabled: Boolean) {
        if (adjustable != enabled) {
            // Cancel, rather than persist, a gesture whose input ownership changed.
            overlayGeometryGeneration++
            draggingOverlay = false
            resizeWidthPercent = null
            resizeHeightPercent = null
            adjustable = enabled
        }
        syncingAdjustment = true
        adjustmentSwitch?.isChecked = enabled
        syncingAdjustment = false
        (expandedView as? app.yuxino.mimi.android.CaptionPanelView)?.setAdjustable(enabled)
        overlayParams?.let { params ->
            val flags = if (immersiveSession || !enabled) params.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
                else params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
            val alpha = .8f
            if (params.flags != flags || params.alpha != alpha) {
                params.flags = flags; params.alpha = alpha
                overlayView?.let { windowManager?.updateViewLayout(it, params) }
            }
        }
        updateOverlayGeometry()
    }

    private fun applyAppearance() {
        val root = overlayView ?: return
        if (overlayPlacement.setImmersive(SettingsStore.immersiveSubtitles(this))) {
            draggingOverlay = false
            resizeWidthPercent = null; resizeHeightPercent = null
            overlayGeometryGeneration++
            val params = checkNotNull(overlayParams)
            params.flags = if (immersiveSession || !adjustable) params.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
                else params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
            params.alpha = 0.8f
            // Update child presentation before WindowManager measures the new
            // compact frame or positions its companion restore control.
            compactView?.visibility = if (expanded) View.GONE else View.VISIBLE
            expandedView?.visibility = if (expanded) View.VISIBLE else View.GONE
            if (immersiveSession) {
                adjustmentSwitch?.let { windowManager?.removeView(it) }
                adjustmentSwitch = null; adjustmentParams = null
                showImmersiveExitControl(checkNotNull(windowManager))
            } else {
                immersiveExitView?.let { windowManager?.removeView(it) }
                immersiveExitView = null
                immersiveExitParams = null
                showAdjustmentSwitch(checkNotNull(windowManager))
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
        applyAdjustment(SettingsStore.overlayAdjustable(this))
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
        val frameHeight = if (expanded) expandedPanelHeight() else WindowManager.LayoutParams.WRAP_CONTENT
        val desiredHeight = frameHeight
        val captionHeight = compactView?.height ?: 0
        val preferredY = when {
            draggingOverlay -> OverlayPlacement.clampY(params.y, height, if (expanded) desiredHeight else captionHeight)
            expanded -> overlayPlacement.expandedY(resources.displayMetrics.density, height, desiredHeight)
            else -> overlayPlacement.compactY(resources.displayMetrics.density, height, captionHeight)
        }
        val desiredY = OverlayPlacement.clampY(preferredY, height,
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
        adjustmentParams?.let { controlParams ->
            val controlWidth = dp(CaptionAdjustmentSwitch.TARGET_DP)
            val controlHeight = dp(CaptionAdjustmentSwitch.TARGET_DP)
            val x = OverlayPlacement.adjustmentX(width, desiredWidth, params.x, controlWidth,
                dp(CaptionAdjustmentSwitch.RIGHT_INSET_DP))
            val y = OverlayPlacement.adjustmentY(height, params.y, desiredHeight, controlHeight, 0)
            if (controlParams.x != x || controlParams.y != y) {
                controlParams.x = x; controlParams.y = y
                adjustmentSwitch?.let { windowManager?.updateViewLayout(it, controlParams) }
            }
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
        val panel = expandedView as? app.yuxino.mimi.android.CaptionPanelView
        val chrome = panel?.chromeHeight() ?: dp(app.yuxino.mimi.android.CaptionPanelView.CHROME_HEIGHT_DP)
        val reading = panel?.minimumReadingHeight() ?: dp(64)
        return app.yuxino.mimi.android.captionPanelHeight(availableHeight,
            resizeHeightPercent ?: SettingsStore.overlayHeightPercent(this), chrome, reading)
    }

    private fun hideOverlay() {
        adjustmentSwitch?.let { windowManager?.removeView(it) }
        adjustmentSwitch = null; adjustmentParams = null
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
            it.showContent(content, mode)
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
        adjustmentSwitch?.visibility = if (coveredByOwnUi) View.GONE else View.VISIBLE
    }

    private fun interfaceString(@androidx.annotation.StringRes id: Int): String =
        InterfaceLanguage.context(this).getString(id)

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
        private val startGate = CaptureStartGate()
        private var startOwner: Pair<Long, java.lang.ref.WeakReference<android.app.Activity>>? = null
        private fun activityVisible(activity: android.app.Activity): Boolean =
            !activity.isFinishing && !activity.isDestroyed && activity.window.decorView.isShown &&
                activity.window.decorView.windowVisibility == View.VISIBLE
        private fun startOwnerVisible(request: Long): Boolean = startOwner?.takeIf { it.first == request }
            ?.second?.get()?.let(::activityVisible) == true
        /** Visible-activity dispatch can be rejected before promotion without an FGS promise.
         * Accepted requests immediately enter the required typed FGS before capture. */
        internal fun startFromActivity(activity: android.app.Activity, intent: Intent) {
            val request = intent.getLongExtra(EXTRA_START_REQUEST, 0L).takeIf { it != 0L } ?: prepareStart(activity)
            try {
                check(startPending(request) && activityVisible(activity)) { "capture_activity_not_visible" }
                startOwner = request to java.lang.ref.WeakReference(activity)
                checkNotNull(activity.startService(intent.putExtra(EXTRA_START_REQUEST, request))) { "capture_start_failed" }
            } catch (error: RuntimeException) {
                cancelStartRequest(request)
                throw error
            }
        }
        private const val START_DELIVERY_TIMEOUT_MS = 10_000L
        val isStarting: Boolean get() = startGate.isPending
        internal fun startPending(request: Long): Boolean = startGate.waiting(request)
        internal fun prepareStart(context: Context): Long {
            check(!isRunning && !isStopping) { "capture_session_active" }
            val request = startGate.begin()
            lastCaptureError = null
            val appContext = context.applicationContext
            Handler(Looper.getMainLooper()).postDelayed({
                if (startPending(request)) {
                    lastCaptureError = "capture.start_failed"
                    completeStartRequest(request)
                    Toast.makeText(InterfaceLanguage.context(appContext), R.string.capture_failed, Toast.LENGTH_LONG).show()
                }
            }, START_DELIVERY_TIMEOUT_MS)
            return request
        }
        internal fun cancelStartRequest(request: Long) {
            if (startGate.waiting(request)) lastCaptureError = "capture.start_failed"
            completeStartRequest(request)
        }
        private fun completeStartRequest(request: Long) {
            if (startGate.complete(request)) {
                if (startOwner?.first == request) startOwner = null
                stateListeners.forEach { it() }
            }
        }

        private val ownUiOwners = CopyOnWriteArraySet<android.app.Activity>()
        private var ownUiChanged: (() -> Unit)? = null
        private val ownUiVisible: Boolean get() = ownUiOwners.isNotEmpty()
        internal fun setOwnUiVisible(owner: android.app.Activity, visible: Boolean) {
            val changed = if (visible) ownUiOwners.add(owner) else ownUiOwners.remove(owner)
            if (changed) ownUiChanged?.invoke()
        }
        @Volatile private var captureEnginesForTests: Map<CaptureSource, ProviderEngine>? = null
        @Volatile internal var captureForegroundTypesForTests: Int = 0
            private set
        @Volatile internal var captureProjectionForTests: MediaProjection? = null
            private set

        /** The release app cannot replace its provider or expose its projection through this seam. */
        internal fun setCaptureEngineForTests(context: Context, value: ProviderEngine?) {
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!isRunning)
            captureEnginesForTests = value?.let { mapOf(CaptureSource.SYSTEM to it) }
        }

        internal fun setCaptureEnginesForTests(context: Context, value: Map<CaptureSource, ProviderEngine>?) {
            check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
            check(!isRunning)
            captureEnginesForTests = value?.toMap()
        }

        private val historyStatusGeneration = AtomicLong()
        /** A terminal runtime can release its journal after its service is destroyed. */
        private fun observeHistoryFinalization(handle: Long) {
            val owner = historyStatusGeneration.get()
            val timeout = NativeRuntimeConfiguration.policy.getLong("sessionFinishTimeoutMs")
            thread(name = "mimi-history-finalize", isDaemon = true) {
                var failed = false
                var limited = false
                try {
                    val deadline = SystemClock.elapsedRealtime() + timeout
                    while (historyStatusGeneration.get() == owner) {
                        val result = NativeRuntimeConfiguration.exchange(JSONObject()
                            .put("operation", "history_state").put("historyHandle", handle))
                        failed = result.getJSONObject("state").getBoolean("saveError")
                        limited = result.getJSONObject("state").getBoolean("limited")
                        if (!result.getBoolean("pending")) break
                        if (SystemClock.elapsedRealtime() >= deadline) { failed = true; break }
                        Thread.sleep(100)
                    }
                } catch (_: Exception) { failed = true }
                Handler(Looper.getMainLooper()).post {
                    if (historyStatusGeneration.get() == owner) {
                        historySaveError = historySaveError || failed
                        historyLimited = historyLimited || limited
                        stateListeners.forEach { it() }
                    }
                }
            }
        }

        @Volatile var historySaveError: Boolean = false
            private set
        @Volatile var historyLimited: Boolean = false
            private set
        @Volatile var captureInput: CaptureInput = CaptureInput.SYSTEM
            private set
        @Volatile var captureObservations: Map<CaptureSource, CaptureHealth.Snapshot> = emptyMap()
            private set
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
        const val EXTRA_CAPTURE_INPUT = "capture_input"
        internal const val EXTRA_START_REQUEST = "capture_start_request"

        fun startIntent(context: Context, resultCode: Int, resultData: Intent, input: CaptureInput = CaptureInput.SYSTEM): Intent =
            Intent(context, MimiService::class.java)
                .putExtra(EXTRA_CAPTURE_INPUT, input.id)
                .putExtra(EXTRA_RESULT_CODE, resultCode)
                .putExtra(EXTRA_RESULT_DATA, resultData)

        fun microphoneIntent(context: Context): Intent =
            Intent(context, MimiService::class.java).putExtra(EXTRA_CAPTURE_INPUT, CaptureInput.MICROPHONE.id)

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
