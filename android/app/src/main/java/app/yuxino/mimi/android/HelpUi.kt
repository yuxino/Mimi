package app.yuxino.mimi.android

import android.view.PointerIcon
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
        button.apply {
            setImageResource(R.drawable.ic_help)
            val background = android.util.TypedValue()
            activity.theme.resolveAttribute(android.R.attr.selectableItemBackgroundBorderless, background, true)
            setBackgroundResource(background.resourceId)
            imageTintList = ContextCompat.getColorStateList(activity, R.color.mimi_muted)
            val padding = ServiceSettingsUi.dp(activity, 12)
            setPadding(padding, padding, padding, padding)
            contentDescription = activity.getString(R.string.help_description, title)
            tooltipText = message
            isFocusable = true
            pointerIcon = PointerIcon.getSystemIcon(activity, PointerIcon.TYPE_HAND)
            setOnClickListener { ServiceSettingsUi.showHelp(activity, title, message) }
        }
    }
}

