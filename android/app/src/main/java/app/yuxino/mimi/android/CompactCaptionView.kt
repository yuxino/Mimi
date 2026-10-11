package app.yuxino.mimi.android

import android.content.Context
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView
import app.yuxino.mimi.android.capture.LatestCaptionTextView

/** Actual compact immersion text, also rendered unchanged inside the preview stage. */
internal class CompactCaptionView(context: Context) : LinearLayout(context) {
    val status = TextView(context).apply { tag = "overlay-status"; visibility = View.GONE }
    val source = LatestCaptionTextView(context).apply { maxLines = 2 }
    val translation = LatestCaptionTextView(context).apply { maxLines = 3 }

    init {
        tag = "compact-subtitle"
        orientation = VERTICAL; gravity = Gravity.CENTER_VERTICAL
        setPadding(dp(3), dp(9), dp(3), dp(10))
        background = GradientDrawable().apply { cornerRadius = dp(12).toFloat() }
        addView(status); addView(source); addView(translation)
    }

    fun applyStyle(style: CaptionFrameStyle, immersive: Boolean) {
        CaptionTypography.apply(source, translation, style.fontSize, immersive, style.mode == CaptionDisplayMode.ORIGINAL)
        CaptionTypography.applyContrast(status, immersive)
        status.textSize = style.fontSize.toFloat(); status.setTextColor(style.color)
        source.setTextColor(if (style.mode == CaptionDisplayMode.ORIGINAL) style.color else 0xFFE7E7E7.toInt())
        translation.setTextColor(style.color)
        listOf(status, source, translation).forEach { it.gravity = style.alignment.gravity }
        CaptionSurface.apply(this, style, immersive)
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
}
