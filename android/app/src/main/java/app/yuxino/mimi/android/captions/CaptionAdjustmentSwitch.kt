package app.yuxino.mimi.android.captions

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.InsetDrawable
import android.view.ContextThemeWrapper
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.localization.InterfaceLanguage
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

    fun applySurfaceContrast(lightSurface: Boolean) {
        val states = arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf())
        if (lightSurface) {
            trackTintList = ColorStateList(states, intArrayOf(0xFF1C1C1E.toInt(), 0x40000000))
            thumbTintList = ColorStateList(states, intArrayOf(Color.WHITE, 0xFF525252.toInt()))
        } else {
            trackTintList = ColorStateList(states, intArrayOf(0xFFF4F4F4.toInt(), 0x4DFFFFFF))
            thumbTintList = ColorStateList(states, intArrayOf(0xFF171717.toInt(), 0xFFB9B9B9.toInt()))
        }
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
