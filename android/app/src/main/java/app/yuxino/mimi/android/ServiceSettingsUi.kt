package app.yuxino.mimi.android

import android.content.Intent
import android.graphics.Typeface
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import android.widget.ScrollView
import android.graphics.drawable.GradientDrawable
import android.text.method.LinkMovementMethod
import com.google.android.material.textfield.TextInputLayout
import com.google.android.material.textfield.TextInputEditText
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.capture.MimiService
import com.google.android.material.button.MaterialButton

internal object ServiceSettingsUi {
    fun addField(activity: AppCompatActivity, container: LinearLayout, box: TextInputLayout,
        edit: TextInputEditText, title: String, bottomMargin: Int) {
        edit.id = View.generateViewId()
        val large = activity.resources.configuration.fontScale >= 1.5f
        val group = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
        group.addView(label(activity, title, 16f).apply {
            tag = "input-label"; labelFor = edit.id
            visibility = if (large) View.VISIBLE else View.GONE
        }, LinearLayout.LayoutParams(-1, -2).apply { this.bottomMargin = dp(activity, 8) })
        box.isHintEnabled = !large
        if (large) edit.hint = null // TextInputLayout moves its disabled hint into the input.
        group.addView(box, LinearLayout.LayoutParams(-1, -2))
        container.addView(group, LinearLayout.LayoutParams(-1, -2).apply { this.bottomMargin = bottomMargin })
    }
    fun fieldLabel(box: TextInputLayout, title: String) {
        box.hint = title
        (box.parent as? View)?.findViewWithTag<TextView>("input-label")?.text = title
    }
    fun fieldVisible(box: TextInputLayout, visible: Boolean) {
        box.visibility = if (visible) View.VISIBLE else View.GONE
        (box.parent as? View)?.visibility = box.visibility
    }
    fun showHelp(activity: AppCompatActivity, title: String, message: CharSequence,
        links: List<Pair<Int, () -> Unit>> = emptyList()) {
        if (activity.isFinishing || activity.isDestroyed) return
        val host = activity.window.decorView
        if ((host.getTag(R.id.active_help_dialog) as? androidx.appcompat.app.AlertDialog)?.isShowing == true) return
        val body = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(activity, 24), dp(activity, 12), dp(activity, 24), dp(activity, 8))
        }
        body.addView(label(activity, "", 16f).apply {
            tag = "help-message"; text = message; movementMethod = LinkMovementMethod.getInstance()
        }, LinearLayout.LayoutParams(-1, -2))
        links.forEach { (label, action) ->
            body.addView(MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
                setText(label); isAllCaps = false; isSingleLine = false
                setTextColor(ContextCompat.getColor(activity, R.color.mimi_text))
                setPadding(paddingLeft, dp(activity, 12), paddingRight, dp(activity, 12))
                setOnClickListener { action() }
            }, LinearLayout.LayoutParams(-1, -2).apply { topMargin = dp(activity, 8) })
        }
        val surface = GradientDrawable().apply { cornerRadius = dp(activity, 28).toFloat() }
        val dialog = com.google.android.material.dialog.MaterialAlertDialogBuilder(activity)
            .setBackground(surface).setTitle(title)
            .setView(ScrollView(activity).apply { addView(body) })
            .setPositiveButton(android.R.string.ok, null).show()
        surface.setColor(ContextCompat.getColor(dialog.context, R.color.mimi_surface))
        host.setTag(R.id.active_help_dialog, dialog)
        val lifecycle = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP) dialog.dismiss() }
        activity.lifecycle.addObserver(lifecycle)
        dialog.setOnDismissListener {
            host.setTag(R.id.active_help_dialog, null)
            activity.lifecycle.removeObserver(lifecycle)
        }
    }
    fun dp(activity: AppCompatActivity, value: Int) = (value * activity.resources.displayMetrics.density).toInt()
    fun label(activity: AppCompatActivity, text: String, size: Float = 14f, secondary: Boolean = false) = TextView(activity).apply {
        this.text = text; textSize = size
        setTextColor(ContextCompat.getColor(activity, if(secondary) R.color.mimi_muted else R.color.mimi_text))
        setLineSpacing(dp(activity,3).toFloat(),1f)
    }
    fun renderList(activity: AppCompatActivity, container: LinearLayout) {
        container.removeAllViews()
        val heading = HelpUi.heading(activity, activity.getString(R.string.services_heading),
            activity.getString(R.string.services_hint) + "\n\n" + activity.getString(R.string.services_key_note), 25f, "services-help")
        container.addView(heading, LinearLayout.LayoutParams(-1,-2).apply { bottomMargin=dp(activity,16) })
        for(provider in ServiceProvider.entries) {
            val active = SettingsStore.provider(activity) == provider.id
            val configured = SettingsStore.isConfigured(activity,provider)
            val row = LinearLayout(activity).apply {
                orientation=LinearLayout.HORIZONTAL; gravity=Gravity.CENTER_VERTICAL
                minimumHeight=dp(activity,78); tag="service-${provider.id}"
            }
            val select = LinearLayout(activity).apply {
                orientation=LinearLayout.HORIZONTAL; gravity=Gravity.CENTER_VERTICAL
                minimumHeight=dp(activity,72); isClickable=true; isFocusable=true
                contentDescription="${providerTitle(activity, provider)}，${activity.getString(if(!configured) R.string.service_missing else if(active) R.string.service_active else R.string.service_configured)}"
            }
            val mark = android.widget.ImageView(activity).apply {
                setImageResource(serviceProviderIcon(provider))
                scaleType=android.widget.ImageView.ScaleType.FIT_CENTER
                setPadding(dp(activity,4),dp(activity,4),dp(activity,4),dp(activity,4))
                importantForAccessibility=View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            select.addView(mark, LinearLayout.LayoutParams(dp(activity,40),dp(activity,40)).apply { marginEnd=dp(activity,12) })
            val copy=LinearLayout(activity).apply { orientation=LinearLayout.VERTICAL }
            copy.addView(label(activity,providerTitle(activity, provider),16f).apply { setTypeface(typeface,if(active) Typeface.BOLD else Typeface.NORMAL) })
            copy.addView(label(activity, activity.getString(if(!configured) R.string.service_missing else if(active) R.string.service_active else R.string.service_configured),16f,true))
            select.addView(copy,LinearLayout.LayoutParams(0,-2,1f))
            fun edit() { activity.startActivity(Intent(activity,ServiceSettingsActivity::class.java).putExtra("provider",provider.id)) }
            select.setOnClickListener {
                when {
                    MimiService.isRunning -> Toast.makeText(activity,R.string.service_stop_first,Toast.LENGTH_SHORT).show()
                    !configured -> edit()
                    !active -> { SettingsStore.activateProvider(activity,provider); renderList(activity,container) }
                    else -> edit()
                }
            }
            row.addView(select,LinearLayout.LayoutParams(0,-2,1f))
            val edit=MaterialButton(activity,null,com.google.android.material.R.attr.borderlessButtonStyle).apply {
                text=activity.getString(if(configured) R.string.service_edit else R.string.service_configure)
                textSize=16f; isAllCaps=false; minWidth=dp(activity,72); minimumWidth=dp(activity,72); minimumHeight=dp(activity,48); tag="configure-${provider.id}"
                contentDescription=activity.getString(R.string.service_edit_named,providerTitle(activity, provider))
                setTextColor(ContextCompat.getColor(activity,R.color.mimi_text))
                setOnClickListener { if(MimiService.isRunning) Toast.makeText(activity,R.string.service_stop_first,Toast.LENGTH_SHORT).show() else edit() }
            }
            row.addView(edit,LinearLayout.LayoutParams(-2,-2))
            container.addView(row)
            container.addView(View(activity).apply { setBackgroundColor(ContextCompat.getColor(activity,R.color.mimi_border)) },LinearLayout.LayoutParams(-1,dp(activity,1)))
        }
    }
}
