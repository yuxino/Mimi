package app.yuxino.mimi.android

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.InsetDrawable
import android.view.ContextThemeWrapper
import androidx.core.content.ContextCompat
import com.google.android.material.materialswitch.MaterialSwitch

/** One compact appearance for the native caption and its inert settings preview. */
internal class CaptionAdjustmentSwitch(context: Context, private val paintsControl: Boolean = true) :
    MaterialSwitch(ContextThemeWrapper(InterfaceLanguage.context(context), R.style.Theme_Mimi)) {
    init {
        minimumWidth = dp(TARGET_DP); minimumHeight = dp(TARGET_DP)
        setPadding(dp(8), 0, dp(8), 0)
        switchMinWidth = dp(32)
        background = null
        // The input-only window must also suppress the framework's focus paint,
        // which is drawn outside onDraw after keyboard input leaves touch mode.
        if (!paintsControl) defaultFocusHighlightEnabled = false
        thumbIconDrawable = null
        trackDecorationDrawable = null
        thumbDrawable = InsetDrawable(GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(Color.WHITE); setSize(dp(16), dp(16))
        }, 0, dp(2), 0, dp(2))
        trackDrawable = GradientDrawable().apply {
            cornerRadius = dp(10).toFloat()
            setColor(Color.GRAY); setSize(dp(32), dp(20))
        }
        trackTintList = ContextCompat.getColorStateList(this.context, R.color.mimi_switch_track)
        thumbTintList = ContextCompat.getColorStateList(this.context, R.color.mimi_switch_thumb)
        contentDescription = this.context.getString(R.string.overlay_adjust)
    }

    override fun onDraw(canvas: Canvas) {
        // The separate recovery window owns input/accessibility only. Painting
        // here as well would composite the thumb twice over the .8-alpha panel.
        if (paintsControl) super.onDraw(canvas)
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    companion object {
        const val TARGET_DP = 48
        const val RIGHT_INSET_DP = 4
    }
}
