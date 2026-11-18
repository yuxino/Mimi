package app.yuxino.mimi.android.services.editor

import android.content.Intent
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.text.method.LinkMovementMethod
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.services.presentation.serviceProviderIcon
import app.yuxino.mimi.android.services.profiles.ServiceProfile
import app.yuxino.mimi.android.services.setup.speechProviderName
import app.yuxino.mimi.android.settings.SettingsStore
import app.yuxino.mimi.android.ui.HelpUi
import app.yuxino.mimi.android.ui.layout.ControlScrollView
import com.google.android.material.button.MaterialButton
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout

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
        HelpUi.dismiss(activity)
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
            .setView(ControlScrollView(activity).apply { addView(body) })
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
            activity.getString(R.string.profiles_list_help) + "\n\n" + activity.getString(R.string.services_key_note), 25f, "services-help")
        val header = object : LinearLayout(activity) {
            init { gravity = Gravity.CENTER_VERTICAL }
            override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
                val add = getChildAt(1)
                val title = (heading as android.view.ViewGroup).getChildAt(0) as TextView
                val available = View.MeasureSpec.getSize(widthMeasureSpec) - paddingLeft - paddingRight
                add?.measure(View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
                    View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED))
                val titleWidth = android.text.Layout.getDesiredWidth(title.text, title.paint) + dp(activity, 48)
                val stacked = add != null && View.MeasureSpec.getMode(widthMeasureSpec) != View.MeasureSpec.UNSPECIFIED &&
                    titleWidth + add.measuredWidth > available
                orientation = if (stacked) VERTICAL else HORIZONTAL
                (heading.layoutParams as LayoutParams).apply {
                    width = if (stacked) LayoutParams.MATCH_PARENT else 0
                    weight = if (stacked) 0f else 1f
                }
                (add?.layoutParams as? LayoutParams)?.gravity = if (stacked) Gravity.END else Gravity.CENTER_VERTICAL
                super.onMeasure(widthMeasureSpec, heightMeasureSpec)
            }
        }
        header.addView(heading, LinearLayout.LayoutParams(0, -2, 1f))
        container.addView(header, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(activity, 16) })
        if ((MimiService.isRunning || MimiService.isStopping)) {
            container.addView(LinearLayout(activity).apply {
                orientation = LinearLayout.VERTICAL
                tag = "service-session-guard"
                addView(label(activity, activity.getString(R.string.service_stop_first), 16f), LinearLayout.LayoutParams(-1, -2))
                addView(MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
                    tag = "service-stop"; setText(if (MimiService.isStopping) R.string.stopping_capture else R.string.stop_capture);
                    isEnabled = !MimiService.isStopping; isAllCaps = false; minimumHeight = dp(activity, 48)
                    setTextColor(ContextCompat.getColor(activity, R.color.mimi_text))
                    setOnClickListener {
                        isEnabled = false
                        try { activity.startService(MimiService.stopIntent(activity)) }
                        catch (_: RuntimeException) {
                            isEnabled = true
                            Toast.makeText(activity, R.string.service_stop_failed, Toast.LENGTH_LONG).show()
                        }
                    }
                }, LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END })
            }, LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(activity, 20) })
        }
        val profiles = try { SettingsStore.profiles(activity) } catch (_: Exception) {
            container.addView(label(activity, activity.getString(R.string.guide_storage_unavailable), 16f))
            container.addView(action(activity, R.string.profiles_retry, "profiles-retry") { renderList(activity, container) })
            return
        }
        if (profiles.isEmpty()) {
            container.addView(label(activity, activity.getString(R.string.profiles_empty), 16f),
                LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(activity, 16) })
            addProviderPicker(activity, container)
            return
        }
        header.addView(action(activity, R.string.profiles_add_short, "profiles-add") {
            val count = runCatching { SettingsStore.profiles(activity).size }.getOrNull()
            if (count == null) Toast.makeText(activity, R.string.guide_storage_unavailable, Toast.LENGTH_LONG).show()
            else if (count >= SettingsStore.MAX_SERVICE_PROFILES) Toast.makeText(activity, R.string.profiles_limit, Toast.LENGTH_SHORT).show()
            else showProviderPicker(activity)
        }.apply { isEnabled = !(MimiService.isRunning || MimiService.isStopping) && profiles.size < SettingsStore.MAX_SERVICE_PROFILES },
            LinearLayout.LayoutParams(-2, -2))
        val activeId = SettingsStore.activeProfileId(activity)
        val stacked = activity.resources.configuration.fontScale >= 1.5f
        for (profile in profiles) {
            val active = activeId == profile.id
            val loaded = runCatching { SettingsStore.profileConfigured(activity, profile.id) }
            val configured = loaded.getOrDefault(false)
            val row = LinearLayout(activity).apply {
                orientation = if (stacked) LinearLayout.VERTICAL else LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(activity, 78); tag = "profile-${profile.id}"
                if (stacked) setPadding(0, dp(activity, 8), 0, dp(activity, 8))
            }
            fun edit() { activity.startActivity(Intent(activity, ServiceSettingsActivity::class.java).putExtra("profileId", profile.id)) }
            val select = LinearLayout(activity).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                minimumHeight = dp(activity, 72); tag = "select-${profile.id}"
                isClickable = true; isFocusable = true; isEnabled = !(MimiService.isRunning || MimiService.isStopping)
                contentDescription = activity.getString(if (!configured) R.string.service_edit_named else R.string.profiles_use_named, profile.name)
                val typed = android.util.TypedValue()
                activity.theme.resolveAttribute(android.R.attr.selectableItemBackground, typed, true)
                setBackgroundResource(typed.resourceId)
                setOnClickListener {
                    if (MimiService.isRunning || MimiService.isStopping) return@setOnClickListener
                    if (!configured) edit()
                    else if (!active) mutate(activity, container) { SettingsStore.activateProfile(activity, profile.id) }
                }
            }
            androidx.core.view.ViewCompat.setAccessibilityDelegate(select, object : androidx.core.view.AccessibilityDelegateCompat() {
                override fun onInitializeAccessibilityNodeInfo(host: View, info: androidx.core.view.accessibility.AccessibilityNodeInfoCompat) {
                    super.onInitializeAccessibilityNodeInfo(host, info)
                    info.className = "android.widget.RadioButton"
                    info.isCheckable = configured
                    info.isChecked = active
                }
            })
            select.addView(androidx.appcompat.widget.AppCompatRadioButton(activity).apply {
                tag = "selection-${profile.id}"
                isChecked = active; isEnabled = select.isEnabled && configured
                isClickable = false; isFocusable = false
                minimumWidth = 0; minimumHeight = 0; setPadding(0, 0, 0, 0)
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }, LinearLayout.LayoutParams(dp(activity, 32), dp(activity, 40)))
            select.addView(mark(activity, profile.provider).apply { setPadding(dp(activity, 4), dp(activity, 4), dp(activity, 4), dp(activity, 4)) },
                LinearLayout.LayoutParams(dp(activity, 40), dp(activity, 40)).apply { marginEnd = dp(activity, 12) })
            val copy = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
            val name = label(activity, profile.name, 16f).apply {
                setTypeface(typeface, if (active) Typeface.BOLD else Typeface.NORMAL)
                maxLines = 2; ellipsize = android.text.TextUtils.TruncateAt.END
            }
            copy.addView(name, LinearLayout.LayoutParams(-1, -2))
            if (!configured) copy.addView(label(activity, activity.getString(
                if (loaded.isFailure) R.string.profiles_load_failed else R.string.service_missing
            ), 16f, true), LinearLayout.LayoutParams(-1, -2))
            select.addView(copy, LinearLayout.LayoutParams(0, -2, 1f))
            row.addView(select, if (stacked) LinearLayout.LayoutParams(-1, -2) else LinearLayout.LayoutParams(0, -2, 1f))
            row.addView(action(activity, R.string.service_edit, "configure-${profile.id}") { edit() }.apply {
                isEnabled = !(MimiService.isRunning || MimiService.isStopping)
                minWidth = dp(activity, 72); minimumWidth = dp(activity, 72)
                contentDescription = activity.getString(R.string.service_edit_named, profile.name)
            }, LinearLayout.LayoutParams(-2, -2).apply { if (stacked) gravity = Gravity.END })
            container.addView(row)
            container.addView(View(activity).apply { setBackgroundColor(ContextCompat.getColor(activity, R.color.mimi_border)) }, LinearLayout.LayoutParams(-1, dp(activity, 1)))
        }
    }

    private fun action(activity: AppCompatActivity, label: Int, tag: String, click: () -> Unit) =
        MaterialButton(activity, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
            setText(label); this.tag = tag; textSize = 16f; isAllCaps = false; isSingleLine = false
            minimumHeight = dp(activity, 48); minWidth = 0; minimumWidth = 0
            setTextColor(ContextCompat.getColor(activity, R.color.mimi_text))
            setOnClickListener {
                if ((MimiService.isRunning || MimiService.isStopping) && tag != "profiles-retry") Toast.makeText(activity, R.string.service_stop_first, Toast.LENGTH_SHORT).show()
                else click()
            }
        }

    private fun mark(activity: AppCompatActivity, provider: ServiceProvider) = android.widget.ImageView(activity).apply {
        setImageResource(serviceProviderIcon(provider)); scaleType = android.widget.ImageView.ScaleType.FIT_CENTER
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    private fun addProviderPicker(activity: AppCompatActivity, container: LinearLayout, onChoose: (() -> Unit)? = null) {
        for (provider in ServiceProvider.entries) {
            val choice = LinearLayout(activity).apply {
                tag = "add-${provider.id}"; gravity = Gravity.CENTER_VERTICAL; minimumHeight = dp(activity, 72)
                isClickable = true; isFocusable = true; isEnabled = !(MimiService.isRunning || MimiService.isStopping)
                val typed = android.util.TypedValue()
                activity.theme.resolveAttribute(android.R.attr.selectableItemBackground, typed, true)
                setBackgroundResource(typed.resourceId)
                contentDescription = speechProviderName(activity, provider)
            }
            choice.addView(mark(activity, provider).apply { setPadding(dp(activity, 4), dp(activity, 4), dp(activity, 4), dp(activity, 4)) },
                LinearLayout.LayoutParams(dp(activity, 40), dp(activity, 40)).apply { marginEnd = dp(activity, 12) })
            choice.addView(label(activity, speechProviderName(activity, provider), 16f), LinearLayout.LayoutParams(0, -2, 1f))
            choice.addView(android.widget.ImageView(activity).apply {
                setImageResource(R.drawable.ic_chevron); imageTintList = ContextCompat.getColorStateList(activity, R.color.mimi_muted)
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }, LinearLayout.LayoutParams(dp(activity, 20), dp(activity, 20)))
            choice.setOnClickListener {
                if ((MimiService.isRunning || MimiService.isStopping)) return@setOnClickListener
                onChoose?.invoke()
                activity.startActivity(Intent(activity, ServiceSettingsActivity::class.java).putExtra("provider", provider.id))
            }
            container.addView(choice, LinearLayout.LayoutParams(-1, -2))
        }
    }

    private fun showProviderPicker(activity: AppCompatActivity) {
        val body = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(activity, 24), 0, dp(activity, 24), dp(activity, 8)) }
        val dialog = com.google.android.material.dialog.MaterialAlertDialogBuilder(activity)
            .setTitle(R.string.profiles_add).setView(ControlScrollView(activity).apply { addView(body) })
            .setNegativeButton(android.R.string.cancel, null).create()
        addProviderPicker(activity, body) { dialog.dismiss() }
        dialog.show()
    }

    fun showProfileMenu(activity: AppCompatActivity, profile: ServiceProfile, onChanged: (deleted: Boolean) -> Unit) {
        val container = activity.window.decorView
        val deleteLabel = android.text.SpannableString(activity.getString(R.string.profiles_delete)).apply {
            setSpan(android.text.style.ForegroundColorSpan(com.google.android.material.color.MaterialColors.getColor(container, com.google.android.material.R.attr.colorError)),
                0, length, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
        com.google.android.material.dialog.MaterialAlertDialogBuilder(activity).setTitle(profile.name)
            .setItems(arrayOf<CharSequence>(activity.getString(R.string.profiles_duplicate), deleteLabel)) { _, index ->
                if ((MimiService.isRunning || MimiService.isStopping)) return@setItems
                if (index == 0) {
                    val count = runCatching { SettingsStore.profiles(activity).size }.getOrNull()
                    if (count == null) Toast.makeText(activity, R.string.guide_storage_unavailable, Toast.LENGTH_LONG).show()
                    else if (count >= SettingsStore.MAX_SERVICE_PROFILES) Toast.makeText(activity, R.string.profiles_limit, Toast.LENGTH_SHORT).show()
                    else if (runCatching { SettingsStore.duplicateProfile(activity, profile.id, copyName(activity, profile.name)) }.getOrNull() != null) onChanged(false)
                    else Toast.makeText(activity, R.string.service_save_failed, Toast.LENGTH_LONG).show()
                } else {
                    val confirm = com.google.android.material.dialog.MaterialAlertDialogBuilder(activity)
                        .setTitle(activity.getString(R.string.profiles_delete_title, profile.name))
                        .setMessage(R.string.profiles_delete_message).setNegativeButton(android.R.string.cancel, null)
                        .setPositiveButton(R.string.profiles_delete) { _, _ ->
                            if (MimiService.isRunning || MimiService.isStopping) Toast.makeText(activity, R.string.service_stop_first, Toast.LENGTH_SHORT).show()
                            else if (runCatching { SettingsStore.deleteProfile(activity, profile.id) }.getOrDefault(false)) onChanged(true)
                            else Toast.makeText(activity, R.string.service_save_failed, Toast.LENGTH_LONG).show()
                        }.show()
                    confirm.getButton(androidx.appcompat.app.AlertDialog.BUTTON_POSITIVE).setTextColor(
                        com.google.android.material.color.MaterialColors.getColor(container, com.google.android.material.R.attr.colorError))
                }
            }.show()
    }

    private fun copyName(activity: AppCompatActivity, name: String): String {
        val suffixLength = activity.getString(R.string.profiles_copy_name, "").length
        var base = name.take((80 - suffixLength).coerceAtLeast(0))
        if (base.lastOrNull()?.isHighSurrogate() == true) base = base.dropLast(1)
        return activity.getString(R.string.profiles_copy_name, base)
    }

    private fun mutate(activity: AppCompatActivity, container: LinearLayout, operation: () -> Boolean) {
        if ((MimiService.isRunning || MimiService.isStopping)) { Toast.makeText(activity, R.string.service_stop_first, Toast.LENGTH_SHORT).show(); return }
        if (runCatching(operation).getOrDefault(false)) renderList(activity, container)
        else Toast.makeText(activity, R.string.service_save_failed, Toast.LENGTH_LONG).show()
    }
}
