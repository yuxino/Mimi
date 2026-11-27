package app.yuxino.mimi.android.capture

import android.Manifest
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.View
import android.view.WindowManager
import android.widget.Toast
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.MimiApplication
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.localization.InterfaceLanguage
import app.yuxino.mimi.android.onboarding.FirstRunEvidence
import app.yuxino.mimi.android.provider.EngineListener
import app.yuxino.mimi.android.provider.NativeRuntimeConfiguration
import app.yuxino.mimi.android.provider.ProviderEngine
import app.yuxino.mimi.android.provider.SharedRuntimeEngine
import app.yuxino.mimi.android.provider.SubtitleBus
import app.yuxino.mimi.android.services.presentation.runtimeModelNames
import app.yuxino.mimi.android.settings.SettingsStore
import java.util.concurrent.CopyOnWriteArraySet
import java.util.concurrent.atomic.AtomicBoolean
import org.json.JSONObject

/**
 * Foreground service that owns independent selected audio-capture lanes and
 * coordinates the overlay window, session history archive and notification.
 */
class MimiService : Service() {

    private val mainHandler = Handler(Looper.getMainLooper())

    private var mediaProjection: MediaProjection? = null
    private val lanes = linkedMapOf<CaptureSource, CaptureLane>()
    private val healthTick = object : Runnable {
        override fun run() {
            if (lanes.isEmpty()) return
            val permissionsValid = capturePermissionsValid()
            val locked = screenLocked()
            if (!permissionsValid || locked) {
                queueUnlockContinuation(permissionsValid, locked)
                lastCaptureError = "capture.permission_or_lock_changed"
                stopEverything()
                return
            }
            updateCaptureObservations()
            stateListeners.forEach { listener -> listener() }
            overlayWindow.render()
            mainHandler.postDelayed(this, 1_000)
        }
    }

    private fun capturePermissionsValid(): Boolean = android.provider.Settings.canDrawOverlays(this) &&
        ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private fun screenLocked(): Boolean = getSystemService(android.app.KeyguardManager::class.java).isKeyguardLocked ||
        !getSystemService(android.os.PowerManager::class.java).isInteractive

    private fun queueUnlockContinuation(permissionsValid: Boolean, locked: Boolean) {
        (application as MimiApplication).captionShortcuts.onLockInterruption(
            captureInput,
            active = isRunning && mediaProjection != null && lanes.isNotEmpty(),
            preview = overlayWindow.isPreviewMode,
            stopping = finishingSession || isStopping,
            permissionsValid = permissionsValid,
            locked = locked,
        )
    }

    private fun updateCaptureObservations() {
        val now = SystemClock.elapsedRealtime()
        captureObservations = lanes.mapNotNull { (source, lane) -> lane.observation(now)?.let { source to it } }.toMap()
        captureObservation = captureObservationForSources(captureObservations.values)
    }

    private var stopObservingStorage: (() -> Unit)? = null
    private var generation = 0
    private var finishingSession = false
    private var projectionCallback: MediaProjection.Callback? = null

    // Retained on MimiService for instrumentation geometry checks reading this window host.
    @Suppress("unused")
    private var windowManager: WindowManager? = null

    private val overlayWindow = CaptureOverlayWindow(
        service = this,
        mainHandler = mainHandler,
        onWindowManagerChanged = { windowManager = it },
        currentGeneration = { generation },
        isRunning = { isRunning },
        isStopping = { isStopping },
        currentObservation = { captureObservation },
        onFirstRunCompleted = { stateListeners.forEach { it() } },
    )
    private val historyTracker = SessionHistoryTracker(
        context = this,
        readStatus = { historySaveError to historyLimited },
        writeStatus = { saveError, limited, notify ->
            historySaveError = saveError
            historyLimited = limited
            if (notify) stateListeners.forEach { it() }
        },
        onAsyncFinalized = Companion::applyFinalizedHistoryStatus,
    )

