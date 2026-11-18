package app.yuxino.mimi.android.captions

import android.graphics.Color
import android.graphics.Typeface
import android.view.Gravity
import android.widget.TextView

/** Keep the native preview and floating caption typography in agreement. */
internal object CaptionTypography {
    fun apply(source: TextView, translation: TextView, fontSize: Int, immersive: Boolean, originalOnly: Boolean = false) {
        val size = fontSize.coerceIn(12, 24)
        // The chosen size is the reading target, including Chinese. The
        // bilingual original is a quieter reference, as on desktop.
        source.textSize = (if (originalOnly) size.toFloat() else size * 0.9f).coerceAtLeast(12f)
        translation.textSize = size.toFloat()
        translation.typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        for (view in listOf(source, translation)) {
            val density = view.resources.displayMetrics.density
            view.setLineSpacing(2f * density, 1f)
            applyContrast(view, immersive)
        }
        (translation.layoutParams as? android.view.ViewGroup.MarginLayoutParams)?.let { params ->
            val gap = ((if (immersive) 4 else 6) * translation.resources.displayMetrics.density).toInt()
            if (params.topMargin != gap) translation.layoutParams = params.apply { topMargin = gap }
        }
    }

    /** Reserve fallback-font space without changing window height on every word. */
    fun minimumLineHeight(view: TextView): Int {
        val sample = "Ag Éç 国語 한글 กำ ฐุ العربية हिन्दी"
        val layout = android.text.StaticLayout.Builder.obtain(sample, 0, sample.length, view.paint, 100000)
            .setIncludePad(view.includeFontPadding)
            .setUseLineSpacingFromFallbacks(view.isFallbackLineSpacing)
            .setLineSpacing(view.lineSpacingExtra, view.lineSpacingMultiplier)
            .build()
        return layout.getLineBottom(0) - layout.getLineTop(0) + view.compoundPaddingTop + view.compoundPaddingBottom
    }

    fun applyContrast(view: TextView, immersive: Boolean) {
        view.gravity = if (immersive) Gravity.CENTER_HORIZONTAL else Gravity.START
        val density = view.resources.displayMetrics.density
        view.setShadowLayer(if (immersive) 1.25f * density else 0f, 0f, if (immersive) 0.5f * density else 0f, Color.BLACK)
    }
}
