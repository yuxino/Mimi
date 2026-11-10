package app.yuxino.mimi.android

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Shader
import android.view.View

/** Deterministic synthetic playback scene, never a captured/private video or a provider session. */
internal class CaptionReviewBackdrop(context: Context) : View(context) {
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.save()
        canvas.scale(width / 720f, height / 1440f)
        paint.shader = LinearGradient(0f, 0f, 0f, 1440f,
            intArrayOf(0xFF384E62.toInt(), 0xFFD0AB8D.toInt(), 0xFF567E87.toInt(), 0xFF244851.toInt()),
            floatArrayOf(0f, .43f, .55f, 1f), Shader.TileMode.CLAMP)
        canvas.drawRect(0f, 0f, 720f, 1440f, paint)
        paint.shader = null
        paint.color = 0xFFFFD8A6.toInt()
        canvas.drawCircle(511f, 506f, 52f, paint)
        fun mountain(color: Int, vararg points: Float) {
            paint.color = color
            val path = Path().apply {
                moveTo(points[0], points[1])
                for (i in 2 until points.size step 2) lineTo(points[i], points[i + 1])
                close()
            }
            canvas.drawPath(path, paint)
        }
        mountain(0xFF536575.toInt(), 0f, 752f, 0f, 521f, 104f, 432f, 206f, 545f, 310f, 499f, 469f, 680f, 720f, 725f)
        mountain(0xFF344C56.toInt(), 0f, 780f, 0f, 650f, 116f, 576f, 226f, 641f, 321f, 722f, 720f, 765f)
        paint.color = 0x32F1D6B9
        for (i in 0..16) {
            val y = 805f + i * 19f
            val spread = 26f + i * 5f
            canvas.drawRect(511f - spread, y, 511f + spread, y + 2f, paint)
        }
        mountain(0xFF1F3539.toInt(), 0f, 1440f, 0f, 1015f, 88f, 956f, 161f, 1023f, 205f, 1161f, 302f, 1230f, 372f, 1440f)
        paint.color = 0xFFDFE5E7.toInt()
        paint.textSize = 22f
        canvas.drawText("SYNTHETIC PLAYBACK · UI REVIEW", 36f, 142f, paint)
        paint.color = 0xFFBFCAD0.toInt()
        paint.textSize = 18f
        canvas.drawText("Static test scene · No audio or network", 36f, 174f, paint)
        // Keep fixture playback controls clear of the real system navigation bar.
        canvas.save()
        canvas.translate(0f, -44f)
        paint.color = 0x99303E44.toInt()
        canvas.drawRoundRect(28f, 1354f, 692f, 1414f, 16f, 16f, paint)
        paint.color = Color.WHITE
        canvas.drawRect(52f, 1372f, 57f, 1396f, paint)
        canvas.drawRect(64f, 1372f, 69f, 1396f, paint)
        paint.color = 0x5FFFFFFF
        canvas.drawRoundRect(92f, 1382f, 577f, 1386f, 2f, 2f, paint)
        paint.color = 0xFFCBD8D2.toInt()
        canvas.drawRoundRect(92f, 1382f, 309f, 1386f, 2f, 2f, paint)
        canvas.drawCircle(309f, 1384f, 5f, paint)
        paint.color = Color.WHITE
        paint.textSize = 18f
        canvas.drawText("02:18", 604f, 1390f, paint)
        canvas.restore()
        canvas.restore()
    }
}
