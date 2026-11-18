package app.yuxino.mimi.android.services.translation

import android.content.Context
import android.view.View
import android.view.ViewGroup
import android.widget.ArrayAdapter
import android.widget.TextView
import androidx.core.content.ContextCompat
import app.yuxino.mimi.android.R

/** One icon slot for translation choices, including recycled selected/dropdown rows. */
internal abstract class TranslationIconAdapter(context: Context, labels: List<String>) :
    ArrayAdapter<String>(context, R.layout.mimi_spinner_item, labels) {
    init { setDropDownViewResource(R.layout.mimi_spinner_icon_dropdown) }

    protected abstract fun iconResource(position: Int): Int

    override fun getView(position: Int, convertView: View?, parent: ViewGroup): View =
        bind(super.getView(position, convertView, parent), position, showArrow = true)

    override fun getDropDownView(position: Int, convertView: View?, parent: ViewGroup): View =
        bind(super.getDropDownView(position, convertView, parent), position, showArrow = false)

    private fun bind(view: View, position: Int, showArrow: Boolean): View = view.apply {
        val row = this as TextView
        val icon = ContextCompat.getDrawable(context, iconResource(position))?.apply { setBounds(0, 0, dp(24), dp(24)) }
        val arrow = if (showArrow) ContextCompat.getDrawable(context, R.drawable.ic_expand)?.apply { setBounds(0, 0, dp(16), dp(16)) } else null
        row.setCompoundDrawablesRelative(icon, null, arrow, null)
        row.compoundDrawablePadding = dp(12)
        row.textSize = 16f
    }

    private fun dp(value: Int): Int = (value * context.resources.displayMetrics.density + 0.5f).toInt()
}
