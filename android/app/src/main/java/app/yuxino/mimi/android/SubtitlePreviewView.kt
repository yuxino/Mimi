package app.yuxino.mimi.android

import android.content.Context
import android.os.Build
import android.util.AttributeSet
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.capture.OverlayPlacement

/** A scaled view of the actual native caption window; never starts capture or a provider. */
class SubtitlePreviewView @JvmOverloads constructor(context: Context, attrs: AttributeSet? = null) : LinearLayout(context, attrs) {
    private val labels = LinearLayout(context).apply { gravity = Gravity.CENTER_VERTICAL }
    private var heightLimit = Int.MAX_VALUE
    private val previewLabel = TextView(context)
    private val exampleLabel = TextView(context)
    internal val stage = CaptionPreviewStage(context)
    internal var frame: View = View(context)
        private set
    private var identity = ""
    private var displayMode = CaptionDisplayMode.BILINGUAL
    private lateinit var source: TextView
    private lateinit var translation: TextView
    private lateinit var status: TextView

    init {
        orientation = VERTICAL
        listOf(previewLabel, exampleLabel).forEach { it.textSize = 14f; it.setTextColor(ContextCompat.getColor(context, R.color.mimi_muted)) }
        labels.addView(previewLabel, LayoutParams(0, -2, 1f))
        labels.addView(exampleLabel)
        addView(labels)
        addView(stage, LayoutParams(-1, dp(230)).apply { topMargin = dp(8); bottomMargin = dp(8) })
        configure(16, android.graphics.Color.WHITE, 100, 80, "zh", false)
    }

    fun configure(fontSize: Int, color: Int, opacity: Int, backgroundAlpha: Int, targetLang: String,
                  immersive: Boolean, originalOnly: Boolean = false, mode: CaptionDisplayMode = CaptionDisplayMode.BILINGUAL,
                  widthPercent: Int = 92, live: Boolean = false, alignment: CaptionAlignment = CaptionAlignment.CENTER,
                  heightPercent: Int = 30) {
        displayMode = if (originalOnly) CaptionDisplayMode.ORIGINAL else mode
        val localized = InterfaceLanguage.context(context.applicationContext)
        val route = languageDisplayName(localized, SettingsStore.sourceLang(context)).let {
            if (originalOnly) it else "$it → ${languageDisplayName(localized, targetLang)}"
        }
        val label = overlayServiceLabel(localized)
        val marks = captionProviderMarks(context)
        val nextIdentity = "$immersive|${localized.resources.configuration.locales}|$route|$label|$marks"
        if (identity != nextIdentity) {
            identity = nextIdentity
            frame = if (immersive) CompactCaptionView(localized) else CaptionPanelView(localized, route, label, marks)
            stage.replaceFrame(frame)
            when (val current = frame) {
                is CaptionPanelView -> { source = current.source; translation = current.translation; status = current.status }
                is CompactCaptionView -> { source = current.source; translation = current.translation; status = current.status }
            }
        }
        val style = CaptionFrameStyle(fontSize, color, opacity.coerceIn(20, 100), backgroundAlpha, displayMode, alignment)
        when (val current = frame) {
            is CaptionPanelView -> current.applyStyle(style)
            is CompactCaptionView -> current.applyStyle(style, true)
        }
        stage.configure(widthPercent, heightPercent, immersive)
        previewLabel.setText(if (live) R.string.home_title else R.string.preview_label)
        exampleLabel.visibility = if (live) GONE else VISIBLE
        exampleLabel.setText(R.string.preview_example)
        if (!live) {
            val original = context.getString(when (targetLang) {
                "en" -> R.string.preview_zh
                "th" -> R.string.preview_en
                else -> R.string.preview_source
            })
            val translated = context.getString(when (targetLang) {
                "en" -> R.string.preview_en
                "ja" -> R.string.preview_ja
                "th" -> R.string.preview_th
                else -> R.string.preview_zh
            })
            showContent(CaptionFrameContent(original, translated))
        }
    }

