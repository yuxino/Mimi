package app.yuxino.mimi.android.capture

import android.content.Context
import android.content.Intent
import android.graphics.PixelFormat
import android.graphics.Rect
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Handler
import android.view.ContextThemeWrapper
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.TextView
import android.widget.Toast
import app.yuxino.mimi.android.MainActivity
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.captions.CaptionAdjustmentSwitch
import app.yuxino.mimi.android.captions.CaptionColorMath
import app.yuxino.mimi.android.captions.CaptionDisplayMode
import app.yuxino.mimi.android.captions.CaptionFrameContent
import app.yuxino.mimi.android.captions.CaptionFrameStyle
import app.yuxino.mimi.android.captions.CaptionOpacitySlider
import app.yuxino.mimi.android.captions.CaptionOverlayHost
import app.yuxino.mimi.android.captions.CaptionPanelActions
import app.yuxino.mimi.android.captions.CaptionPanelView
import app.yuxino.mimi.android.captions.CaptionSurface
import app.yuxino.mimi.android.captions.CompactCaptionView
import app.yuxino.mimi.android.captions.captionPanelHeight
import app.yuxino.mimi.android.localization.InterfaceLanguage
import app.yuxino.mimi.android.provider.SubtitleBus
import app.yuxino.mimi.android.settings.SettingsStore
import com.google.android.material.materialswitch.MaterialSwitch

/**
 * Owns the floating subtitle window, companion adjustment/immersive-exit controls,
 * placement and drag/resize gestures, insets geometry, appearance updates, and SubtitleBus rendering.
 */