    private val ownUiUpdate = Runnable { overlayWindow.render() }
    private var interfaceLocaleTags = ""
    private val interfaceLanguageListener: () -> Unit = { mainHandler.post { refreshInterfaceLanguage() } }
    private var stopObservingAppearance: (() -> Unit)? = null
    private val appearanceUpdate = Runnable { overlayWindow.applyAppearance() }
    private val captionUpdatePending = AtomicBoolean(false)
    private val captionUpdate = Runnable {
        captionUpdatePending.set(false)
        overlayWindow.render()
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
        interfaceLocaleTags = InterfaceLanguage.context(this).resources.configuration.locales.toLanguageTags()
        InterfaceLanguage.addListener(interfaceLanguageListener)
        stopObservingAppearance = SettingsStore.observeAppearance(this) {
            mainHandler.removeCallbacks(appearanceUpdate)
            mainHandler.post(appearanceUpdate)
        }
        stopObservingStorage = SettingsStore.observeSessionStorage(this) {
            mainHandler.post {
                if (!SettingsStore.retainSessionHistory(this)) historyTracker.disable()
            }
        }
        ownUiChanged = {
            mainHandler.removeCallbacks(ownUiUpdate)
            if (ownUiVisible) overlayWindow.render() else mainHandler.postDelayed(ownUiUpdate, 120)
        }
        CaptureServiceNotification.createChannel(this)
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
            (application as MimiApplication).captionShortcuts.onCaptureStartingOrUserStop()
            startOwner?.first?.let(::completeStartRequest)
            finishSession()
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_APPLY_APPEARANCE) {
            overlayWindow.applyAppearance()
            if (!overlayWindow.isShowing && !isRunning) stopSelf()
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_UI_PREVIEW_HISTORY && overlayWindow.isPreviewMode &&
            applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        ) {
            overlayWindow.enablePreviewHistory()
            return START_NOT_STICKY
        }
        if (isRunning || overlayWindow.isShowing) return START_NOT_STICKY
        // Instrumentation can exercise the ordinary consent/shortcut path with an
        // in-process discard engine. A fresh visible-activity request is still required.
        val testEngines = if ((intent?.action == null || intent.action == ACTION_CAPTURE_TEST) &&
            applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        ) captureEnginesForTests else null
        if (intent?.action == ACTION_CAPTURE_TEST && testEngines == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        // Debug-only screenshot fixture: the real overlay with synthetic
        // subtitles and no MediaProjection, provider, or credential access.
        if (intent?.action == ACTION_UI_PREVIEW &&
            applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        ) {
            overlayWindow.showPreview(overOwnUi = intent.getBooleanExtra("preview_over_own_ui", false))
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
            testEngines != null && testEngines.keys != input.sources.toSet()
        ) {
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
            overlayWindow.render()
        } catch (_: Exception) {
            lastCaptureError = "capture.start_failed"
            Log.w(TAG, "capture_start_failed")
            Toast.makeText(InterfaceLanguage.context(this), R.string.capture_failed, Toast.LENGTH_LONG).show()
            stopEverything()
        }
        return START_NOT_STICKY
    }

    private fun startAsForeground(input: CaptureInput) {
        startForeground(
            CaptureServiceNotification.NOTIFICATION_ID,
            CaptureServiceNotification.build(this, input),
            CaptureServiceNotification.foregroundServiceType(input),
        )
        captureForegroundTypesForTests = foregroundServiceType
    }

