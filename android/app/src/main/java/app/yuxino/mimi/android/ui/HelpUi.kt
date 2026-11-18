package app.yuxino.mimi.android.ui

import android.graphics.Rect
import android.graphics.drawable.GradientDrawable
import android.text.method.LinkMovementMethod
import android.view.Gravity
import android.view.MotionEvent
import android.view.PointerIcon
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewTreeObserver
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.PopupWindow
import androidx.appcompat.app.AppCompatActivity
import androidx.constraintlayout.widget.ConstraintLayout
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.services.editor.ServiceSettingsUi
import app.yuxino.mimi.android.ui.layout.ControlScrollView
import com.google.android.material.button.MaterialButton

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
        val previous = button.getTag(R.id.help_hover) as? HoverHelp
        if (previous?.title == title && previous.message == message) return
        previous?.dispose()
        val hover = HoverHelp(activity, button, title, message)
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
            setOnClickListener { showDetails(activity, title, hover.message) }
        }
    }
    fun showDetails(activity: AppCompatActivity, title: String, message: CharSequence,
        links: List<Pair<Int, () -> Unit>> = emptyList()) {
        if (activity.isFinishing || activity.isDestroyed) return
        val host = activity.window.decorView
        if ((host.getTag(R.id.active_help_dialog) as? androidx.appcompat.app.AlertDialog)?.isShowing == true) return
        HelpUi.dismiss(activity)
        val body = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(
                ServiceSettingsUi.dp(activity, 24), ServiceSettingsUi.dp(activity, 12),
                ServiceSettingsUi.dp(activity, 24), ServiceSettingsUi.dp(activity, 8),
            )
        }
        body.addView(ServiceSettingsUi.label(activity, "", 16f).apply {
            tag = "help-message"; text = message; movementMethod = LinkMovementMethod.getInstance()
        }, LinearLayout.LayoutParams(-1, -2))
        links.forEach { (label, action) ->
            body.addView(MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
                setText(label); isAllCaps = false; isSingleLine = false
                setTextColor(ContextCompat.getColor(activity, R.color.mimi_text))
                setPadding(
                    paddingLeft, ServiceSettingsUi.dp(activity, 12),
                    paddingRight, ServiceSettingsUi.dp(activity, 12),
                )
                setOnClickListener { action() }
            }, LinearLayout.LayoutParams(-1, -2).apply { topMargin = ServiceSettingsUi.dp(activity, 8) })
        }
        val surface = GradientDrawable().apply { cornerRadius = ServiceSettingsUi.dp(activity, 28).toFloat() }
        val dialog = com.google.android.material.dialog.MaterialAlertDialogBuilder(activity)
            .setBackground(surface).setTitle(title)
            .setView(ControlScrollView(activity).apply { addView(body) })
            .setPositiveButton(android.R.string.ok, null).show()
        surface.setColor(ContextCompat.getColor(dialog.context, R.color.mimi_surface))
        host.setTag(R.id.active_help_dialog, dialog)
        val lifecycle = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP) dialog.dismiss() }
        activity.lifecycle.addObserver(lifecycle)
        dialog.setOnDismissListener {
            host.setTag(R.id.active_help_dialog, null)
            activity.lifecycle.removeObserver(lifecycle)
        }
    }
    fun setMessage(button: ImageButton, message: String) {
        (button.getTag(R.id.help_hover) as? HoverHelp)?.apply { dismiss(); this.message = message }
    }
    fun dismiss(activity: AppCompatActivity) {
        (activity.window.decorView.getTag(R.id.active_help_hover) as? HoverHelp)?.dismiss()
    }

    /** Native tooltips truncate at three lines. This bounded scrollable popup preserves the full explanation. */
    private class HoverHelp(private val activity: AppCompatActivity, private val button: ImageButton, val title: String, var message: String) : View.OnAttachStateChangeListener {
        private var popup: PopupWindow? = null
        private var pending = false
        private val show = Runnable { pending = false; show() }
        private val hide = Runnable { dismiss() }
        private val windowFocus = ViewTreeObserver.OnWindowFocusChangeListener { focused ->
            if (!focused) dismiss()
        }
        private val lifecycle = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) dismiss()
            if (event == Lifecycle.Event.ON_DESTROY) dispose()
        }
        init {
            button.addOnAttachStateChangeListener(this)
            activity.lifecycle.addObserver(lifecycle)
            button.viewTreeObserver.addOnWindowFocusChangeListener(windowFocus)
        }
        fun scheduleShow() {
            if (!button.hasWindowFocus() || !button.isShown || !button.isEnabled) return
            button.removeCallbacks(hide)
            if (popup == null && !pending) {
                HelpUi.dismiss(activity)
                activity.window.decorView.setTag(R.id.active_help_hover, this)
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
            val host = activity.window.decorView
            if (host.getTag(R.id.active_help_hover) === this) host.setTag(R.id.active_help_hover, null)
        }
        fun dispose() {
            dismiss()
            button.removeOnAttachStateChangeListener(this)
            button.viewTreeObserver.takeIf { it.isAlive }?.removeOnWindowFocusChangeListener(windowFocus)
            activity.lifecycle.removeObserver(lifecycle)
        }
        override fun onViewAttachedToWindow(view: View) {
            button.viewTreeObserver.removeOnWindowFocusChangeListener(windowFocus)
            button.viewTreeObserver.addOnWindowFocusChangeListener(windowFocus)
        }
        override fun onViewDetachedFromWindow(view: View) = dispose()
        private fun show() {
            if (!button.hasWindowFocus() || !button.isShown || !button.isEnabled ||
                (!button.hasFocus() && !button.isHovered) || activity.isFinishing || activity.isDestroyed) {
                dismiss(); return
            }
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
            val scroll = ControlScrollView(activity).apply {
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