internal class CaptureOverlayWindow(
    private val service: MimiService,
    private val mainHandler: Handler,
    private val onWindowManagerChanged: (WindowManager) -> Unit,
    private val currentGeneration: () -> Int,
    private val isRunning: () -> Boolean,
    private val isStopping: () -> Boolean,
    private val currentObservation: () -> CaptureHealth.Snapshot?,
    private val onFirstRunCompleted: () -> Unit,
) {
    private var windowManager: WindowManager? = null
    private var overlayView: View? = null
    private var overlayParams: WindowManager.LayoutParams? = null
    private var immersiveExitView: View? = null
    private var immersiveExitParams: WindowManager.LayoutParams? = null
    private var compactView: CompactCaptionView? = null
    private var expandedView: CaptionPanelView? = null
    private val overlayPlacement = OverlayPlacement()
    private val expanded get() = overlayPlacement.isExpanded
    private val immersiveSession get() = overlayPlacement.isImmersive
    private var draggingOverlay = false
    private var resizeWidthPercent: Int? = null
    private var resizeHeightPercent: Int? = null
    private var overlayGeometryGeneration = 0
    private var previewMode = false
    private var previewHistoryEnabled = false
    private var previewOverOwnUi = false
    private var sessionOriginalOnly = false
    private var statusView: TextView? = null
    private var sourceView: TextView? = null
    private var translationView: TextView? = null
    private var expandedSourceView: TextView? = null
    private var expandedTranslationView: TextView? = null
    private var adjustmentSwitch: MaterialSwitch? = null
    private var adjustmentParams: WindowManager.LayoutParams? = null
    private var adjustable = true
    private var syncingAdjustment = false

    val isShowing: Boolean get() = overlayView != null
    val isPreviewMode: Boolean get() = previewMode

    fun showPreview(overOwnUi: Boolean) {
        previewMode = true
        previewOverOwnUi = overOwnUi
        previewHistoryEnabled = false
        SubtitleBus.clear()
        SubtitleBus.setHistoryLimit(0)
        SubtitleBus.onSourceDraft("もう少し歩いてみましょう。", "ja")
        SubtitleBus.onTranslationDraft("再往前走一会儿吧。")
        show(originalOnly = false)
    }

    fun enablePreviewHistory() {
        previewHistoryEnabled = true
        seedPreviewHistory()
        render()
    }

    fun onConfigurationChanged() {
        // A gesture measured in the previous screen's axes cannot be committed
        // after rotation. Reapply saved bounds and clamp only the visible frame.
        draggingOverlay = false
        resizeWidthPercent = null
        resizeHeightPercent = null
        overlayGeometryGeneration++
    }

    fun refreshInterfaceLanguage() {
        fun label(tag: String, description: Int) {
            val control = overlayView?.findViewWithTag<View>(tag)
                ?: immersiveExitView?.takeIf { it.tag == tag } ?: return
            control.contentDescription = interfaceString(description)
            control.tooltipText = control.contentDescription
        }
        updateOverlayFontSize()
        label("exit-immersive", R.string.overlay_exit_immersive)
        label("overlay-drag", R.string.overlay_drag)
        label("overlay-resize", R.string.overlay_resize)
        label("overlay-opacity-slider", R.string.settings_background)
        adjustmentSwitch?.contentDescription = interfaceString(R.string.overlay_adjust)
        expandedView?.requestLayout()
        render()
    }

    /** The actual floating window: a small live line that opens a bounded reading panel. */
    fun show(originalOnly: Boolean) {
        sessionOriginalOnly = originalOnly
        if (overlayView != null) return
        val displayManager = service.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        @Suppress("DEPRECATION")
        val overlayContext = if (Build.VERSION.SDK_INT >= 30) {
            service.createDisplayContext(displayManager.defaultDisplay)
                .createWindowContext(WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY, null)
        } else service
        val wm = overlayContext.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        windowManager = wm
        onWindowManagerChanged(wm)

        adjustable = SettingsStore.overlayAdjustable(service)
        overlayPlacement.reset(SettingsStore.overlayYOffset(service), SettingsStore.immersiveSubtitles(service))
        overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(service))
        val compact = CompactCaptionView(service)
        val status = compact.status
        val source = compact.source
        val translation = compact.translation
        compact.applyStyle(CaptionFrameStyle.saved(service, sessionOriginalOnly), immersiveSession)
        val panel = buildExpandedPanel()
        val root = CaptionOverlayHost(service).apply {
            tag = "mimi-overlay"
            addView(
                compact,
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT,
                    FrameLayout.LayoutParams.WRAP_CONTENT,
                    Gravity.CENTER,
                ),
            )
            compact.visibility = if (expanded) View.GONE else View.VISIBLE
            panel.visibility = if (expanded) View.VISIBLE else View.GONE
            addView(
                panel,
                FrameLayout.LayoutParams(
                    FrameLayout.LayoutParams.MATCH_PARENT,
                    FrameLayout.LayoutParams.MATCH_PARENT,
                ),
            )
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
            y = overlayPlacement.compactY(service.resources.displayMetrics.density, overlayAvailableSize().second, 0)
            fitOverlayInsets(this)
            // Android 12+ passes touches through an untrusted overlay only when
            // its window opacity stays at or below the system threshold (0.8).
            // Android blocks touches through an obscuring overlay above .8.
            // Keep this cap in BOTH adjustment states so toggling never dims captions.
            alpha = 0.8f
        }

        attachDragGesture(panel.findViewWithTag("overlay-drag"), root, params, wm)
        attachResizeGesture(panel.findViewWithTag("overlay-resize"), root, params)
        attachOpacityGesture(panel.opacitySlider, panel)

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
        val renderGeneration = currentGeneration()
        root.viewTreeObserver.addOnDrawListener {
            val caption = (if (expanded) listOf(expandedTranslationView, expandedSourceView)
            else listOf(translationView, sourceView)).firstOrNull { it?.isShown == true && !it.text.isNullOrBlank() }
            val newlyComplete = synchronized(MimiService.firstRunEvidence) {
                val wasComplete = MimiService.firstRunEvidence.complete
                if (currentGeneration() == renderGeneration && isRunning() && caption != null) {
                    MimiService.firstRunEvidence.rendered(
                        !caption.text.isNullOrBlank(),
                        caption.isShown && caption.width > 0 && caption.height > 0 &&
                            android.provider.Settings.canDrawOverlays(service),
                        previewMode,
                    )
                }
                !wasComplete && MimiService.firstRunEvidence.complete
            }
            if (newlyComplete) mainHandler.post {
                service.getSharedPreferences("first_run", 0).edit().putBoolean("completed", true).apply()
                onFirstRunCompleted()
            }
        }
        if (immersiveSession) showImmersiveExitControl(wm) else showAdjustmentSwitch(wm)
        panel.setAdjustable(adjustable)
        updateOverlayFontSize()
        render()
    }

    private fun attachDragGesture(
        control: View,
        root: View,
        params: WindowManager.LayoutParams,
        wm: WindowManager,
    ) {
        var initialY = 0
        var initialX = 0
        var touchY = 0f
        var touchX = 0f
        var dragged = false
        var gestureGeneration = -1
        control.setOnTouchListener { _, event ->
            if (immersiveSession || !adjustable) return@setOnTouchListener false
            if (event.actionMasked != MotionEvent.ACTION_DOWN && gestureGeneration != overlayGeometryGeneration) {
                return@setOnTouchListener true
            }
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    gestureGeneration = overlayGeometryGeneration
                    initialY = params.y
                    initialX = params.x
                    touchY = event.rawY
                    touchX = event.rawX
                    dragged = false
                    draggingOverlay = false
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    if (kotlin.math.abs(event.rawY - touchY) > dp(8) || kotlin.math.abs(event.rawX - touchX) > dp(8)) {
                        dragged = true
                    }
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
                            val density = service.resources.displayMetrics.density
                            SettingsStore.setOverlayPosition(
                                service,
                                true,
                                kotlin.math.round(params.x / density).toInt(),
                                kotlin.math.round(params.y / density).toInt(),
                            )
                            overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(service))
                        } catch (_: RuntimeException) {
                            Toast.makeText(InterfaceLanguage.context(service), R.string.service_save_failed, Toast.LENGTH_LONG).show()
                        }
                        updateOverlayGeometry()
                    } else {
                        control.performClick()
                    }
                    true
                }
                MotionEvent.ACTION_CANCEL -> {
                    draggingOverlay = false
                    updateOverlayGeometry()
                    true
                }
                else -> false
            }
        }
    }

    private fun attachResizeGesture(
        handle: View,
        root: View,
        params: WindowManager.LayoutParams,
    ) {
        var resizeStartWidth = 0
        var resizeStartHeight = 0
        var resizeStartX = 0f
        var resizeStartY = 0f
        var resizeStartOffsetX = 0
        var resizeStartOffsetY = 0
        var resizeGestureGeneration = -1
        handle.setOnTouchListener { control, event ->
            if (immersiveSession || !adjustable) return@setOnTouchListener false
            if (event.actionMasked != MotionEvent.ACTION_DOWN && resizeGestureGeneration != overlayGeometryGeneration) {
                return@setOnTouchListener true
            }
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    resizeGestureGeneration = overlayGeometryGeneration
                    resizeStartWidth = root.width
                    resizeStartHeight = root.height
                    resizeStartX = event.rawX
                    resizeStartY = event.rawY
                    resizeStartOffsetX = params.x
                    resizeStartOffsetY = params.y
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
                        updateOverlayGeometry()
                        expandedView?.requestLayout()
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    val width = resizeWidthPercent
                    val height = resizeHeightPercent
                    if (width == null || height == null) control.performClick()
                    else try {
                        val density = service.resources.displayMetrics.density
                        SettingsStore.setOverlayBounds(
                            service,
                            width,
                            height,
                            kotlin.math.round(params.x / density).toInt(),
                            kotlin.math.round(params.y / density).toInt(),
                        )
                        overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(service))
                    } catch (_: RuntimeException) {
                        Toast.makeText(InterfaceLanguage.context(service), R.string.service_save_failed, Toast.LENGTH_LONG).show()
                    }
                    resizeWidthPercent = null
                    resizeHeightPercent = null
                    draggingOverlay = false
                    updateOverlayGeometry()
                    true
                }
                MotionEvent.ACTION_CANCEL -> {
                    resizeWidthPercent = null
                    resizeHeightPercent = null
                    draggingOverlay = false
                    updateOverlayGeometry()
                    true
                }
                else -> false
            }
        }
    }

    private fun attachOpacityGesture(slider: CaptionOpacitySlider, panel: CaptionPanelView) {
        var gestureGeneration = -1
        var previewAlpha: Int? = null
        var baseStyle: CaptionFrameStyle? = null
        var moved = false

        fun restoreSaved() {
            previewAlpha = null
            baseStyle = null
            moved = false
            slider.dragging = false
            panel.applySurfaceChrome(CaptionFrameStyle.saved(service, sessionOriginalOnly))
        }

        slider.onAccessibilityStep = { next ->
            if (!immersiveSession && adjustable) {
                try {
                    SettingsStore.setOverlayBgAlpha(service, next)
                    panel.applySurfaceChrome(CaptionFrameStyle.saved(service, sessionOriginalOnly))
                } catch (_: RuntimeException) {
                    restoreSaved()
                    Toast.makeText(InterfaceLanguage.context(service), R.string.service_save_failed, Toast.LENGTH_LONG).show()
                }
            }
        }

        slider.setOnTouchListener { _, event ->
            if (immersiveSession || !adjustable) return@setOnTouchListener false
            if (event.actionMasked != MotionEvent.ACTION_DOWN && gestureGeneration != overlayGeometryGeneration) {
                if (previewAlpha != null || slider.dragging) restoreSaved()
                return@setOnTouchListener true
            }
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    slider.parent?.requestDisallowInterceptTouchEvent(true)
                    gestureGeneration = overlayGeometryGeneration
                    moved = false
                    slider.dragging = true
                    val style = CaptionFrameStyle.saved(service, sessionOriginalOnly)
                    baseStyle = style
                    val next = slider.valueFromY(event.y)
                    previewAlpha = next
                    panel.applySurfaceChrome(style.copy(backgroundAlpha = next))
                    true
                }
                MotionEvent.ACTION_MOVE -> {
                    moved = true
                    val style = baseStyle ?: CaptionFrameStyle.saved(service, sessionOriginalOnly)
                    val next = slider.valueFromY(event.y)
                    if (previewAlpha != next) {
                        previewAlpha = next
                        panel.applySurfaceChrome(style.copy(backgroundAlpha = next))
                    }
                    true
                }
                MotionEvent.ACTION_UP -> {
                    val committed = previewAlpha
                    val wasTap = !moved
                    previewAlpha = null
                    baseStyle = null
                    moved = false
                    slider.dragging = false
                    if (committed != null) {
                        try {
                            SettingsStore.setOverlayBgAlpha(service, committed)
                            panel.applySurfaceChrome(CaptionFrameStyle.saved(service, sessionOriginalOnly))
                        } catch (_: RuntimeException) {
                            restoreSaved()
                            Toast.makeText(InterfaceLanguage.context(service), R.string.service_save_failed, Toast.LENGTH_LONG).show()
                        }
                    }
                    if (wasTap) slider.performClick()
                    true
                }
                MotionEvent.ACTION_CANCEL -> {
                    restoreSaved()
                    true
                }
                else -> false
            }
        }
    }

    private fun showImmersiveExitControl(wm: WindowManager) {
        if (immersiveExitView != null) return
        val exit = FrameLayout(service).apply {
            tag = "exit-immersive"
            contentDescription = interfaceString(R.string.overlay_exit_immersive)
            isClickable = true
            isFocusable = true
            addView(
                ImageView(service).apply {
                    setImageResource(R.drawable.ic_overlay_close)
                    setPadding(dp(5), dp(5), dp(5), dp(5))
                    background = GradientDrawable().apply {
                        shape = GradientDrawable.OVAL
                        setColor(0x99000000.toInt())
                    }
                    importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                },
                FrameLayout.LayoutParams(dp(24), dp(24), Gravity.CENTER),
            )
            minimumWidth = dp(48)
            minimumHeight = dp(48)
            setOnClickListener { exitImmersiveMode() }
            addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
                if (immersiveExitView === this) updateOverlayGeometry()
            }
        }
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
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
        overlayParams?.y ?: 0,
        compactView?.height ?: 0,
        overlayAvailableSize().second,
        immersiveExitView?.height?.takeIf { it > 0 } ?: dp(48),
        dp(6),
    )

    private fun exitImmersiveMode() {
        if (!immersiveSession) return
        runCatching {
            SettingsStore.setImmersiveSubtitles(service, false)
            applyAppearance()
        }.onFailure {
            Toast.makeText(InterfaceLanguage.context(service), R.string.service_save_failed, Toast.LENGTH_LONG).show()
        }
    }

    private fun buildExpandedPanel(): CaptionPanelView {
        val panel = CaptionPanelView(
            service,
            CaptionPanelActions(
                home = { service.startActivity(Intent(service, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) },
            ),
        )
        panel.applyStyle(CaptionFrameStyle.saved(service, sessionOriginalOnly))
        expandedSourceView = panel.source
        expandedTranslationView = panel.translation
        panel.status.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> updateOverlayGeometry() }
        listOf(panel.source, panel.translation, panel.timestamp).forEach { row ->
            row.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
                mainHandler.post { if (expandedView === panel) updateOverlayGeometry() }
            }
        }
        return panel
    }

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
        val themed = ContextThemeWrapper(InterfaceLanguage.context(service), R.style.Theme_Mimi)
        // Paint belongs to CaptionPanelView so native and preview share one
        // surface. This fully inset window owns only input and accessibility.
        val control = CaptionAdjustmentSwitch(themed, paintsControl = false).apply {
            tag = "overlay-adjust"
            isChecked = adjustable
            setOnCheckedChangeListener { _, enabled ->
                if (!syncingAdjustment) {
                    val previous = adjustable
                    try {
                        SettingsStore.setOverlayAdjustable(service, enabled)
                        applyAdjustment(enabled)
                    } catch (_: RuntimeException) {
                        if (adjustable != previous) runCatching {
                            SettingsStore.setOverlayAdjustable(service, previous)
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
        val params = WindowManager.LayoutParams(
            dp(CaptionAdjustmentSwitch.TARGET_DP),
            dp(CaptionAdjustmentSwitch.TARGET_DP),
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT,
        ).apply {
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
            expandedView?.applySurfaceChrome(CaptionFrameStyle.saved(service, sessionOriginalOnly))
        }
        syncingAdjustment = true
        adjustmentSwitch?.isChecked = enabled
        syncingAdjustment = false
        expandedView?.setAdjustable(enabled)
        overlayParams?.let { params ->
            val flags = if (immersiveSession || !enabled) {
                params.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
            } else {
                params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
            }
            val alpha = 0.8f
            if (params.flags != flags || params.alpha != alpha) {
                params.flags = flags
                params.alpha = alpha
                overlayView?.let { windowManager?.updateViewLayout(it, params) }
            }
        }
        updateOverlayGeometry()
    }

    fun applyAppearance() {
        val root = overlayView ?: return
        if (overlayPlacement.setImmersive(SettingsStore.immersiveSubtitles(service))) {
            draggingOverlay = false
            resizeWidthPercent = null
            resizeHeightPercent = null
            overlayGeometryGeneration++
            val params = checkNotNull(overlayParams)
            params.flags = if (immersiveSession || !adjustable) {
                params.flags or WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE
            } else {
                params.flags and WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE.inv()
            }
            params.alpha = 0.8f
            // Update child presentation before WindowManager measures the new
            // compact frame or positions its companion restore control.
            compactView?.visibility = if (expanded) View.GONE else View.VISIBLE
            expandedView?.visibility = if (expanded) View.VISIBLE else View.GONE
            if (immersiveSession) {
                adjustmentSwitch?.let { windowManager?.removeView(it) }
                adjustmentSwitch = null
                adjustmentParams = null
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
        translationView?.setTextColor(SettingsStore.translationColor(service))
        expandedTranslationView?.setTextColor(SettingsStore.translationColor(service))
        overlayPlacement.setPreferredOffsetDp(SettingsStore.overlayYOffset(service))
        overlayPlacement.setExpandedOffsetDp(SettingsStore.expandedYOffset(service))
        updateOverlayGeometry()
        expandedView?.requestLayout()
        applyAdjustment(SettingsStore.overlayAdjustable(service))
        render()
    }

    /** Initial paint and live updates share the same renderer as the preview. */
    private fun applyOverlaySurface(compact: View, panel: View) {
        val style = CaptionFrameStyle.saved(service, sessionOriginalOnly)
        CaptionSurface.apply(compact, style, immersiveSession)
        CaptionSurface.apply(panel, style, false)
    }

    private fun updateOverlayFontSize() {
        val style = CaptionFrameStyle.saved(service, sessionOriginalOnly)
        compactView?.applyStyle(style, immersiveSession)
        expandedView?.applyStyle(style)
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
        else service.resources.displayMetrics.widthPixels to service.resources.displayMetrics.heightPixels
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
            expanded -> overlayPlacement.expandedY(service.resources.displayMetrics.density, height, desiredHeight)
            else -> overlayPlacement.compactY(service.resources.displayMetrics.density, height, captionHeight)
        }
        val desiredY = OverlayPlacement.clampY(
            preferredY,
            height,
            if (expanded) desiredHeight else captionHeight,
        )
        val desiredX = OverlayPlacement.clampX(
            if (draggingOverlay) params.x else (SettingsStore.overlayXOffset(service, expanded) * service.resources.displayMetrics.density).toInt(),
            width,
            desiredWidth,
        )
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
            val x = OverlayPlacement.adjustmentX(
                width,
                desiredWidth,
                params.x,
                controlWidth,
                dp(CaptionAdjustmentSwitch.RIGHT_INSET_DP),
            )
            val y = OverlayPlacement.adjustmentY(height, params.y, desiredHeight, controlHeight, 0)
            if (controlParams.x != x || controlParams.y != y) {
                controlParams.x = x
                controlParams.y = y
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
        overlayAvailableSize().first,
        resizeWidthPercent ?: SettingsStore.overlayWidthPercent(service),
        dp(if (expanded) 176 else 160),
        dp(8),
    )

    private fun expandedPanelHeight(): Int {
        val availableHeight = overlayAvailableSize().second
        val panel = expandedView
        val chrome = panel?.chromeHeight() ?: dp(CaptionPanelView.CHROME_HEIGHT_DP)
        val reading = panel?.minimumReadingHeight() ?: dp(64)
        return captionPanelHeight(
            availableHeight,
            resizeHeightPercent ?: SettingsStore.overlayHeightPercent(service),
            chrome,
            reading,
        )
    }

    fun hide() {
        adjustmentSwitch?.let { windowManager?.removeView(it) }
        adjustmentSwitch = null
        adjustmentParams = null
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
        previewMode = false
        previewHistoryEnabled = false
        previewOverOwnUi = false
        sessionOriginalOnly = false
        statusView = null
        sourceView = null
        translationView = null
        expandedSourceView = null
        expandedTranslationView = null
    }

    fun render() {
        val maxHistory = if (previewMode) (if (previewHistoryEnabled) 3 else 0)
        else SettingsStore.historyLines(service)
        val mode = if (sessionOriginalOnly) CaptionDisplayMode.ORIGINAL else SettingsStore.displayMode(service)
        val content = CaptionFrameContent.live(
            service,
            mode,
            maxHistory,
            currentObservation()?.state,
            isRunning(),
            isStopping(),
        )
        val statusLine = content.status
        compactView?.let {
            it.showContent(content, mode)
            it.source.setTextColor(if (mode == CaptionDisplayMode.ORIGINAL) SettingsStore.translationColor(service) else 0xFFE7E7E7.toInt())
        }
        expandedView?.let {
            it.showContent(content, mode, expanded)
            val lightSurface = CaptionColorMath.isLightSurface(
                SettingsStore.overlayBgColor(service),
                SettingsStore.overlayBgAlpha(service),
            )
            it.source.setTextColor(when {
                mode == CaptionDisplayMode.ORIGINAL -> SettingsStore.translationColor(service)
                lightSurface -> 0xFF3A3A3C.toInt()
                else -> 0xFFD5D5D5.toInt()
            })
        }
        updateOverlayGeometry()
        val hasCaption = listOf(sourceView, translationView).any {
            it?.visibility == View.VISIBLE && !it.text.isNullOrBlank()
        }
        val coveredByOwnUi = MimiService.ownUiVisible && !previewOverOwnUi
        overlayView?.visibility = if (!coveredByOwnUi && (expanded || hasCaption || statusLine.isNotBlank())) {
            View.VISIBLE
        } else {
            View.GONE
        }
        immersiveExitView?.visibility = if (coveredByOwnUi) View.GONE else View.VISIBLE
        adjustmentSwitch?.visibility = if (coveredByOwnUi) View.GONE else View.VISIBLE
    }

    private fun interfaceString(@androidx.annotation.StringRes id: Int): String =
        InterfaceLanguage.context(service).getString(id)

    private fun dp(value: Int): Int =
        (value * service.resources.displayMetrics.density).toInt()
}