    private fun startCapture(
        input: CaptureInput,
        projection: MediaProjection?,
        testEngines: Map<CaptureSource, ProviderEngine>? = null,
    ) {
        check(ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
            "audio_permission_required"
        }
        check(
            android.provider.Settings.canDrawOverlays(this) &&
                getSystemService(android.os.PowerManager::class.java).isInteractive &&
                !getSystemService(android.app.KeyguardManager::class.java).isDeviceLocked,
        ) { "capture_permission_or_lock_changed" }
        val sessionGeneration = ++generation
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(SettingsStore.historyLines(this))
        synchronized(firstRunEvidence) { firstRunEvidence.reset() }
        if (projection != null) {
            val callback = object : MediaProjection.Callback() {
                override fun onStop() {
                    if (generation == sessionGeneration) {
                        queueUnlockContinuation(capturePermissionsValid(), screenLocked())
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
        val sessionOriginalOnly = (configuration?.getString("targetLanguage") ?: "original") == "original"
        activeModelNames = configuration?.let { runtimeModelNames(it) } ?: emptyList()
        val historyHandle = historyTracker.begin(testEngines == null && SettingsStore.retainSessionHistory(this))

        for (source in input.sources) {
            fun dispatch(action: () -> Unit) {
                mainHandler.post { if (generation == sessionGeneration) action() }
            }
            val sessionEngine = testEngines?.get(source) ?: run {
                val listener = object : EngineListener {
                    override fun onRuntimeSnapshot(state: JSONObject, originalOnly: Boolean) = dispatch {
                        historyTracker.onSnapshot(source, state)
                        SubtitleBus.onRuntimeSnapshot(state, originalOnly, source.id)
                        if (SubtitleBus.displayPairFinal) scheduleAutoHide() else cancelAutoHide()
                    }
                    override fun onError(code: String) = dispatch {
                        Toast.makeText(InterfaceLanguage.context(this@MimiService), providerErrorMessageResource(code), Toast.LENGTH_LONG).show()
                        stopEverything()
                    }
                    override fun onClosed() = dispatch { if (!finishingSession) stopEverything() }
                }
                SharedRuntimeEngine(checkNotNull(configuration), listener, source.id, historyHandle)
            }
            val lane = CaptureLane(source, sessionEngine)
            lanes[source] = lane
            if (testEngines == null) sessionEngine.start()
            lane.prepare(projection)
        }
        // Prepare every selected lane before admitting any audio. A failed lane
        // tears the whole session down; it cannot silently become single-source.
        for (lane in lanes.values) startCaptureLane(lane, sessionGeneration)
        updateCaptureObservations()
        overlayWindow.show(originalOnly = sessionOriginalOnly)
        mainHandler.post(healthTick)
    }

    private fun startCaptureLane(lane: CaptureLane, sessionGeneration: Int) {
        lane.start(
            onAudioSubmitted = { pcm ->
                synchronized(firstRunEvidence) { if (lane.isCapturing) firstRunEvidence.submitted(pcm) }
            },
            onFailure = {
                mainHandler.post {
                    if (generation == sessionGeneration) {
                        lastCaptureError = "capture.read_failed"
                        Toast.makeText(InterfaceLanguage.context(this), R.string.capture_failed, Toast.LENGTH_LONG).show()
                        stopEverything()
                    }
                }
            },
        )
    }

    private fun releaseSession() {
        ++generation
        finishingSession = false
        isStopping = false
        captureObservation = null
        captureObservations = emptyMap()
        mainHandler.removeCallbacksAndMessages(null)
        captionUpdatePending.set(false)
        val retired = lanes.values.toList()
        lanes.clear()
        retired.forEach(CaptureLane::stopCapture)
        retired.forEach(CaptureLane::release)
        historyTracker.finish(retain = SettingsStore.retainSessionHistory(this))
        synchronized(firstRunEvidence) { firstRunEvidence.reset() }
        val projection = mediaProjection
        mediaProjection = null
        captureProjectionForTests = null
        captureForegroundTypesForTests = 0
        projectionCallback?.let { projection?.unregisterCallback(it) }
        projectionCallback = null
        try { projection?.stop() } catch (_: Exception) { }
        SubtitleBus.finishSnapshot()
        overlayWindow.hide()
        activeModelNames = emptyList()
        setRunning(false)
    }

    /** User stop drains accepted work; revocation/errors/destruction still abort. */
    private fun finishSession() {
        if (finishingSession) return
        if (lanes.isEmpty() || overlayWindow.isPreviewMode) {
            stopEverything()
            return
        }
        finishingSession = true
        isStopping = true
        stateListeners.forEach { it() }
        val owner = generation
        lanes.values.forEach(CaptureLane::stopCapture)
        mainHandler.removeCallbacks(healthTick)
        cancelAutoHide()
        val policy = NativeRuntimeConfiguration.policy
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
                        overlayWindow.render()
                        mainHandler.post { if (generation == owner && finishingSession) stopEverything() }
                    }
                }
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
        stopObservingStorage?.invoke()
        stopObservingStorage = null
        SubtitleBus.removeListener(busListener)
        releaseSession()
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        overlayWindow.onConfigurationChanged()
        mainHandler.post {
            refreshInterfaceLanguage()
            overlayWindow.applyAppearance()
        }
    }

    private fun refreshInterfaceLanguage() {
        val localeTags = InterfaceLanguage.context(this).resources.configuration.locales.toLanguageTags()
        if (localeTags == interfaceLocaleTags) return
        interfaceLocaleTags = localeTags
        overlayWindow.refreshInterfaceLanguage()
        if (isRunning) CaptureServiceNotification.refresh(this, captureInput)
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
        val isSessionBusy: Boolean get() = isStarting || isRunning || isStopping
        internal fun startPending(request: Long): Boolean = startGate.waiting(request)
        internal fun prepareStart(context: Context): Long {
            check(!isRunning && !isStopping) { "capture_session_active" }
            val request = startGate.begin()
            (context.applicationContext as MimiApplication).captionShortcuts.onCaptureStartingOrUserStop()
            lastCaptureError = null
            stateListeners.forEach { it() }
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
        internal val ownUiVisible: Boolean get() = ownUiOwners.isNotEmpty()
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

        private fun applyFinalizedHistoryStatus(failed: Boolean, limited: Boolean) {
            historySaveError = historySaveError || failed
            historyLimited = historyLimited || limited
            stateListeners.forEach { it() }
        }

        @Volatile var historySaveError: Boolean = false
            private set
        @Volatile var historyLimited: Boolean = false
            private set
        @Volatile var captureInput: CaptureInput = CaptureInput.SYSTEM
            private set
        @Volatile var captureObservations: Map<CaptureSource, CaptureHealth.Snapshot> = emptyMap()
            private set
        val firstRunEvidence = FirstRunEvidence()
        private const val TAG = "MimiService"
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
