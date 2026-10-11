package app.yuxino.mimi.android

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

internal data class CaptionPanelActions(
    val home: (() -> Unit)? = null,
    val style: (() -> Unit)? = null,
    val immerse: (() -> Unit)? = null,
    val services: (() -> Unit)? = null,
)

/** The same native caption frame in the system overlay and the settings preview. */
internal class CaptionPanelView(
    context: Context,
    route: String,
    serviceLabel: String,
    providerMarks: List<Int>,
    private val actions: CaptionPanelActions = CaptionPanelActions(),
) : LinearLayout(context) {
    val status: TextView
    val source: TextView
    val translation: TextView
    val history: TextView
    val transcriptScroll: ScrollView
    private var followCurrentOnLayout = false
    private var interactive = actions.home != null

    init {
        tag = "expanded-subtitles"
        orientation = VERTICAL
        setPadding(dp(12), dp(10), dp(12), dp(10))
        background = GradientDrawable().apply { cornerRadius = dp(22).toFloat() }
        val panel = this
        val header = FrameLayout(context).apply { tag = "overlay-toolbar" }
        header.addView(FrameLayout(context).apply {
            tag = "overlay-drag"; isClickable = true; isFocusable = true
            contentDescription = interfaceString(R.string.overlay_drag)
            setOnClickListener { if (interactive) actions.home?.invoke() }
            addView(View(context).apply {
                tag = "overlay-drag-mark"
                background = GradientDrawable().apply { cornerRadius = dp(2).toFloat(); setColor(0xFF858585.toInt()) }
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }, FrameLayout.LayoutParams(dp(36), dp(4), Gravity.TOP or Gravity.CENTER_HORIZONTAL))
            addView(TextView(context).apply {
                tag = "overlay-route"
                text = route
                contentDescription = interfaceString(R.string.overlay_language_description)
                textSize = 14f; setTextColor(0xFFDDDDDD.toInt()); maxLines = 1
                ellipsize = android.text.TextUtils.TruncateAt.END; gravity = Gravity.CENTER
            }, FrameLayout.LayoutParams(-1, -1).apply {
                topMargin = dp(8); marginStart = dp(48); marginEnd = dp(48)
            })
        }, FrameLayout.LayoutParams(-1, dp(48)))
        header.addView(panelIcon(R.drawable.ic_settings, R.string.caption_style_controls).apply {
            tag = "overlay-font"; setOnClickListener { if (interactive) actions.style?.invoke() }
        }, FrameLayout.LayoutParams(dp(48), dp(48), Gravity.END))
        panel.addView(header, LinearLayout.LayoutParams(-1, dp(48)))
        val immersion = panelButton(interfaceString(R.string.overlay_enter_immersive)).apply {
            contentDescription = text; tooltipText = text
            tag = "enter-immersive"; setOnClickListener { if (interactive) actions.immerse?.invoke() }
        }
        val immersionMark = immersion.compoundDrawablesRelative[0]
        val providerRow = LinearLayout(context).apply {
            gravity = Gravity.CENTER; tag = "overlay-providers"
            isClickable = true; isFocusable = true; minimumHeight = dp(48); minimumWidth = dp(48)
            contentDescription = serviceLabel
            setOnClickListener { if (interactive) actions.services?.invoke() }
        }
        fun mark(resource: Int) = ImageView(context).apply {
            setImageResource(resource); scaleType = ImageView.ScaleType.FIT_CENTER
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        for (resource in providerMarks) {
            providerRow.addView(mark(resource), LinearLayout.LayoutParams(dp(18), dp(18)).apply { marginEnd = dp(4) })
        }
        val providerModel = TextView(context).apply {
            tag = "overlay-model"; text = providerRow.contentDescription
            textSize = 14f; setTextColor(0xFFBBBBBB.toInt()); maxLines = 1
            ellipsize = android.text.TextUtils.TruncateAt.END
        }
        providerRow.addView(providerModel, LinearLayout.LayoutParams(-2, -2))
        val resize = panelIcon(R.drawable.ic_overlay_resize, R.string.overlay_resize).apply {
            tag = "overlay-resize"; setOnClickListener { if (interactive) actions.style?.invoke() }
        }
        val footer = object : FrameLayout(context) {
            override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
                val available = MeasureSpec.getSize(widthMeasureSpec)
                val maximumActionWidth = (available - dp(56)).coerceAtLeast(dp(48))
                val labelWidth = kotlin.math.ceil(immersion.paint.measureText(immersion.text.toString())).toInt() +
                    immersion.paddingLeft + immersion.paddingRight
                val markWidth = (immersionMark?.bounds?.width() ?: 0) + immersion.compoundDrawablePadding
                val visibleMark = immersionMark.takeIf { labelWidth + markWidth <= maximumActionWidth }
                if (immersion.compoundDrawablesRelative[0] !== visibleMark) {
                    immersion.setCompoundDrawablesRelative(visibleMark, null, null, null)
                }
                val actionWidth = (labelWidth + if (visibleMark != null) markWidth else 0)
                    .coerceIn(dp(48), maximumActionWidth)
                // Measure wrapping at the exact width FrameLayout will assign.
                // AT_MOST can shrink to a rounded text width that gains another
                // line during the subsequent exact pass after a locale change.
                immersion.measure(MeasureSpec.makeMeasureSpec(actionWidth, MeasureSpec.EXACTLY),
                    MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED))
                val actionHeight = maxOf(immersion.measuredHeight, dp(48))
                val marksWidth = (providerRow.childCount - 1) * dp(22)
                val nameWidth = kotlin.math.ceil(providerModel.paint.measureText(providerModel.text.toString())).toInt()
                val centerWidth = available - 2 * (actionWidth + dp(8))
                val singleRow = centerWidth >= maxOf(dp(48), marksWidth + nameWidth)
                // Keep the provider group centered against the whole window.
                // Long names get their own row instead of shifting off-center
                // or squeezing the localized immersion action.
                providerModel.maxWidth = ((if (singleRow) centerWidth else available) - marksWidth).coerceAtLeast(0)
                (providerRow.layoutParams as LayoutParams).apply {
                    gravity = if (singleRow) Gravity.CENTER else Gravity.TOP or Gravity.CENTER_HORIZONTAL
                }
                (immersion.layoutParams as LayoutParams).apply {
                    width = actionWidth; height = actionHeight
                }
                val desiredHeight = actionHeight + if (singleRow) 0 else dp(48)
                super.onMeasure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(
                    resolveSize(desiredHeight, heightMeasureSpec), MeasureSpec.EXACTLY))
            }
        }.apply { tag = "overlay-actions" }
        footer.addView(providerRow, FrameLayout.LayoutParams(-2, dp(48), Gravity.CENTER))
        footer.addView(immersion, FrameLayout.LayoutParams(-2, -2, Gravity.BOTTOM or Gravity.START))
        footer.addView(resize, FrameLayout.LayoutParams(dp(48), dp(48), Gravity.BOTTOM or Gravity.END))

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
        scroll.viewTreeObserver.addOnPreDrawListener {
            if (isShown && followCurrentOnLayout) {
                followCurrentOnLayout = false
                currentCaption()?.let { scroll.scrollTo(0, it.top) }
            }
            true
        }
        val transcript = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
        transcript.addView(status, LinearLayout.LayoutParams(-1, -2))
        history = TextView(context).apply {
            setTextColor(0xFFB9B9B9.toInt())
            textSize = (17 - 1).coerceAtLeast(12).toFloat()
            setLineSpacing(dp(5).toFloat(), 1f)
        }
        transcript.addView(history)
        transcript.addView(View(context).apply { tag = "history-divider"; setBackgroundColor(0xFF555555.toInt()) },
            LinearLayout.LayoutParams(-1, dp(1)))
        source = TextView(context).apply {
            tag = "expanded-source"
            setTextColor(0xFFD5D5D5.toInt())
            textSize = 17.toFloat()
        }
        translation = TextView(context).apply {
            tag = "expanded-translation"
            setTextColor(Color.WHITE)
            textSize = (17 + 3).toFloat()
            setTypeface(typeface, Typeface.BOLD)
        }
        // The shared core bounds sentence content. The reading panel keeps its
        // fixed viewport while allowing the complete current pair to scroll.
        // Compact and immersive captions retain their two/three-line limits.
        transcript.addView(source, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(14) })
        transcript.addView(translation, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(6) })
        scroll.addView(transcript)
        panel.addView(scroll, LinearLayout.LayoutParams(-1, 0, 1f).apply { topMargin = dp(8) })
        panel.addView(footer, LinearLayout.LayoutParams(-1, -2))
        setPreviewOnly(actions.home == null)
    }

    /** Disable actions without dimming or changing the frame's visible appearance. */
    fun setPreviewOnly(preview: Boolean) {
        interactive = !preview && actions.home != null
        listOf("overlay-drag", "overlay-font", "enter-immersive", "overlay-providers", "overlay-resize").forEach { tag ->
            findViewWithTag<View>(tag).apply {
                isClickable = interactive; isFocusable = interactive
                tooltipText = if (interactive) contentDescription else null
                importantForAccessibility = if (interactive) View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
                    else View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
        }
    }

    fun applyStyle(style: CaptionFrameStyle) {
        CaptionTypography.apply(source, translation, style.fontSize, false, style.mode == CaptionDisplayMode.ORIGINAL)
        status.textSize = style.fontSize.toFloat()
        history.textSize = (style.fontSize - 1).coerceAtLeast(12).toFloat()
        status.setTextColor(style.color)
        source.setTextColor(if (style.mode == CaptionDisplayMode.ORIGINAL) style.color else 0xFFD5D5D5.toInt())
        translation.setTextColor(style.color)
        listOf(status, source, translation, history).forEach { it.gravity = style.alignment.gravity }
        CaptionSurface.apply(this, style, false)
    }

    private fun currentCaption() = source.takeIf { it.visibility == View.VISIBLE && it.text.isNotEmpty() }
        ?: translation.takeIf { it.visibility == View.VISIBLE && it.text.isNotEmpty() }

    private fun readingCurrent(): Boolean {
        val current = currentCaption() ?: return true
        val maximum = ((transcriptScroll.getChildAt(0)?.height ?: 0) - transcriptScroll.height).coerceAtLeast(0)
        return transcriptScroll.scrollY >= minOf(current.top, maximum) - dp(8)
    }

    fun showContent(content: CaptionFrameContent, mode: CaptionDisplayMode, active: Boolean = true) {
        val pairChanged = source.text.toString() != content.original || translation.text.toString() != content.translated
        val historyChanged = history.text.toString() != content.history
        if (active && (historyChanged || pairChanged && content.confirmed) && readingCurrent()) followCurrentOnLayout = true
        content.applyRows(source, translation, status, mode)
        history.text = content.history
        history.maxLines = maxOf(content.historyLimit, 1) * 3
        history.visibility = if (content.history.isBlank()) View.GONE else View.VISIBLE
        findViewWithTag<View>("history-divider").visibility = history.visibility
    }

    fun chromeHeight(): Int = paddingTop + paddingBottom + dp(8) +
        findViewWithTag<View>("overlay-toolbar").measuredHeight + findViewWithTag<View>("overlay-actions").measuredHeight

    private fun interfaceString(id: Int): String = InterfaceLanguage.context(context).getString(id)
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    private fun panelIcon(resource: Int, description: Int): ImageButton = ImageButton(context).apply {
        setImageResource(resource); imageTintList = android.content.res.ColorStateList.valueOf(Color.WHITE)
        setPadding(dp(12), dp(12), dp(12), dp(12)); setBackgroundColor(Color.TRANSPARENT)
        contentDescription = interfaceString(description); tooltipText = contentDescription
        isFocusable = true
    }

    private fun panelButton(label: String): TextView = TextView(context).apply {
        text = label
        textSize = 14f
        minimumHeight = dp(48)
        minimumWidth = dp(48)
        typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
        setTextColor(Color.WHITE)
        gravity = Gravity.CENTER
        setPadding(dp(8), 0, dp(8), 0)
        val icon = androidx.core.content.ContextCompat.getDrawable(context, R.drawable.ic_translation_captions)?.mutate()
        icon?.setTint(Color.WHITE); icon?.setBounds(0, 0, dp(18), dp(18))
        setCompoundDrawablesRelative(icon, null, null, null); compoundDrawablePadding = dp(6)
        background = android.graphics.drawable.StateListDrawable().apply {
            fun feedback() = GradientDrawable().apply { cornerRadius = dp(8).toFloat(); setColor(0x30FFFFFF) }
            addState(intArrayOf(android.R.attr.state_pressed), feedback())
            addState(intArrayOf(android.R.attr.state_focused), feedback())
            addState(intArrayOf(), android.graphics.drawable.ColorDrawable(Color.TRANSPARENT))
        }
        isClickable = true
        isFocusable = true
        pointerIcon = android.view.PointerIcon.getSystemIcon(context, android.view.PointerIcon.TYPE_HAND)
    }

}
