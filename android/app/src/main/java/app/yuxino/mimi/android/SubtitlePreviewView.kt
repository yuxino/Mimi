package app.yuxino.mimi.android

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.util.AttributeSet
import android.view.Gravity
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.content.ContextCompat

/** Clearly marked sample content; never starts capture or opens a provider session. */
class SubtitlePreviewView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : LinearLayout(context, attrs) {
    private val exampleLabel = TextView(context)
    private val source = TextView(context)
    private val translation = TextView(context)
    private val subtitle = LinearLayout(context)

    init {
        orientation = VERTICAL
        minimumHeight = dp(152)
        setPadding(dp(18), dp(16), dp(18), dp(18))
        background = GradientDrawable().apply {
            setColor(ContextCompat.getColor(context, R.color.mimi_preview_bg))
            cornerRadius = dp(14).toFloat()
        }
        val labelRow = LinearLayout(context).apply {
            orientation = HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        labelRow.addView(TextView(context).apply {
            setText(R.string.preview_label)
            setTextColor(Color.rgb(184, 184, 184))
            textSize = 11f
        }, LayoutParams(0, LayoutParams.WRAP_CONTENT, 1f))
        labelRow.addView(exampleLabel.apply {
            setText(R.string.preview_example)
            setTextColor(Color.rgb(145, 145, 145))
            textSize = 11f
        })
        addView(labelRow)
        subtitle.orientation = VERTICAL
        subtitle.setPadding(dp(12), dp(9), dp(12), dp(10))
        source.setTextColor(Color.rgb(203, 203, 203))
        source.textSize = 14f
        source.setLineSpacing(dp(3).toFloat(), 1f)
        translation.setTextColor(Color.WHITE)
        translation.textSize = 19f
        translation.typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        translation.setLineSpacing(dp(4).toFloat(), 1f)
        subtitle.addView(source, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
        subtitle.addView(translation, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT).apply {
            topMargin = dp(8)
        })
        addView(subtitle, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT).apply {
            topMargin = dp(14)
        })
        configure(16, Color.WHITE, 100, 65, "zh", false)
    }

    fun showAppearanceShortcut() {
        exampleLabel.setText(R.string.preview_edit)
    }

    fun configure(fontSize: Int, color: Int, opacity: Int, backgroundAlpha: Int, targetLang: String, immersive: Boolean, originalOnly: Boolean = false) {
        translation.visibility = if (originalOnly) GONE else VISIBLE
        source.setText(when (targetLang) {
            "en" -> R.string.preview_zh
            "th" -> R.string.preview_en
            else -> R.string.preview_source
        })
        translation.setText(when (targetLang) {
            "en" -> R.string.preview_en
            "ja" -> R.string.preview_ja
            "th" -> R.string.preview_th
            else -> R.string.preview_zh
        })
        source.textSize = fontSize.coerceIn(12, 24).toFloat()
        translation.textSize = fontSize.coerceIn(12, 24) + 3f
        translation.setTextColor(color)
        subtitle.alpha = if (immersive) 0.8f else opacity.coerceIn(20, 100) / 100f
        source.setShadowLayer(if (immersive) dp(4).toFloat() else 0f, 0f, dp(1).toFloat(), Color.BLACK)
        translation.setShadowLayer(if (immersive) dp(4).toFloat() else 0f, 0f, dp(1).toFloat(), Color.BLACK)
        subtitle.background = GradientDrawable().apply {
            setColor(Color.argb(if (immersive) 0 else backgroundAlpha.coerceIn(0, 90) * 255 / 100, 16, 16, 16))
            cornerRadius = dp(12).toFloat()
        }
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()
}