    internal fun limitHeight(pixels: Int) { heightLimit = pixels.coerceAtLeast(0) }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        labels.measure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED))
        val stageHeight = (heightLimit - labels.measuredHeight - dp(16)).coerceIn(0, dp(230))
        (stage.layoutParams as LayoutParams).height = stageHeight
        super.onMeasure(widthMeasureSpec, heightMeasureSpec)
    }

    internal fun showContent(content: CaptionFrameContent) {
        when (val current = frame) {
            is CaptionPanelView -> current.showContent(content, displayMode)
            is CompactCaptionView -> content.applyRows(current.source, current.translation, current.status, displayMode)
        }
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
}

/** Measure the entire real frame before scaling it uniformly into a fixed stage. */
internal class CaptionPreviewStage(context: Context) : ViewGroup(context) {
    private var widthPercent = 92
    private var heightPercent = 30
    private var immersive = false
    internal var frameScale = 1f
        private set
    private var offsetX = 0f
    private var offsetY = 0f

    init { clipChildren = false; clipToPadding = false }
    fun replaceFrame(frame: View) { removeAllViews(); addView(frame) }
    fun configure(widthPercent: Int, heightPercent: Int, immersive: Boolean) {
        this.widthPercent = widthPercent; this.heightPercent = heightPercent; this.immersive = immersive
        // Match WindowManager's touch-through opacity without changing the frame style.
        alpha = if (immersive) .8f else 1f
        // Transparent white captions need a neutral video-like reference surface
        // on the light settings page. This belongs to the stage, never the frame.
        setBackgroundColor(if (immersive) 0xFF333333.toInt() else android.graphics.Color.TRANSPARENT)
        requestLayout()
    }
    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val width = MeasureSpec.getSize(widthMeasureSpec)
        val height = resolveSize(dp(230), heightMeasureSpec)
        setMeasuredDimension(width, height)
        val frame = getChildAt(0) ?: return
        // An activity can occupy only part of the display. The floating service
        // uses the application display frame, so the preview must do the same.
        val wm = context.applicationContext.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        val extent = if (Build.VERSION.SDK_INT >= 30) {
            val metrics = wm.currentWindowMetrics
            val insets = metrics.windowInsets.getInsetsIgnoringVisibility(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
            OverlayPlacement.availableExtent(metrics.bounds.width(), insets.left, insets.right) to
                OverlayPlacement.availableExtent(metrics.bounds.height(), insets.top, insets.bottom)
        } else {
            val bounds = android.graphics.Rect().also(::getWindowVisibleDisplayFrame)
            if (!bounds.isEmpty) bounds.width() to bounds.height()
            else resources.displayMetrics.widthPixels to resources.displayMetrics.heightPixels
        }
        val nativeWidth = OverlayPlacement.windowWidth(extent.first, widthPercent, dp(if (immersive) 160 else 176), dp(8))
        val exactWidth = MeasureSpec.makeMeasureSpec(nativeWidth, MeasureSpec.EXACTLY)
        frame.measure(exactWidth, MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED))
        if (frame is CaptionPanelView) {
            val nativeHeight = captionPanelHeight(extent.second, heightPercent, frame.chromeHeight(),
                frame.translation.paint.fontSpacing.toInt(), resources.displayMetrics.density)
            frame.measure(exactWidth, MeasureSpec.makeMeasureSpec(nativeHeight, MeasureSpec.EXACTLY))
        }
        frameScale = minOf(1f, width.toFloat() / frame.measuredWidth.coerceAtLeast(1), height.toFloat() / frame.measuredHeight.coerceAtLeast(1))
        offsetX = (width - frame.measuredWidth * frameScale) / 2
        offsetY = (height - frame.measuredHeight * frameScale) / 2
    }
    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        getChildAt(0)?.let {
            it.pivotX = 0f; it.pivotY = 0f; it.scaleX = frameScale; it.scaleY = frameScale
            val x = offsetX.toInt(); val y = offsetY.toInt()
            it.layout(x, y, x + it.measuredWidth, y + it.measuredHeight)
        }
    }
    // This is a preview: decorative home/style/provider buttons must never navigate.
    override fun onInterceptTouchEvent(event: android.view.MotionEvent) = true
    override fun onTouchEvent(event: android.view.MotionEvent) = true
    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()
}
