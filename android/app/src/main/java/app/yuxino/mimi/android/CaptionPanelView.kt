package app.yuxino.mimi.android

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.view.MotionEvent
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import app.yuxino.mimi.android.capture.LatestCaptionTextView

internal data class CaptionPanelActions(
    val home: (() -> Unit)? = null,
)

/** The same native caption frame in the system overlay and the settings preview. */
internal class CaptionPanelView(
    context: Context,
    private val actions: CaptionPanelActions = CaptionPanelActions(),
) : FrameLayout(context) {
    val status: TextView
    val source: TextView
    val translation: TextView
    val history: TextView
    val transcriptScroll: ScrollView
    val timestamp: TextView
    private var followCurrentOnLayout = false
    internal var followingLive = true
        private set
    private var readingAnchor: Int? = null
    private var touchY = 0f
    private var towardsLive = false
    private var displayMode = CaptionDisplayMode.BILINGUAL
    private var showDividers = false
    private var interactive = actions.home != null
    private var minimumSourceLine = 0
    private var minimumTranslationLine = 0
    private var minimumTimeLine = 0
    private val sourceViewport = CaptionSourceViewport()

    init {
        tag = "expanded-subtitles"
        background = GradientDrawable().apply { cornerRadius = dp(12).toFloat() }
        val panel = this

        status = TextView(context).apply {
            tag = "expanded-status"
            setTextColor(0xFFD1D1D1.toInt())
            textSize = 16f
            visibility = View.GONE
        }

        val scroll = ScrollView(context).apply {
            tag = "subtitle-transcript-scroll"
            isFillViewport = true
        }
        transcriptScroll = scroll
        scroll.setOnTouchListener { _, event ->
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> { touchY = event.y; towardsLive = false }
                MotionEvent.ACTION_MOVE -> {
                    val delta = event.y - touchY
                    if (delta > dp(2)) beginReading()
                    if (delta < -dp(2)) towardsLive = true
                    touchY = event.y
                }
                MotionEvent.ACTION_UP -> if (towardsLive && atBottom()) followLive()
                MotionEvent.ACTION_CANCEL -> towardsLive = false
            }
            false
        }
        scroll.setOnScrollChangeListener { _, _, _, _, _ ->
            if (!followingLive && towardsLive && readingAnchor == null && atBottom()) followLive()
        }
        scroll.viewTreeObserver.addOnPreDrawListener {
            val anchor = readingAnchor
            if (anchor != null) {
                readingAnchor = null
                scroll.scrollTo(0, anchor)
            } else if (isShown && followCurrentOnLayout) {
                followCurrentOnLayout = false
                scroll.scrollTo(0, maxScroll())
            }
            true
        }
        val transcript = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            // Match the desktop reading edge: growth in the reference lane
            // must not push an unchanged translation down the panel. Fill the
            // viewport from its bottom; overflowing history remains scrollable.
            gravity = Gravity.BOTTOM
        }
        transcript.addView(status, LinearLayout.LayoutParams(-1, -2))
        history = CaptionHistoryTextView(context).apply {
            setTextColor(0xFFB9B9B9.toInt())
            textSize = (17 - 1).coerceAtLeast(12).toFloat()
            setLineSpacing(dp(5).toFloat(), 1f)
        }
        transcript.addView(history)
        transcript.addView(View(context).apply { tag = "history-divider"; setBackgroundColor(0xFF555555.toInt()) },
            LinearLayout.LayoutParams(-1, dp(1)))
        timestamp = captionTimestampView(context)
        transcript.addView(timestamp)
        source = LatestCaptionTextView(context).apply {
            tag = "expanded-source"
            setTextColor(0xFFD5D5D5.toInt())
            textSize = 17.toFloat()
        }
        translation = LatestCaptionTextView(context).apply {
            tag = "expanded-translation"
            setTextColor(Color.WHITE)
            textSize = 16f
            typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        }
        // Following keeps both languages in the viewport. Deliberate scrolling
        // opens their full bounded content without changing the user's window.
        transcript.addView(source, LinearLayout.LayoutParams(-1, -2))
        transcript.addView(translation, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(6) })
        scroll.addView(transcript)
        panel.addView(scroll, FrameLayout.LayoutParams(-1, -1).apply {
            setMargins(dp(16), dp(CaptionAdjustmentSwitch.TARGET_DP), dp(16), dp(8))
        })
        // One surface: the inset control targets clear the reading viewport.
        // No separate toolbar background, side column or footer row.
        panel.addView(CaptionAdjustmentSwitch(context).apply {
            tag = "overlay-adjust-mark"
            isChecked = SettingsStore.overlayAdjustable(context)
            isClickable = false; isFocusable = false
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }, FrameLayout.LayoutParams(dp(CaptionAdjustmentSwitch.TARGET_DP), dp(CaptionAdjustmentSwitch.TARGET_DP),
            Gravity.TOP or Gravity.RIGHT).apply { rightMargin = dp(CaptionAdjustmentSwitch.RIGHT_INSET_DP) })
        panel.addView(FrameLayout(context).apply {
            tag = "overlay-drag"; isClickable = true; isFocusable = true
            contentDescription = interfaceString(R.string.overlay_drag)
            setOnClickListener { if (interactive) actions.home?.invoke() }
            addView(View(context).apply {
                tag = "overlay-drag-mark"
                background = GradientDrawable().apply {
                    cornerRadius = dp(2).toFloat(); setColor(0xFFB9B9B9.toInt())
                }
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }, FrameLayout.LayoutParams(dp(24), dp(3), Gravity.CENTER))
        }, FrameLayout.LayoutParams(dp(48), dp(48), Gravity.TOP or Gravity.CENTER_HORIZONTAL))
        panel.addView(panelIcon(R.drawable.ic_overlay_resize, R.string.overlay_resize).apply {
            tag = "overlay-resize"
            // Keep the visible mark in the bottom/right inset, not over a glyph.
            setPadding(dp(32), dp(32), dp(4), dp(4))
        }, FrameLayout.LayoutParams(dp(48), dp(48), Gravity.BOTTOM or Gravity.RIGHT))
        setAdjustable(SettingsStore.overlayAdjustable(context))
    }

    /** Disable actions without dimming or changing the frame's visible appearance. */
    fun setPreviewOnly(preview: Boolean) {
        interactive = !preview && actions.home != null
        listOf("overlay-drag", "overlay-resize").forEach { tag ->
            findViewWithTag<View>(tag).apply {
                if (actions.home == null) setOnClickListener(null)
                isClickable = interactive; isFocusable = interactive
                tooltipText = if (interactive) contentDescription else null
                importantForAccessibility = if (interactive) View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
                    else View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
        }
    }

    /** Keep bounds and reading position unchanged when the window stops receiving touches. */
    fun setAdjustable(adjustable: Boolean) {
        findViewWithTag<CaptionAdjustmentSwitch>("overlay-adjust-mark").isChecked = adjustable
        findViewWithTag<View>("overlay-drag-mark").visibility = if (adjustable) View.VISIBLE else View.INVISIBLE
        findViewWithTag<View>("overlay-resize").visibility = if (adjustable) View.VISIBLE else View.INVISIBLE
        setPreviewOnly(!adjustable)
    }

    fun applyStyle(style: CaptionFrameStyle) {
        sourceViewport.reset()
        displayMode = style.mode; showDividers = style.dividers
        (history as CaptionHistoryTextView).dividers = style.dividers
        CaptionTypography.apply(source, translation, style.fontSize, false, style.mode == CaptionDisplayMode.ORIGINAL)
        minimumSourceLine = CaptionTypography.minimumLineHeight(source)
        minimumTranslationLine = CaptionTypography.minimumLineHeight(translation)
        minimumTimeLine = CaptionTypography.minimumLineHeight(timestamp)
        status.textSize = style.fontSize.toFloat()
        history.textSize = (style.fontSize - 1).coerceAtLeast(12).toFloat()
        status.setTextColor(style.color)
        source.setTextColor(if (style.mode == CaptionDisplayMode.ORIGINAL) style.color else 0xFFD5D5D5.toInt())
        translation.setTextColor(style.color)
        listOf(status, source, translation, history, timestamp).forEach { it.gravity = style.alignment.gravity }
        CaptionSurface.apply(this, style, false)
        findViewWithTag<View>("history-divider").visibility = if (history.visibility == View.VISIBLE && style.dividers) View.VISIBLE else View.GONE
        followCurrentOnLayout = followingLive
    }

    private fun maxScroll() = ((transcriptScroll.getChildAt(0)?.height ?: 0) - transcriptScroll.height).coerceAtLeast(0)
    private fun atBottom() = transcriptScroll.scrollY >= maxScroll() - dp(8)

    private fun beginReading() {
        if (!followingLive) return
        sourceViewport.reset()
        val first = if (source.visibility == View.VISIBLE) source else translation
        readingAnchor = transcriptScroll.scrollY + first.scrollY
        followingLive = false; followCurrentOnLayout = false; towardsLive = false
        requestLayout()
    }

    private fun followLive() {
        sourceViewport.reset()
        followingLive = true; followCurrentOnLayout = true; readingAnchor = null
        requestLayout()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        super.onMeasure(widthMeasureSpec, heightMeasureSpec)
        if (MeasureSpec.getMode(heightMeasureSpec) == MeasureSpec.UNSPECIFIED) return
        fun heights(view: TextView): List<Int> {
            if (view.visibility != View.VISIBLE) return emptyList()
            val layout = view.layout ?: return emptyList()
            val bottom = layout.getLineBottom(layout.lineCount - 1)
            return (1..layout.lineCount).map { count ->
                bottom - layout.getLineTop(layout.lineCount - count) + view.compoundPaddingTop + view.compoundPaddingBottom
            }
        }
        val both = source.visibility == View.VISIBLE && translation.visibility == View.VISIBLE
        val translationParams = translation.layoutParams as LinearLayout.LayoutParams
        val margin = if (both) dp(6) else 0
        val marginChanged = translationParams.topMargin != margin
        translationParams.topMargin = margin
        val available = transcriptScroll.measuredHeight - (if (both) dp(6) else 0) -
            (if (timestamp.visibility == View.VISIBLE) timestamp.measuredHeight else 0)
        val measuredBudget = if (followingLive) captionLaneBudget(available, heights(source), heights(translation))
            else Int.MAX_VALUE to Int.MAX_VALUE
        val budget = if (followingLive && both) {
            // MT may grow and then shorten while the original is unchanged. Do
            // not reveal an older reference line again just because MT freed
            // some room. Its vertical baseline still follows the compact pair.
            sourceViewport.limit(source.text.toString(), source.measuredWidth, available, measuredBudget.first) to measuredBudget.second
        } else {
            sourceViewport.reset()
            measuredBudget
        }
        var changed = false
        for ((view, count) in listOf(source to budget.first, translation to budget.second)) {
            (view as LatestCaptionTextView).followsLatest = followingLive
            val limit = count.coerceAtLeast(1)
            if (view.maxLines != limit) { view.maxLines = limit; changed = true }
        }
        if (changed || marginChanged) super.onMeasure(widthMeasureSpec, heightMeasureSpec)
        if (followingLive) followCurrentOnLayout = true
    }

    fun showContent(content: CaptionFrameContent, mode: CaptionDisplayMode, active: Boolean = true) {
        val pairChanged = source.text.toString() != content.original || translation.text.toString() != content.translated
        val historyChanged = history.text.toString() != content.history
        if (active && (historyChanged || pairChanged) && followingLive) followCurrentOnLayout = true
        content.applyRows(source, translation, status, mode)
        timestamp.setCaptionText(content.timestamp)
        timestamp.visibility = if (content.timestamp.isBlank() || content.hidden) View.GONE else View.VISIBLE
        history.setCaptionText(content.history)
        // The shared state already bounds retained pairs; a second visual cap
        // would discard later history rows when a sentence wraps.
        history.maxLines = Int.MAX_VALUE
        history.visibility = if (content.history.isBlank()) View.GONE else View.VISIBLE
        findViewWithTag<View>("history-divider").visibility = if (history.visibility == View.VISIBLE && showDividers) View.VISIBLE else View.GONE
    }

    fun minimumReadingHeight(): Int {
        // Guarantee one complete fallback-font row per active lane. Additional
        // translated lines use the chosen viewport budget, not permanent chrome.
        val mode = displayMode
        return maxOf(dp(64), (if (mode.sourceVisible) minimumSourceLine else 0) +
            (if (mode.translationVisible) minimumTranslationLine else 0) +
            (if (mode == CaptionDisplayMode.BILINGUAL) dp(6) else 0) +
            (if (SettingsStore.subtitleTimestamps(context)) minimumTimeLine else 0))
    }

    fun chromeHeight(): Int = (transcriptScroll.layoutParams as FrameLayout.LayoutParams).let {
        it.topMargin + it.bottomMargin
    }

    companion object {
        const val CHROME_HEIGHT_DP = CaptionAdjustmentSwitch.TARGET_DP + 8
    }

    private fun interfaceString(id: Int): String = InterfaceLanguage.context(context).getString(id)
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    private fun panelIcon(resource: Int, description: Int): ImageButton = ImageButton(context).apply {
        setImageResource(resource); imageTintList = android.content.res.ColorStateList.valueOf(Color.WHITE)
        setPadding(dp(12), dp(12), dp(12), dp(12)); setBackgroundColor(Color.TRANSPARENT)
        contentDescription = interfaceString(description); tooltipText = contentDescription
        isFocusable = true
    }

}
