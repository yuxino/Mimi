package app.yuxino.mimi.android.shortcuts

import android.annotation.SuppressLint
import android.animation.ValueAnimator
import android.content.Context
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
    }
    private val artwork = CaptionEdgeArtwork(context)
    private val accessibility = context.getSystemService(AccessibilityManager::class.java)
    private val touchSize = dp(48)
    private val slop = ViewConfiguration.get(context).scaledTouchSlop
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
    private var travelX = 0
    private var travelY = 0
    private var added = false
    private var gesture: CaptionEdgeGesture? = null
    private var tucked: CaptionEdge? = null
    private var settling: ValueAnimator? = null
    private var currentLabel = 0
    private val updateFrame = Runnable { if (added) measureFrame() }
    private val tuck = Runnable { dock(placement.nearest(travelX, travelY), animate = true) }
    private val explorationChanged = AccessibilityManager.TouchExplorationStateChangeListener {
        if (it) { cancelSettle(); present(null) }
        scheduleTuck()
    }

    init {
        button.addView(artwork, FrameLayout.LayoutParams(touchSize, touchSize))
        button.isFocusable = true
        button.filterTouchesWhenObscured = true
        // An inward drag on a docked button shares the system Back gesture's starting edge.
        button.systemGestureExclusionRects = listOf(Rect(0, 0, touchSize, touchSize))
        button.setOnClickListener {
            if (tucked != null) present(null) else toggle()
            scheduleTuck()
        }
        @SuppressLint("ClickableViewAccessibility") // Real taps and TalkBack both use the view's ordinary performClick.
        button.setOnTouchListener { _, event -> touch(event) }
        button.setOnApplyWindowInsetsListener { _, insets ->
            button.post(updateFrame)
            insets
        }
        ViewCompat.addAccessibilityAction(button, InterfaceLanguage.context(context).getString(R.string.caption_edge_move)) { _, _ ->
            val edge = when (placement.nearest(travelX, travelY)) {
                CaptionEdge.LEFT -> CaptionEdge.RIGHT
                CaptionEdge.RIGHT -> CaptionEdge.LEFT
                CaptionEdge.TOP -> CaptionEdge.BOTTOM
                CaptionEdge.BOTTOM -> CaptionEdge.TOP
            }
            button.removeCallbacks(tuck)
            dock(edge)
            savePosition(placement)
            true
        }
    }

    fun show() {
        present(null)
        refreshAction()
        measureFrame()
        manager.addView(button, params)
        added = true
        accessibility.addTouchExplorationStateChangeListener(explorationChanged)
        scheduleTuck()
    }

    fun close() {
        added = false
        cancelSettle()
        accessibility.removeTouchExplorationStateChangeListener(explorationChanged)
        button.removeCallbacks(tuck)
        button.removeCallbacks(updateFrame)
        gesture = null
        try {
            manager.removeViewImmediate(button)
        } catch (_: IllegalArgumentException) {
            // WindowManager may already have removed this display's window.
        }
    }

    fun refreshAction() {
        val state = CaptionShortcutActions.state()
        val label = when (state) {
            CaptionShortcutState.INACTIVE -> R.string.start_capture
            CaptionShortcutState.ACTIVE -> R.string.stop_capture
            CaptionShortcutState.STARTING -> R.string.home_starting_status
            CaptionShortcutState.STOPPING -> R.string.stopping_capture
        }
        if (label == currentLabel) return
        currentLabel = label
        if (gesture != null) endPress()
        button.isEnabled = state == CaptionShortcutState.INACTIVE || state == CaptionShortcutState.ACTIVE
        describeAction()
        val stop = state == CaptionShortcutState.ACTIVE || state == CaptionShortcutState.STOPPING
        artwork.setStop(stop)
        val status = when (state) {
            CaptionShortcutState.INACTIVE -> R.string.shortcut_captions_off
            CaptionShortcutState.ACTIVE -> R.string.notification_title
            else -> label
        }
        ViewCompat.setStateDescription(button, InterfaceLanguage.context(context).getString(status))
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
        artwork.place(x, travelX - x, y, travelY - y)
        if (params.x == x && params.y == y) return
        params.x = x
        params.y = y
        if (added) manager.updateViewLayout(button, params)
    }

    private fun present(edge: CaptionEdge?) {
        tucked = edge
        artwork.showAction(edge == null)
        describeAction()
    }

    private fun cancelSettle() {
        val motion = settling
        settling = null
        motion?.cancel()
    }

    private fun dock(edge: CaptionEdge, animate: Boolean = false) {
        cancelSettle()
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
        if (dragged) artwork.showAction(tucked == null)
        scheduleTuck()
    }

    private fun touch(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                button.removeCallbacks(tuck)
                val revealing = tucked != null || settling != null
                cancelSettle()
                gesture = CaptionEdgeGesture(event.rawX, event.rawY, slop, params.x, params.y, startedTucked = revealing)
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
                    artwork.showAction(false)
                    placement = placement.moved(press.left(event.rawX), press.top(event.rawY), travelX, travelY)
                    moveWindow()
                }
                if (event.actionMasked == MotionEvent.ACTION_UP) {
                    val inside = event.x >= 0 && event.y >= 0 && event.x < button.width && event.y < button.height
                    val tap = press.activates(event.eventTime - event.downTime, ViewConfiguration.getLongPressTimeout(), inside)
                    if (press.dragging) savePosition(placement.dock(placement.nearest(travelX, travelY)))
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
}
