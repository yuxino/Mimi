package app.yuxino.mimi.android.captions

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.view.View
import app.yuxino.mimi.android.settings.SettingsStore

internal data class CaptionFrameStyle(
    val fontSize: Int,
    val color: Int,
    val opacity: Int,
    val backgroundAlpha: Int,
    val mode: CaptionDisplayMode,
    val alignment: CaptionAlignment,
    val dividers: Boolean = false,
    val backgroundColor: Int = CaptionColorMath.DEFAULT_BG_COLOR,
) {
    companion object {
        fun saved(context: Context, originalOnly: Boolean = false) = CaptionFrameStyle(
            SettingsStore.fontSize(context), SettingsStore.translationColor(context),
            SettingsStore.overlayOpacity(context), SettingsStore.overlayBgAlpha(context),
            if (originalOnly || SettingsStore.originalTextOnly(context)) CaptionDisplayMode.ORIGINAL else SettingsStore.displayMode(context),
            SettingsStore.captionAlignment(context),
            SettingsStore.subtitleDividers(context),
            SettingsStore.overlayBgColor(context),
        )
    }
}

/** Matches desktop subtitleBackgroundColor: user-selected RGB color with independent background alpha. */
internal object CaptionSurface {
    fun apply(view: View, style: CaptionFrameStyle, immersive: Boolean) {
        view.alpha = if (immersive) 1f else style.opacity / 100f
        (view.background as GradientDrawable).setColor(Color.argb(
            if (immersive) 0 else style.backgroundAlpha.coerceIn(0, 100) * 255 / 100,
            CaptionColorMath.red(style.backgroundColor),
            CaptionColorMath.green(style.backgroundColor),
            CaptionColorMath.blue(style.backgroundColor),
        ))
    }
}

/** Native frame sizing shared by the preview and WindowManager adapter. */
internal fun captionPanelHeight(availableHeight: Int, percent: Int, chromeHeight: Int, minimumReadingHeight: Int, limit: Int = availableHeight): Int {
    return maxOf(chromeHeight + minimumReadingHeight, availableHeight * percent.coerceIn(10, 70) / 100).coerceAtMost(limit)
}
