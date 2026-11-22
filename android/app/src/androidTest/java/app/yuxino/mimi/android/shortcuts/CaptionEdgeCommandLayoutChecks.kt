package app.yuxino.mimi.android.shortcuts

import android.content.Context
import android.content.res.Configuration
import android.graphics.Rect
import android.os.LocaleList
import android.view.View
import android.view.View.MeasureSpec
import android.widget.TextView
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.localization.InterfaceLanguage
import kotlin.math.roundToInt

/** Check the real command at the edge of a 320dp screen, including large-script font metrics. */
internal fun checkCaptionEdgeCommandLayout(context: Context) {
    for (language in InterfaceLanguage.tags.filter(String::isNotEmpty)) for (scale in listOf(1f, 2f)) {
        val configuration = Configuration(context.resources.configuration).apply {
            setLocales(LocaleList.forLanguageTags(language))
            fontScale = scale
        }
        val localized = context.createConfigurationContext(configuration)
        val density = localized.resources.displayMetrics.density
        val touchHeight = (48 * density).roundToInt()
        val availableWidth = ((320 - 26) * density).roundToInt()
        val command = CaptionEdgeCommand(localized).apply { visibility = View.VISIBLE }
        for (label in listOf(R.string.start_action, R.string.stop_action)) {
            val text = localized.getString(label)
            command.setLabel(text)
            val width = command.measuredWidthWithin(availableWidth)
            val views = ArrayList<View>()
            command.findViewsWithText(views, text, View.FIND_VIEWS_WITH_TEXT)
            val field = views.filterIsInstance<TextView>().single { it.text.toString() == text }
            for (side in CaptionEdge.entries) {
                command.alignToPortrait(side)
                command.measure(MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY),
                    MeasureSpec.makeMeasureSpec(touchHeight, MeasureSpec.EXACTLY))
                command.layout(0, 0, width, touchHeight)
                val labelBounds = Rect(0, 0, field.width, field.height)
                command.offsetDescendantRectToMyCoords(field, labelBounds)
                val expectedLeft = if (side == CaptionEdge.LEFT) 0 else width - labelBounds.width()
                check(labelBounds.width() in 1..width && labelBounds.height() in 1..touchHeight &&
                    labelBounds.left == expectedLeft && labelBounds.top == (touchHeight - labelBounds.height()) / 2) {
                    "Misaligned $language '$text' on $side at font scale $scale: label=$labelBounds, target=${width}x$touchHeight"
                }
            }
            val layout = checkNotNull(field.layout)
            check(width in touchHeight..availableWidth && layout.lineCount == 1 && layout.getEllipsisCount(0) == 0 &&
                layout.getLineEnd(0) == text.length &&
                layout.getLineWidth(0) <= field.width - field.compoundPaddingLeft - field.compoundPaddingRight + 1 &&
                layout.height <= field.height - field.compoundPaddingTop - field.compoundPaddingBottom) {
                "Clipped $language command '$text' at font scale $scale: field=${field.width}x${field.height}, " +
                    "text=${layout.getLineWidth(0)}x${layout.height}, lines=${layout.lineCount}"
            }
        }
    }
}
