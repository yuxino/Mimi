package app.yuxino.mimi.android.captions

import android.content.Context
import android.view.View
import android.widget.TextView
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** The shared reducer supplies confirmation time; rendering never invents an arrival time. */
internal fun captionTime(epochMs: Long?): String = epochMs?.takeIf { it > 0 }?.let {
    SimpleDateFormat("HH:mm:ss", Locale.ROOT).format(Date(it))
} ?: ""

internal fun captionTimestampView(context: Context) = TextView(context).apply {
    tag = "caption-time"; textSize = 12f
    typeface = android.graphics.Typeface.create("sans-serif", android.graphics.Typeface.NORMAL)
    setTextColor(0xFFBBBBBB.toInt()); visibility = View.GONE
}
