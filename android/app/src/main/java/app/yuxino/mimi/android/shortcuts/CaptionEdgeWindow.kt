package app.yuxino.mimi.android.shortcuts

import android.annotation.SuppressLint
import android.animation.ValueAnimator
import android.content.Context
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.graphics.PixelFormat
import android.graphics.Rect
import android.os.Build
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowInsets
import android.view.WindowManager
import android.view.accessibility.AccessibilityManager
import android.widget.FrameLayout
import android.widget.ImageButton
import androidx.core.animation.doOnEnd
import androidx.core.view.ViewCompat
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat.AccessibilityActionCompat
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.localization.InterfaceLanguage
import kotlin.math.roundToInt

internal class CaptionEdgeWindow(
    private val context: Context, initial: CaptionEdgePlacement, toggle: () -> Unit,
    private val savePosition: (CaptionEdgePlacement) -> Unit,
) {
    var placement = initial
        private set
    private val manager = context.getSystemService(WindowManager::class.java)
    private val button = object : FrameLayout(context) {
        override fun getAccessibilityClassName(): CharSequence = ImageButton::class.java.name
        override fun onConfigurationChanged(newConfig: Configuration) {
            super.onConfigurationChanged(newConfig)
            post(updateResources)
        }
    }
    private var artwork = CaptionEdgeArtwork(context)
    // WindowManager and touch dispatch keep these two roots even if fonts or artwork need reloading.
    private val command = CaptionEdgeCommand(InterfaceLanguage.context(context))
    private var side = initial.nearestSide()
    private val accessibility = context.getSystemService(AccessibilityManager::class.java)
    private var touchSize = dp(48)
    private var slop = ViewConfiguration.get(context).scaledTouchSlop
    private val renderedConfig = Configuration(InterfaceLanguage.context(context).resources.configuration)
    @SuppressLint("RtlHardcoded") // Offsets are physical pixels from the left of WindowManager's fitted frame.
    private val params = WindowManager.LayoutParams(touchSize, touchSize,
        WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
        PixelFormat.TRANSLUCENT).apply {
        gravity = Gravity.LEFT or Gravity.TOP
        title = InterfaceLanguage.context(context).getString(R.string.shortcut_edge_label)
        if (Build.VERSION.SDK_INT >= 30) {
            setFitInsetsTypes(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            setFitInsetsIgnoringVisibility(true)
        }
    }
    private val commandParams = WindowManager.LayoutParams().apply { copyFrom(params) }
    private var travelX = 0
    private var travelY = 0
    private var added = false
    private var gesture: CaptionEdgeGesture? = null
    private var tucked: CaptionEdge? = null
    private var settling: ValueAnimator? = null
    private var currentLabel = 0
    private var moveActionId = View.NO_ID
    private val updateFrame = Runnable { if (added) measureFrame() }
    private val updateResources = Runnable { if (added) refreshConfiguration() }
    private val tuck = Runnable { dock(placement.nearestSide(), animate = true) }
    private val explorationChanged = AccessibilityManager.TouchExplorationStateChangeListener {
        if (it) { cancelSettle(); present(null) }
        scheduleTuck()
    }

    init {
        button.addView(artwork)
        artwork.face(side)
        command.setOnClickListener {
            if (command.isEnabled && command.isShown) {
                toggle()
                scheduleTuck()
            }
        }
        command.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> button.removeCallbacks(tuck)
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> scheduleTuck()
            }
            false
        }
        button.isFocusable = true
        button.filterTouchesWhenObscured = true
        // An inward drag on a docked button shares the system Back gesture's starting edge.
        button.systemGestureExclusionRects = listOf(Rect(0, 0, touchSize, touchSize))
        button.setOnClickListener {
            present(null)
            scheduleTuck()
        }
        // The portrait and label share one screen-reader target so focus stays on it
        // when the label opens. A physical tap on the portrait only reveals the label.
        ViewCompat.replaceAccessibilityAction(button, AccessibilityActionCompat.ACTION_CLICK, null) { _, _ ->
            if (!button.isEnabled) false else {
                if (tucked == null) toggle() else button.performClick()
                scheduleTuck()
                true
            }
        }
        @SuppressLint("ClickableViewAccessibility") // Completed taps use the portrait's ordinary performClick.
        button.setOnTouchListener { _, event -> touch(event) }
        button.setOnApplyWindowInsetsListener { _, insets ->
            button.post(updateFrame)
            insets
        }
        setMoveActionLabel(InterfaceLanguage.context(context))
    }

    private fun setMoveActionLabel(localized: Context) {
        if (moveActionId != View.NO_ID) ViewCompat.removeAccessibilityAction(button, moveActionId)
        moveActionId = ViewCompat.addAccessibilityAction(button, localized.getString(R.string.caption_edge_move)) { _, _ ->
            val edge = when (placement.nearestSide()) {
                CaptionEdge.LEFT -> CaptionEdge.RIGHT
                CaptionEdge.RIGHT -> CaptionEdge.LEFT
            }
            button.removeCallbacks(tuck)
            dock(edge)
            savePosition(placement)
            true
        }
    }

    /** A configuration dispatch can accompany returning from consent with both touches still in flight. */
    fun refreshConfiguration() {
        val localized = InterfaceLanguage.context(context)
        val config = localized.resources.configuration
        val changes = renderedConfig.diff(config)
        if (changes and VISUAL_CONFIGURATION != 0) {
            touchSize = dp(48)
            slop = ViewConfiguration.get(context).scaledTouchSlop
            button.removeView(artwork)
            artwork = CaptionEdgeArtwork(localized)
            button.addView(artwork)
            artwork.face(side)
            command.reloadAppearance(localized)
            button.systemGestureExclusionRects = listOf(Rect(0, 0, touchSize, touchSize))
            setMoveActionLabel(localized)
            params.title = localized.getString(R.string.shortcut_edge_label)
            params.width = touchSize
            params.height = touchSize
            commandParams.title = params.title
            commandParams.height = touchSize
            manager.updateViewLayout(button, params)
            manager.updateViewLayout(command, commandParams)
            refreshAction(reloadLabels = true)
        }
        renderedConfig.setTo(config)
        measureFrame()
    }

    fun show() {
        placement = placement.dock(side)
        tucked = side
        refreshAction()
        measureFrame()
        manager.addView(button, params)
        try {
            manager.addView(command, commandParams)
        } catch (failure: RuntimeException) {
            manager.removeViewImmediate(button)
            throw failure
        }
        added = true
        present(if (accessibility.isTouchExplorationEnabled) null else side)
        accessibility.addTouchExplorationStateChangeListener(explorationChanged)
        scheduleTuck()
    }

    fun close() {
        added = false
        cancelSettle()
        accessibility.removeTouchExplorationStateChangeListener(explorationChanged)
        button.removeCallbacks(tuck)
        button.removeCallbacks(updateFrame)
        button.removeCallbacks(updateResources)
        gesture = null
        for (view in listOf(command, button)) {
            try { manager.removeViewImmediate(view) } catch (_: IllegalArgumentException) {
                // A removed display may have already released either window.
            }
        }
    }

    fun refreshAction(reloadLabels: Boolean = false) {
        val state = CaptionShortcutActions.state()
        val label = when (state) {
            CaptionShortcutState.INACTIVE -> R.string.start_capture
            CaptionShortcutState.ACTIVE -> R.string.stop_capture
            CaptionShortcutState.STARTING -> R.string.home_starting_status
            CaptionShortcutState.STOPPING -> R.string.stopping_capture
        }
        if (label == currentLabel && !reloadLabels) return
        if (label != currentLabel && gesture != null) endPress()
        currentLabel = label
        button.isEnabled = state == CaptionShortcutState.INACTIVE || state == CaptionShortcutState.ACTIVE
        describeAction()
        val localized = InterfaceLanguage.context(context)
        val visibleLabel = when (state) {
            CaptionShortcutState.INACTIVE -> R.string.start_action
            CaptionShortcutState.ACTIVE -> R.string.stop_action
            else -> label
        }
        command.setLabel(localized.getString(visibleLabel))
        command.isEnabled = button.isEnabled
        updateCommand()
        val status = when (state) {
            CaptionShortcutState.INACTIVE -> R.string.shortcut_captions_off
            CaptionShortcutState.ACTIVE -> R.string.notification_title
            else -> label
        }
        ViewCompat.setStateDescription(button, localized.getString(status))
    }

    private fun describeAction() {
        if (currentLabel != 0) button.contentDescription = InterfaceLanguage.context(context)
            .getString(if (tucked == null) currentLabel else R.string.shortcut_reveal_edge)
    }

    private fun dp(value: Int) = (value * context.resources.displayMetrics.density).roundToInt()

    /** LayoutParams x/y are already relative to the fitted frame. Insets only reduce travel. */
    private fun measureFrame() {
        val safe = if (Build.VERSION.SDK_INT >= 30) {
            val metrics = manager.currentWindowMetrics
            val insets = metrics.windowInsets.getInsetsIgnoringVisibility(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            val bounds = metrics.bounds
            Rect(bounds.left + insets.left, bounds.top + insets.top, bounds.right - insets.right, bounds.bottom - insets.bottom)
        } else {
            Rect().apply {
                if (button.isAttachedToWindow) button.getWindowVisibleDisplayFrame(this)
                if (isEmpty) set(0, 0, context.resources.displayMetrics.widthPixels, context.resources.displayMetrics.heightPixels)
            }
        }
        val x = (safe.width() - touchSize).coerceAtLeast(0)
        val y = (safe.height() - touchSize).coerceAtLeast(0)
        if (x == travelX && y == travelY) return
        if (gesture != null) endPress()
        travelX = x
        travelY = y
        moveWindow()
    }

    private fun moveWindow() {
        val x = placement.left(travelX)
        val y = placement.top(travelY)
        if (params.x != x || params.y != y) {
            params.x = x
            params.y = y
            if (added) manager.updateViewLayout(button, params)
        }
        updateCommand()
    }

    private fun present(edge: CaptionEdge?) {
        tucked = edge
        describeAction()
        updateCommand()
    }

    private fun updateCommand() {
        if (!added) return
        if (tucked != null || gesture?.dragging == true) {
            command.visibility = View.GONE
            return
        }
        // The bubble starts just beyond the visible 20 dp portrait. Its center and the
        // portrait's center lie in separate touch areas even on the compact docked edge.
        command.alignToPortrait(side)
        val rightSide = side == CaptionEdge.RIGHT
        val insetFromEdge = dp(26)
        val end = params.x + touchSize - insetFromEdge
        val start = params.x + insetFromEdge
        val available = if (rightSide) end else travelX + touchSize - start
        val width = command.measuredWidthWithin(available)
        val x = if (rightSide) end - width else start
        if (commandParams.x != x || commandParams.y != params.y || commandParams.width != width) {
            commandParams.x = x
            commandParams.y = params.y
            commandParams.width = width
            manager.updateViewLayout(command, commandParams)
        }
        command.visibility = View.VISIBLE
    }

    private fun face(edge: CaptionEdge) {
        side = edge
        artwork.face(edge)
    }

    private fun cancelSettle() {
        val motion = settling
        settling = null
        motion?.cancel()
    }

    private fun dock(edge: CaptionEdge, animate: Boolean = false) {
        cancelSettle()
        if (gesture == null) face(edge)
        val from = placement
        val target = from.dock(edge)
        if (!animate || from == target || !ValueAnimator.areAnimatorsEnabled()) {
            placement = target
            moveWindow()
            present(if (accessibility.isTouchExplorationEnabled) null else edge)
            return
        }
        val motion = ValueAnimator.ofFloat(0f, 1f)
        settling = motion
        motion.duration = 160
        motion.interpolator = android.view.animation.DecelerateInterpolator()
        motion.addUpdateListener {
            val fraction = it.animatedValue as Float
            placement = CaptionEdgePlacement(from.x + (target.x - from.x) * fraction,
                from.y + (target.y - from.y) * fraction)
            moveWindow()
        }
        motion.doOnEnd {
            if (settling === motion) {
                settling = null
                present(edge)
            }
        }
        motion.start()
    }

    private fun scheduleTuck() {
        button.removeCallbacks(tuck)
        if (added && gesture == null && !accessibility.isTouchExplorationEnabled) button.postDelayed(tuck, 3_000)
    }

    private fun endPress() {
        val dragged = gesture?.dragging == true
        gesture = null
        button.isPressed = false
        if (dragged) {
            face(placement.nearestSide())
            updateCommand()
        }
        scheduleTuck()
    }

    private fun touch(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                button.removeCallbacks(tuck)
                cancelSettle()
                gesture = CaptionEdgeGesture(event.rawX, event.rawY, slop, params.x, params.y)
                button.isPressed = true
                present(null)
            }
            MotionEvent.ACTION_MOVE, MotionEvent.ACTION_UP -> {
                val press = gesture
                if (press == null) {
                    if (event.actionMasked == MotionEvent.ACTION_UP) scheduleTuck()
                    return true
                }
                if (press.update(event.rawX, event.rawY)) {
                    button.isPressed = false
                    if (command.visibility == View.VISIBLE) command.visibility = View.GONE
                    placement = placement.moved(press.left(event.rawX), press.top(event.rawY), travelX, travelY)
                    moveWindow()
                }
                if (event.actionMasked == MotionEvent.ACTION_UP) {
                    val inside = event.x >= 0 && event.y >= 0 && event.x < button.width && event.y < button.height
                    val tap = press.isTap(event.eventTime - event.downTime, ViewConfiguration.getLongPressTimeout(), inside)
                    if (press.dragging) savePosition(placement.dock(placement.nearestSide()))
                    endPress()
                    if (tap) button.performClick()
                }
            }
            MotionEvent.ACTION_CANCEL -> endPress()
            MotionEvent.ACTION_POINTER_DOWN -> {
                endPress()
                button.removeCallbacks(tuck)
            }
        }
        return true
    }

    private companion object {
        val VISUAL_CONFIGURATION = ActivityInfo.CONFIG_DENSITY or ActivityInfo.CONFIG_FONT_SCALE or
            ActivityInfo.CONFIG_LOCALE or ActivityInfo.CONFIG_LAYOUT_DIRECTION or
            (if (Build.VERSION.SDK_INT >= 31) ActivityInfo.CONFIG_FONT_WEIGHT_ADJUSTMENT else 0)
    }
}
