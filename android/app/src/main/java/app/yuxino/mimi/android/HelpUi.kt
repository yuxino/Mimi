package app.yuxino.mimi.android

import android.graphics.Rect
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.MotionEvent
import android.view.ViewGroup
import android.view.ViewConfiguration
import android.widget.PopupWindow
import android.widget.ScrollView
import android.view.PointerIcon
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import android.widget.ImageButton
import android.view.View
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.constraintlayout.widget.ConstraintLayout

/** Hover/long press explains; tap opens readable, scrollable details on touch devices. */
internal object HelpUi {
    fun heading(activity: AppCompatActivity, title: String, message: String, size: Float, tag: String): View {
        val row = ConstraintLayout(activity)
        val label = ServiceSettingsUi.label(activity, title, size).apply { id = View.generateViewId() }
        val help = ImageButton(activity).apply {
            id = View.generateViewId(); this.tag = tag
            bind(activity, this, title, message)
        }
        row.addView(label, ConstraintLayout.LayoutParams(-2, -2).apply {
            startToStart = 0; endToStart = help.id; topToTop = 0; bottomToBottom = 0
            constrainedWidth = true; horizontalChainStyle = ConstraintLayout.LayoutParams.CHAIN_PACKED; horizontalBias = 0f
        })
        row.addView(help, ConstraintLayout.LayoutParams(ServiceSettingsUi.dp(activity, 48), ServiceSettingsUi.dp(activity, 48)).apply {
            startToEnd = label.id; endToEnd = 0; topToTop = 0; bottomToBottom = 0
        })
        return row
    }
    fun bind(activity: AppCompatActivity, button: ImageButton, title: String, message: String) {
        (button.getTag(R.id.help_hover) as? HoverHelp)?.dispose()
        val hover = HoverHelp(activity, button, message)
        button.setTag(R.id.help_hover, hover)
        button.apply {
            setImageResource(R.drawable.ic_help)
            val background = android.util.TypedValue()
            activity.theme.resolveAttribute(android.R.attr.selectableItemBackgroundBorderless, background, true)
            setBackgroundResource(background.resourceId)
            imageTintList = ContextCompat.getColorStateList(activity, R.color.mimi_muted)
            val padding = ServiceSettingsUi.dp(activity, 12)
            setPadding(padding, padding, padding, padding)
            contentDescription = activity.getString(R.string.help_description, title)
            tooltipText = null // The framework otherwise shows a second, truncated tooltip.
            isFocusable = true
            pointerIcon = PointerIcon.getSystemIcon(activity, PointerIcon.TYPE_HAND)
            setOnHoverListener { _, event ->
                if (!isEnabled) false else {
                    when (event.actionMasked) {
                        MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> { isHovered = true; hover.scheduleShow() }
                        MotionEvent.ACTION_HOVER_EXIT -> { isHovered = false; hover.scheduleHide() }
                    }
                    true
                }
            }
            onFocusChangeListener = View.OnFocusChangeListener { _, focused -> if (focused) hover.scheduleShow() else hover.dismiss() }
            setOnLongClickListener { performClick(); true }
            setOnClickListener { ServiceSettingsUi.showHelp(activity, title, hover.message) }
        }
    }
    fun setMessage(button: ImageButton, message: String) {
        (button.getTag(R.id.help_hover) as? HoverHelp)?.apply { dismiss(); this.message = message }
    }
    fun dismiss(activity: AppCompatActivity) {
        fun visit(view: View) {
            (view.getTag(R.id.help_hover) as? HoverHelp)?.dismiss()
            if (view is ViewGroup) for (index in 0 until view.childCount) visit(view.getChildAt(index))
        }
        visit(activity.window.decorView)
    }

    /** Native tooltips truncate at three lines. This bounded scrollable popup preserves the full explanation. */
    private class HoverHelp(private val activity: AppCompatActivity, private val button: ImageButton, var message: String) : View.OnAttachStateChangeListener {
        private var popup: PopupWindow? = null
        private var pending = false
        private val show = Runnable { pending = false; show() }
        private val hide = Runnable { dismiss() }
        private val lifecycle = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) dismiss()
            if (event == Lifecycle.Event.ON_DESTROY) dispose()
        }
        init { button.addOnAttachStateChangeListener(this); activity.lifecycle.addObserver(lifecycle) }
        fun scheduleShow() {
            button.removeCallbacks(hide)
            if (popup == null && !pending) {
                pending = true; button.postDelayed(show, ViewConfiguration.getLongPressTimeout().toLong())
            }
        }
        fun scheduleHide() {
            pending = false; button.removeCallbacks(show)
            button.postDelayed(hide, 150)
        }
        fun dismiss() {
            pending = false; button.removeCallbacks(show); button.removeCallbacks(hide)
            popup?.dismiss(); popup = null
        }
        fun dispose() { dismiss(); button.removeOnAttachStateChangeListener(this); activity.lifecycle.removeObserver(lifecycle) }
        override fun onViewAttachedToWindow(view: View) = Unit
        override fun onViewDetachedFromWindow(view: View) = dispose()
        private fun show() {
            if (!button.isShown || !button.isEnabled || activity.isFinishing || activity.isDestroyed) return
            HelpUi.dismiss(activity)
            val frame = Rect().also(button::getWindowVisibleDisplayFrame)
            val point = IntArray(2).also(button::getLocationOnScreen)
            val gap = ServiceSettingsUi.dp(activity, 8)
            val inset = ServiceSettingsUi.dp(activity, 16)
            val width = minOf(ServiceSettingsUi.dp(activity, 320), frame.width() - 2 * inset)
            val below = frame.bottom - point[1] - button.height - gap
            val above = point[1] - frame.top - gap
            val useBelow = below >= above
            val maxHeight = maxOf(below, above)
            if (width <= 0 || maxHeight < ServiceSettingsUi.dp(activity, 48)) return
            val text = ServiceSettingsUi.label(activity, message, 16f).apply {
                tag = "help-tooltip-message"
                setPadding(inset, gap, inset, gap)
            }
            val scroll = ScrollView(activity).apply {
                addView(text)
                setOnHoverListener { _, event ->
                    if (event.actionMasked == MotionEvent.ACTION_HOVER_EXIT) scheduleHide()
                    else button.removeCallbacks(hide)
                    false
                }
            }
            scroll.measure(View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(maxHeight, View.MeasureSpec.AT_MOST))
            val height = scroll.measuredHeight
            val surface = GradientDrawable().apply {
                cornerRadius = gap.toFloat()
                setColor(ContextCompat.getColor(activity, R.color.mimi_surface))
                setStroke(ServiceSettingsUi.dp(activity, 1), ContextCompat.getColor(activity, R.color.mimi_border))
            }
            val next = PopupWindow(scroll, width, height, false).apply {
                setBackgroundDrawable(surface); isOutsideTouchable = true; setIsLaidOutInScreen(true)
                setOnDismissListener { if (popup === this) popup = null }
            }
            val x = (point[0] + button.width / 2 - width / 2).coerceIn(frame.left + inset, frame.right - inset - width)
            val y = if (useBelow) point[1] + button.height + gap else point[1] - gap - height
            popup = next
            next.showAtLocation(button, Gravity.TOP or Gravity.LEFT, x, y)
        }
    }
}
