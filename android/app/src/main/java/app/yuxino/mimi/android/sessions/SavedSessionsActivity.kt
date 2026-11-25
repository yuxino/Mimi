package app.yuxino.mimi.android.sessions

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.core.view.doOnNextLayout
import androidx.lifecycle.ViewModelProvider
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.services.editor.ServiceSettingsUi
import app.yuxino.mimi.android.ui.HelpUi
import app.yuxino.mimi.android.ui.layout.ControlScrollView
import app.yuxino.mimi.android.ui.layout.applySystemBarInsets
import com.google.android.material.button.MaterialButton
import com.google.android.material.checkbox.MaterialCheckBox
import com.google.android.material.color.MaterialColors
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import java.text.DateFormat
import java.util.Date

/** Native navigation and SAF only; archive contents and policy come from shared Rust. */
class SavedSessionsActivity : AppCompatActivity() {
    private lateinit var model: SavedSessionsViewModel
    private lateinit var body: LinearLayout
    private lateinit var scroll: ControlScrollView
    private var displayedPage: Pair<String, Int>? = null
    private var pendingId: String? = null
    private var pendingMarkdown = false
    private val document = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val id = pendingId; pendingId = null
        val uri = result.data?.data
        if (result.resultCode == Activity.RESULT_OK && uri != null && id != null) model.export(id, pendingMarkdown, uri)
    }
    private fun dp(value: Int) = ServiceSettingsUi.dp(this, value)
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        model = ViewModelProvider(this)[SavedSessionsViewModel::class.java]
        pendingId = savedInstanceState?.getString("exportId")
        pendingMarkdown = savedInstanceState?.getBoolean("exportMarkdown") ?: false
        val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        val header = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL; setPadding(dp(12), dp(8), dp(12), dp(8)) }
        header.addView(ImageButton(this).apply {
            id = R.id.back; setImageResource(R.drawable.ic_back); setBackgroundColor(android.graphics.Color.TRANSPARENT)
            imageTintList = ContextCompat.getColorStateList(context, R.color.mimi_text)
            contentDescription = getString(R.string.settings_back); setOnClickListener { goBack() }
        }, LinearLayout.LayoutParams(dp(48), dp(48)))
        header.addView(HelpUi.heading(this, getString(R.string.session_history_title), getString(R.string.session_history_help), 21f,
            "saved-sessions-help"), LinearLayout.LayoutParams(0, -2, 1f))
        root.addView(header)
        body = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(24), dp(8), dp(24), dp(24)) }
        scroll = ControlScrollView(this).apply { addView(body) }
        root.addView(scroll, LinearLayout.LayoutParams(-1, 0, 1f))
        setContentView(root); applySystemBarInsets()
        onBackPressedDispatcher.addCallback(this, object : androidx.activity.OnBackPressedCallback(true) {
            override fun handleOnBackPressed() { goBack() }
        })
    }
    override fun onStart() {
        super.onStart()
        model.observer = ::render
        if (!model.initialized && !model.busy) model.refresh() else render()
    }
    override fun onStop() { model.observer = null; HelpUi.dismiss(this); super.onStop() }
    override fun onSaveInstanceState(state: Bundle) {
        state.putString("exportId", pendingId); state.putBoolean("exportMarkdown", pendingMarkdown)
        super.onSaveInstanceState(state)
    }
    private fun goBack() {
        when {
            model.selecting && !model.busy -> model.stopSelecting()
            model.selected != null && !model.busy -> model.list()
            else -> finish()
        }
    }
    private fun label(text: String, muted: Boolean = false, selectable: Boolean = false) = ServiceSettingsUi.label(this, text, 16f, muted).apply {
        setTextIsSelectable(selectable)
    }
    private fun button(title: String, tag: String, destructive: Boolean = false, action: () -> Unit) = MaterialButton(this, null,
        com.google.android.material.R.attr.borderlessButtonStyle).apply {
        text = title; this.tag = tag; isAllCaps = false; isEnabled = !model.busy; minHeight = dp(48)
        if (destructive) setTextColor(MaterialColors.getColor(this, com.google.android.material.R.attr.colorError))
        setOnClickListener { action() }
    }
    private fun button(title: Int, tag: String, destructive: Boolean = false, action: () -> Unit) =
        button(getString(title), tag, destructive, action)
    private fun actionRow(vararg buttons: View): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.END or Gravity.CENTER_VERTICAL
        buttons.forEach { addView(it) }
        doOnNextLayout {
            val total = buttons.sumOf { it.measuredWidth }
            if (total > width && orientation != LinearLayout.VERTICAL) {
                orientation = LinearLayout.VERTICAL
                gravity = Gravity.END
            }
        }
    }
    private fun actions(item: SavedSessionItem, showOpen: Boolean): LinearLayout = LinearLayout(this).apply {
        gravity = Gravity.END
        if (showOpen) addView(button(R.string.session_history_view, "session-view-${item.id}") { model.open(item) })
        addView(button(R.string.session_history_export, "session-export-${item.id}") { export(item) })
        addView(ImageButton(this@SavedSessionsActivity).apply {
            contentDescription = getString(R.string.navigation_more); tag = "session-more-${item.id}"
            setImageResource(androidx.appcompat.R.drawable.abc_ic_menu_overflow_material)
            imageTintList = ContextCompat.getColorStateList(context, R.color.mimi_muted)
            setBackgroundColor(android.graphics.Color.TRANSPARENT); isEnabled = !model.busy
            setOnClickListener { view -> androidx.appcompat.widget.PopupMenu(this@SavedSessionsActivity, view).apply {
                menu.add(R.string.session_history_delete).setOnMenuItemClickListener { confirmDelete(item); true }; show()
            } }
        }, LinearLayout.LayoutParams(dp(48), dp(48)))
    }
    private fun render() {
        val nextPage = model.selected?.id?.let { id -> model.page?.page?.let { id to it } }
        val openingPage = nextPage != null && displayedPage != nextPage
        displayedPage = nextPage
        body.removeAllViews()
        model.notice?.let { resource ->
            val args = model.noticeArgs
            model.notice = null
            model.noticeArgs = emptyList()
            val message = if (args.isEmpty()) getString(resource) else getString(resource, *args.toTypedArray())
            Toast.makeText(this, message, Toast.LENGTH_LONG).show()
        }
        if (model.busy) body.addView(label(getString(R.string.session_history_working)).apply { accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE })
        if (model.failed) {
            body.addView(label(getString(R.string.session_history_unavailable)))
            body.addView(button(R.string.session_history_retry, "session-retry") { model.refresh() })
        }
        val selected = model.selected
        if (selected == null) {
            if (model.initialized && model.items.isEmpty() && !model.failed && !model.busy) body.addView(label(getString(R.string.session_history_empty)))
            if (model.items.isNotEmpty()) {
                if (model.selecting) {
                    val allSelected = model.items.all { it.id in model.selectedIds }
                    body.addView(actionRow(
                        button(if (allSelected) R.string.session_history_deselect_all else R.string.session_history_select_all, "sessions-select-all") { model.toggleSelectAll() },
                        button(R.string.session_history_done, "sessions-select-done") { model.stopSelecting() },
                    ))
                    val count = model.selectedIds.size
                    body.addView(label(getString(R.string.session_history_selected_count, count), true))
                    val deleteLabel = if (count > 0) getString(R.string.session_history_delete_selected, count) else getString(R.string.session_history_delete)
                    body.addView(actionRow(
                        button(deleteLabel, "sessions-delete-selected", destructive = true) {
                            confirmBatchDelete(model.items.map { it.id }.filter { it in model.selectedIds }, clearAll = false)
                        }.apply { isEnabled = !model.busy && count > 0 },
                    ))
                    divider()
                } else {
                    body.addView(actionRow(
                        button(R.string.session_history_select, "sessions-select") { model.startSelecting() },
                        button(R.string.session_history_clear_all, "sessions-clear-all", destructive = true) {
                            confirmBatchDelete(model.items.map { it.id }, clearAll = true)
                        },
                    ))
                    divider()
                }
            }
            model.items.forEach { item ->
                val dateText = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(item.startedAtMs))
                if (model.selecting) {
                    val checked = item.id in model.selectedIds
                    val row = LinearLayout(this).apply {
                        orientation = LinearLayout.HORIZONTAL
                        gravity = Gravity.CENTER_VERTICAL
                        minimumHeight = dp(48)
                        isClickable = !model.busy
                        tag = "session-select-row-${item.id}"
                        setOnClickListener { model.toggleSelection(item.id) }
                    }
                    val checkBox = MaterialCheckBox(this).apply {
                        tag = "session-check-${item.id}"
                        isChecked = checked
                        isEnabled = !model.busy
                        contentDescription = dateText
                        minimumWidth = dp(48)
                        minimumHeight = dp(48)
                        setOnClickListener { model.toggleSelection(item.id) }
                    }
                    val details = LinearLayout(this).apply {
                        orientation = LinearLayout.VERTICAL
                        addView(label(dateText))
                        addView(label(getString(R.string.session_history_count, item.count), true))
                        if (item.limited) addView(label(getString(R.string.session_history_limit)))
                    }
                    row.addView(checkBox)
                    row.addView(details, LinearLayout.LayoutParams(0, -2, 1f))
                    body.addView(row); divider()
                } else {
                    body.addView(label(dateText))
                    body.addView(label(getString(R.string.session_history_count, item.count), true))
                    if (item.limited) body.addView(label(getString(R.string.session_history_limit)))
                    body.addView(actions(item, true)); divider()
                }
            }
            body.addView(button(R.string.session_history_refresh, "sessions-refresh") { model.refresh() },
                LinearLayout.LayoutParams(-2, -2).apply { gravity = Gravity.END })
        } else {
            body.addView(label(DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(selected.startedAtMs))))
            body.addView(actions(selected, false))
            model.page?.let { page ->
                page.entries.forEach { entry ->
                    val source = getString(if (entry.audioSource == "microphone") R.string.capture_input_microphone else R.string.capture_input_system)
                    body.addView(label(getString(R.string.session_history_caption_meta, source,
                        DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(entry.createdAtMs))), true))
                    if (entry.source.isNotBlank()) body.addView(label(entry.source, selectable = true))
                    if (entry.translation.isNotBlank()) body.addView(label(entry.translation, selectable = true))
                    divider()
                }
                if (page.entries.isEmpty()) body.addView(label(getString(R.string.session_history_empty)))
                val navigation = LinearLayout(this).apply { gravity = Gravity.END or Gravity.CENTER_VERTICAL }
                navigation.addView(button(R.string.session_history_previous, "session-previous") { model.loadPage(page.page - 1) }.apply { isEnabled = !model.busy && page.page > 0 })
                navigation.addView(label(getString(R.string.session_history_page, page.page + 1)))
                navigation.addView(button(R.string.session_history_next, "session-next") { model.loadPage(page.page + 1) }.apply { isEnabled = !model.busy && page.hasNext })
                body.addView(navigation)
            }
        }
        if (openingPage) scroll.doOnNextLayout {
            if (!isFinishing && !isDestroyed && displayedPage == nextPage) scroll.scrollTo(0, 0)
        }
    }
    private fun divider() { body.addView(View(this).apply { setBackgroundColor(ContextCompat.getColor(context, R.color.mimi_border)) },
        LinearLayout.LayoutParams(-1, dp(1)).apply { topMargin = dp(12); bottomMargin = dp(16) }) }
    private fun confirmDelete(item: SavedSessionItem) {
        MaterialAlertDialogBuilder(this).setTitle(R.string.session_history_delete).setMessage(R.string.session_history_delete_confirm)
            .setNegativeButton(android.R.string.cancel, null).setPositiveButton(R.string.session_history_delete) { _, _ -> model.delete(item) }
            .create().apply { setOnShowListener { getButton(android.app.AlertDialog.BUTTON_POSITIVE).setTextColor(MaterialColors.getColor(body, com.google.android.material.R.attr.colorError)) }; show() }
    }
    private fun confirmBatchDelete(ids: List<String>, clearAll: Boolean) {
        if (ids.isEmpty() || model.busy) return
        val title = if (clearAll) getString(R.string.session_history_clear_all) else getString(R.string.session_history_delete_selected, ids.size)
        val message = getString(
            if (clearAll) R.string.session_history_clear_all_confirm else R.string.session_history_delete_selected_confirm,
            ids.size,
        )
        MaterialAlertDialogBuilder(this).setTitle(title).setMessage(message)
            .setNegativeButton(android.R.string.cancel, null)
            .setPositiveButton(if (clearAll) R.string.session_history_clear_all else R.string.session_history_delete) { _, _ -> model.deleteMany(ids) }
            .create().apply { setOnShowListener { getButton(android.app.AlertDialog.BUTTON_POSITIVE).setTextColor(MaterialColors.getColor(body, com.google.android.material.R.attr.colorError)) }; show() }
    }
    private fun export(item: SavedSessionItem) {
        MaterialAlertDialogBuilder(this).setTitle(R.string.session_history_export).setItems(arrayOf(getString(R.string.session_history_txt), getString(R.string.session_history_markdown))) { _, format ->
            pendingId = item.id; pendingMarkdown = format == 1
            runCatching { document.launch(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE); type = if (pendingMarkdown) "text/markdown" else "text/plain"
                putExtra(Intent.EXTRA_TITLE, "mimi-${item.id}.${if (pendingMarkdown) "md" else "txt"}")
            }) }.onFailure { pendingId = null; Toast.makeText(this, R.string.session_history_export_error, Toast.LENGTH_LONG).show() }
        }.show()
    }
}
