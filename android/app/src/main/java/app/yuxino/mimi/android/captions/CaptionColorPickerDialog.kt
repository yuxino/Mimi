package app.yuxino.mimi.android.captions

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.ComposeShader
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.RectF
import android.graphics.Shader
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.text.Editable
import android.text.InputFilter
import android.text.TextWatcher
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AlertDialog
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.R
import kotlin.math.max

/** Native color picker dialog combining an HSV color palette with a synced #RRGGBB HEX field. */
internal object CaptionColorPickerDialog {
    fun show(
        context: Context,
        title: String,
        initialColor: Int,
        onSelected: (Int) -> Unit,
    ) {
        val density = context.resources.displayMetrics.density
        fun dp(value: Int) = (value * density + 0.5f).toInt()
        val textColor = ContextCompat.getColor(context, R.color.mimi_text)
        val mutedColor = ContextCompat.getColor(context, R.color.mimi_muted)
        val borderColor = ContextCompat.getColor(context, R.color.mimi_border)

        val hsv = FloatArray(3)
        Color.colorToHSV(initialColor or (0xFF shl 24), hsv)
        var selectedColor = initialColor or (0xFF shl 24)
        var syncingText = false

        val root = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(16), dp(20), dp(8))
        }

        val svView = SaturationValueView(context, hsv)
        val hueView = HueBarView(context, hsv[0])

        val previewRow = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        val swatchDrawable = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            setColor(selectedColor)
            setStroke(dp(1), mutedColor)
        }
        val previewSwatch = View(context).apply { background = swatchDrawable }
        val hexInput = EditText(context).apply {
            setText(CaptionColorMath.formatHexColor(selectedColor))
            hint = "#1E293B"
            setTextColor(textColor)
            setHintTextColor(mutedColor)
            typeface = Typeface.MONOSPACE
            textSize = 15f
            isSingleLine = true
            filters = arrayOf(InputFilter.LengthFilter(9))
            background = ContextCompat.getDrawable(context, R.drawable.settings_field)
            setPadding(dp(12), dp(10), dp(12), dp(10))
            contentDescription = context.getString(R.string.settings_hex_color)
        }
        val errorText = TextView(context).apply {
            textSize = 13f
            setTextColor(0xFFDC2626.toInt())
            text = context.getString(R.string.settings_hex_invalid)
            visibility = View.GONE
        }

        fun updateFromHsv() {
            selectedColor = Color.HSVToColor(hsv)
            swatchDrawable.setColor(selectedColor)
            errorText.visibility = View.GONE
            syncingText = true
            val formatted = CaptionColorMath.formatHexColor(selectedColor)
            if (hexInput.text.toString() != formatted) {
                hexInput.setText(formatted)
                hexInput.setSelection(formatted.length)
            }
            syncingText = false
        }

        svView.onChanged = { sat, value ->
            hsv[1] = sat
            hsv[2] = value
            updateFromHsv()
        }
        hueView.onHueChanged = { hue ->
            hsv[0] = hue
            svView.setHue(hue)
            updateFromHsv()
        }

        hexInput.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) = Unit
            override fun afterTextChanged(s: Editable?) {
                if (syncingText) return
                errorText.visibility = View.GONE
                val parsed = CaptionColorMath.parseHexColor(s?.toString()) ?: return
                selectedColor = parsed
                Color.colorToHSV(parsed, hsv)
                swatchDrawable.setColor(parsed)
                hueView.setHue(hsv[0])
                svView.setHsv(hsv[0], hsv[1], hsv[2])
            }
        })

        previewRow.addView(previewSwatch, LinearLayout.LayoutParams(dp(36), dp(36)))
        previewRow.addView(hexInput, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f).apply {
            marginStart = dp(12)
        })

        root.addView(svView, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(164)))
        root.addView(hueView, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(28)).apply {
            topMargin = dp(12)
        })
        root.addView(View(context).apply { setBackgroundColor(borderColor) },
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, dp(1)).apply {
                topMargin = dp(16); bottomMargin = dp(14)
            })
        root.addView(previewRow, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT))
        root.addView(errorText, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            topMargin = dp(6)
        })

        val dialog = AlertDialog.Builder(context)
            .setTitle(title)
            .setView(root)
            .setNegativeButton(android.R.string.cancel, null)
            .setPositiveButton(android.R.string.ok, null)
            .create()

        dialog.setOnShowListener {
            dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
                val parsed = CaptionColorMath.parseHexColor(hexInput.text?.toString())
                if (parsed == null) {
                    errorText.visibility = View.VISIBLE
                    return@setOnClickListener
                }
                onSelected(parsed)
                dialog.dismiss()
            }
        }
        dialog.show()
    }

    private class SaturationValueView(context: Context, initialHsv: FloatArray) : View(context) {
        private var hue = initialHsv[0]
        private var sat = initialHsv[1]
        private var value = initialHsv[2]
        var onChanged: ((Float, Float) -> Unit)? = null

        private val fillPaint = Paint(Paint.ANTI_ALIAS_FLAG)
        private val ringOuter = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = 2.5f * resources.displayMetrics.density
            color = Color.WHITE
        }
        private val ringInner = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = 1.5f * resources.displayMetrics.density
            color = 0xAA000000.toInt()
        }
        private val bounds = RectF()
        private val corner = 8f * resources.displayMetrics.density

        init {
            setLayerType(LAYER_TYPE_SOFTWARE, null)
        }

        fun setHue(nextHue: Float) {
            hue = nextHue
            rebuildShader()
            invalidate()
        }

        fun setHsv(nextHue: Float, nextSat: Float, nextValue: Float) {
            hue = nextHue
            sat = nextSat.coerceIn(0f, 1f)
            value = nextValue.coerceIn(0f, 1f)
            rebuildShader()
            invalidate()
        }

        override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
            super.onSizeChanged(w, h, oldw, oldh)
            bounds.set(0f, 0f, w.toFloat(), h.toFloat())
            rebuildShader()
        }

        private fun rebuildShader() {
            if (width <= 0 || height <= 0) return
            val hueColor = Color.HSVToColor(floatArrayOf(hue, 1f, 1f))
            val satShader = LinearGradient(0f, 0f, width.toFloat(), 0f, Color.WHITE, hueColor, Shader.TileMode.CLAMP)
            val valShader = LinearGradient(0f, 0f, 0f, height.toFloat(), Color.TRANSPARENT, Color.BLACK, Shader.TileMode.CLAMP)
            fillPaint.shader = ComposeShader(satShader, valShader, PorterDuff.Mode.SRC_OVER)
        }

        override fun onDraw(canvas: Canvas) {
            super.onDraw(canvas)
            canvas.drawRoundRect(bounds, corner, corner, fillPaint)
            val cx = sat * width
            val cy = (1f - value) * height
            val radius = 7f * resources.displayMetrics.density
            canvas.drawCircle(cx, cy, radius, ringOuter)
            canvas.drawCircle(cx, cy, radius + 1.5f * resources.displayMetrics.density, ringInner)
        }

        override fun onTouchEvent(event: MotionEvent): Boolean {
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE -> {
                    parent?.requestDisallowInterceptTouchEvent(true)
                    sat = (event.x / max(1, width)).coerceIn(0f, 1f)
                    value = (1f - event.y / max(1, height)).coerceIn(0f, 1f)
                    invalidate()
                    onChanged?.invoke(sat, value)
                    return true
                }
            }
            return super.onTouchEvent(event)
        }
    }

    private class HueBarView(context: Context, initialHue: Float) : View(context) {
        private var hue = initialHue.coerceIn(0f, 360f)
        var onHueChanged: ((Float) -> Unit)? = null

        private val barPaint = Paint(Paint.ANTI_ALIAS_FLAG)
        private val thumbPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = 2.5f * resources.displayMetrics.density
            color = Color.WHITE
        }
        private val thumbShadow = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = 1.5f * resources.displayMetrics.density
            color = 0xAA000000.toInt()
        }
        private val rect = RectF()
        private val corner = 10f * resources.displayMetrics.density

        fun setHue(nextHue: Float) {
            hue = nextHue.coerceIn(0f, 360f)
            invalidate()
        }

        override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
            super.onSizeChanged(w, h, oldw, oldh)
            val pad = 4f * resources.displayMetrics.density
            rect.set(0f, pad, w.toFloat(), (h - pad).coerceAtLeast(pad + 1f))
            barPaint.shader = LinearGradient(
                0f, 0f, w.toFloat(), 0f,
                intArrayOf(Color.RED, Color.YELLOW, Color.GREEN, Color.CYAN, Color.BLUE, Color.MAGENTA, Color.RED),
                null,
                Shader.TileMode.CLAMP,
            )
        }

        override fun onDraw(canvas: Canvas) {
            super.onDraw(canvas)
            canvas.drawRoundRect(rect, corner, corner, barPaint)
            val cx = (hue / 360f) * width
            val cy = height / 2f
            val r = 9f * resources.displayMetrics.density
            canvas.drawCircle(cx, cy, r, thumbPaint)
            canvas.drawCircle(cx, cy, r + 1.5f * resources.displayMetrics.density, thumbShadow)
        }

        override fun onTouchEvent(event: MotionEvent): Boolean {
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE -> {
                    parent?.requestDisallowInterceptTouchEvent(true)
                    hue = ((event.x / max(1, width)).coerceIn(0f, 1f) * 360f)
                    invalidate()
                    onHueChanged?.invoke(hue)
                    return true
                }
            }
            return super.onTouchEvent(event)
        }
    }
}
