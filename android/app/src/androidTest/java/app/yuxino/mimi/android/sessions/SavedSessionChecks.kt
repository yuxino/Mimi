package app.yuxino.mimi.android.sessions

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.content.IntentFilter
import android.content.SharedPreferences
import android.net.Uri
import android.os.Bundle
import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.inspector.WindowInspector
import android.widget.ScrollView
import android.widget.TextView
import androidx.lifecycle.ViewModelProvider
import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.settings.SettingsStore
import com.google.android.material.materialswitch.MaterialSwitch
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.ExecutorService
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

/** Synthetic archive only; preserves real settings and files, starts no capture or provider. */
internal class SavedSessionChecks(private val test: Instrumentation) {
    private val context get() = test.targetContext
    private var stage = "preconditions"
    private var listSelections = 0
    private var buttonSelections = 0
    private var lastTextClickable = false
    private var monitorHits = 0
    private var exportBusy = false
    private var exportExists = false
    private var exportBytes = 0L
    private var pageScrollSteps = 0
    private var pageScrollY = 0
    private var pageTap = "none"
    private var exportRefreshStarted = false
    private var exportRefreshReturned = false
    private var exportRefreshNotice = false
    private var exportRefreshClosed = false
    private var exportObserverCalls = 0
    private lateinit var activity: SavedSessionsActivity
    private lateinit var model: SavedSessionsViewModel
    private fun onUi(action: () -> Unit) {
        var failure: Throwable? = null
        test.runOnMainSync { try { action() } catch (error: Throwable) { failure = error } }
        failure?.let { throw it }
    }
    private fun await(condition: () -> Boolean) {
        val deadline = SystemClock.elapsedRealtime() + 10_000
        while (true) {
            var done = false
            onUi { done = condition() }
            if (done) break
            check(SystemClock.elapsedRealtime() < deadline) { "history_timeout" }
            SystemClock.sleep(25)
        }
        test.waitForIdleSync()
    }
    private fun view(tag: String): View = checkNotNull(activity.window.decorView.findViewWithTag<View>(tag))
    private fun texts(root: View): List<TextView> = buildList {
        if (root is TextView) add(root)
        if (root is ViewGroup) for (index in 0 until root.childCount) addAll(texts(root.getChildAt(index)))
    }
    private fun clickText(resource: Int) {
        onUi {
            val expected = activity.getString(resource)
            val item = checkNotNull(WindowInspector.getGlobalWindowViews().flatMap(::texts)
                .lastOrNull { it.text.toString() == expected && it.isShown }) { "dialog_action_missing" }
            lastTextClickable = item.isClickable
            val list = generateSequence<View>(item) { it.parent as? View }
                .filterIsInstance<android.widget.ListView>().firstOrNull()
            if (list != null) {
                val position = list.getPositionForView(item)
                check(position >= 0 && list.isEnabled && list.performItemClick(item, position, list.adapter.getItemId(position))) {
                    "dialog_list_selection_failed"
                }
                listSelections++
            } else {
                val clickable = generateSequence<View>(item) { it.parent as? View }.firstOrNull { it.isClickable }
                check(clickable != null && clickable.isEnabled && clickable.performClick()) { "dialog_button_selection_failed" }
                buttonSelections++
            }
        }
        test.waitForIdleSync()
    }
    /** Move through the native ScrollView and tap the visible target through the system input path. */
    private fun touchPageAction(tag: String, click: Boolean = true) {
        lateinit var scroll: ScrollView
        onUi { scroll = generateSequence<View>(view(tag)) { it.parent as? View }.filterIsInstance<ScrollView>().first() }
        test.setInTouchMode(true)
        val action = if (tag == "session-next") "next" else "previous"
        stage = "pagination_${action}_scroll"
        fun gesture(fromY: Float, toY: Float, x: Float) {
            val down = SystemClock.uptimeMillis()
            fun send(action: Int, y: Float) {
                val event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, x, y, 0).apply {
                    source = InputDevice.SOURCE_TOUCHSCREEN
                }
                try { check(test.uiAutomation.injectInputEvent(event, true)) { "history_input_rejected" } }
                finally { event.recycle() }
            }
            send(MotionEvent.ACTION_DOWN, fromY)
            if (fromY != toY) {
                for (step in 1..10) {
                    SystemClock.sleep(25)
                    send(MotionEvent.ACTION_MOVE, fromY + (toY - fromY) * step / 10)
                }
                // End without a fling; layout assertions must address settled native geometry.
                SystemClock.sleep(125); send(MotionEvent.ACTION_MOVE, toY)
            }
            if (fromY == toY) SystemClock.sleep(100)
            send(MotionEvent.ACTION_UP, toY)
            test.waitForIdleSync()
        }
        repeat(30) {
            var bottom = false
            val origin = IntArray(2)
            var x = 0f; var fromY = 0f; var toY = 0f
            onUi {
                bottom = !scroll.canScrollVertically(1)
                scroll.getLocationOnScreen(origin)
                x = origin[0] + scroll.width * .5f
                fromY = origin[1] + scroll.height * .8f
                toY = origin[1] + scroll.height * .2f
            }
            if (bottom) {
                // A native ScrollView can be at its final offset while a spring-back is
                // still consuming the next DOWN. Require stable geometry for several
                // frames before deriving the actual screen touch point.
                var stableSince = SystemClock.elapsedRealtime()
                var previous = -1
                val deadline = SystemClock.elapsedRealtime() + 5_000
                while (true) {
                    var current = 0; var laidOut = false
                    onUi { current = scroll.scrollY; laidOut = !scroll.isLayoutRequested && !view(tag).isLayoutRequested }
                    if (current != previous || !laidOut) stableSince = SystemClock.elapsedRealtime()
                    previous = current
                    check(SystemClock.elapsedRealtime() < deadline) { "history_scroll_not_settled" }
                    if (SystemClock.elapsedRealtime() - stableSince >= 500) break
                    SystemClock.sleep(50)
                }
                if (!click) return
                stage = "pagination_${action}_tap"
                capture("pagination-$action-before")
                val before = model.page?.page
                val point = IntArray(2)
                var tapX = 0f; var tapY = 0f
                onUi {
                    val button = view(tag)
                    val visible = android.graphics.Rect()
                    check(button.isEnabled && button.getGlobalVisibleRect(visible) &&
                        visible.width() == button.width && visible.height() == button.height) { "history_page_action_clipped" }
                    button.getLocationOnScreen(point)
                    tapX = point[0] + button.width / 2f; tapY = point[1] + button.height / 2f
                    pageScrollY = scroll.scrollY
                    pageTap = "${point[0]},${point[1]},${button.width},${button.height}"
                }
                gesture(tapY, tapY, tapX)
                stage = "pagination_${action}_dispatch"
                await { model.busy || model.page?.page != before }
                return
            }
            gesture(fromY, toY, x)
            pageScrollSteps++
        }
        error("history_page_action_unreachable")
    }
    private fun checkPageOpening(index: Int) {
        lateinit var source: TextView
        lateinit var translation: TextView
        lateinit var scroll: ScrollView
        onUi {
            source = checkNotNull(texts(activity.window.decorView).firstOrNull { it.text.toString() == "Synthetic source $index" })
            translation = checkNotNull(texts(activity.window.decorView).firstOrNull { it.text.toString() == "Synthetic translation $index" })
            scroll = generateSequence<View>(source) { it.parent as? View }.filterIsInstance<ScrollView>().first()
        }
        await { !scroll.isLayoutRequested && source.height > 0 && translation.height > 0 }
        onUi {
            check(scroll.scrollY == 0) { "history_new_page_not_at_top" }
            for (line in listOf(source, translation)) {
                val visible = android.graphics.Rect()
                check(line.getGlobalVisibleRect(visible) && visible.height() == line.height && visible.width() == line.width) {
                    "history_new_page_first_entry_clipped"
                }
            }
        }
    }
    private fun capture(name: String) {
        await { activity.hasWindowFocus() && activity.window.decorView.isShown }
        SystemClock.sleep(300)
        val bitmap = checkNotNull(test.uiAutomation.takeScreenshot())
        try {
            File(context.cacheDir, "mimi-saved-sessions-$name.png").outputStream().use {
                check(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it))
            }
        } finally { bitmap.recycle() }
    }
    /** Return through the real document contract while a genuine archive refresh is active. */
    private fun exportReturningDuringRefresh(id: String, outputs: MutableList<File>, closeAfterReturn: Boolean) {
        val output = File(context.cacheDir, "history-refresh-export-$id.txt")
        outputs += output
        val exports = File(context.filesDir, "session-exports")
        val existingExports = exports.listFiles().orEmpty().map { it.name }.toSet()
        // Hold only worker completion: busy is set by the same refresh() used in onStart.
        // The SAF callback remains the real ActivityResultRegistry -> model.export path.
        val worker = SavedSessionsViewModel::class.java.getDeclaredField("worker").apply { isAccessible = true }
            .get(model) as ExecutorService
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val previousObserver = model.observer
        onUi { model.observer = {
            exportObserverCalls++
            if (model.notice == R.string.session_history_exported) exportRefreshNotice = true
            previousObserver?.invoke()
        } }
        val monitor = object : Instrumentation.ActivityMonitor() {
            override fun onStartActivity(intent: Intent): Instrumentation.ActivityResult? {
                if (intent.action != Intent.ACTION_CREATE_DOCUMENT) return null
                check(android.os.Looper.myLooper() == android.os.Looper.getMainLooper())
                model.refresh()
                exportRefreshStarted = model.busy
                return Instrumentation.ActivityResult(Activity.RESULT_OK, Intent().setData(Uri.fromFile(output)))
            }
        }
        test.addMonitor(monitor)
        try {
            worker.execute { entered.countDown(); release.await(15, TimeUnit.SECONDS) }
            check(entered.await(5, TimeUnit.SECONDS)) { "history_refresh_barrier_not_started" }
            stage = "export_return_refresh_launch"
            onUi { check(view("session-export-$id").performClick()) { "export_action_failed" } }
            clickText(R.string.session_history_txt)
            stage = "export_return_refresh_callback"
            val pendingId = SavedSessionsActivity::class.java.getDeclaredField("pendingId").apply { isAccessible = true }
            await { monitor.hits == 1 && pendingId.get(activity) == null }
            onUi {
                exportRefreshReturned = true
                check(exportRefreshStarted && model.busy) { "history_refresh_not_active_at_return" }
            }
            val callsBeforeClose = exportObserverCalls
            if (closeAfterReturn) {
                stage = "export_return_refresh_close"
                onUi { activity.finish() }
                await { activity.isDestroyed && model.observer == null }
                exportRefreshClosed = true
            }
            release.countDown()
            stage = if (closeAfterReturn) "export_return_refresh_close_write" else "export_return_refresh_write"
            await { (if (closeAfterReturn) worker.isTerminated else !model.busy) && output.isFile && output.length() > 0 }
            check("Synthetic source 1" in output.readText() && "Synthetic translation 60" in output.readText()) {
                "export_bilingual_content_missing"
            }
            if (closeAfterReturn) {
                check(exportObserverCalls == callsBeforeClose && !exportRefreshNotice) { "export_closed_ui_notified" }
            } else check(exportRefreshNotice) { "export_success_feedback_missing" }
            check(exports.listFiles().orEmpty().map { it.name }.toSet() == existingExports) { "export_temporary_not_cleaned" }
        } finally {
            release.countDown()
            test.removeMonitor(monitor)
            if (activity.isFinishing || activity.isDestroyed) await { worker.isTerminated }
            else {
                await { !model.busy }
                onUi { model.observer = previousObserver }
            }
        }
    }
    fun run(exportRefreshRace: Boolean = false, closeAfterExportReturn: Boolean = false) {
        val id = UUID.randomUUID().toString()
        val history = File(context.filesDir, "session-history")
        val existed = history.exists()
        val file = File(history, "$id.json")
        val outputs = mutableListOf<File>()
        var prefs: SharedPreferences? = null
        var previous: Boolean? = null
        var failure: String? = null
        var checks = 0
        var listScreenshot = false
        try {
            check(!MimiService.isRunning && !MimiService.isStopping) { "active_session" }
            // Inspect and restore only the opt-in slot, never credentials or other settings.
            SettingsStore.retainSessionHistory(context)
            prefs = SettingsStore::class.java.getDeclaredField("prefs").apply { isAccessible = true }.get(null) as SharedPreferences
            previous = if (prefs.contains("retain_session_history")) prefs.getBoolean("retain_session_history", false) else null
            check(prefs.edit().remove("retain_session_history").commit())
            stage = "default_off"
            check(!SettingsStore.retainSessionHistory(context)); checks++
            check(history.mkdirs() || history.isDirectory)
            android.system.Os.chmod(history.absolutePath, 448)
            val fixture = JSONObject().put("version", 1).put("id", id).put("startedAtMs", 1000L)
                .put("endedAtMs", 61_000L).put("limited", false).put("entries", JSONArray().apply {
                    for (number in 1..60) put(JSONObject().put("source", "Synthetic source $number")
                        .put("translation", "Synthetic translation $number").put("createdAt", 1000L + number * 1000)
                        .put("audioSource", if (number % 2 == 0) "microphone" else "system"))
                }).toString()
            file.writeText(fixture); android.system.Os.chmod(file.absolutePath, 384)
            activity = test.startActivitySync(Intent(context, SavedSessionsActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as SavedSessionsActivity
            onUi { model = ViewModelProvider(activity)[SavedSessionsViewModel::class.java] }
            await { !model.busy && model.initialized }
            // A list screenshot must never include an existing user's archive metadata.
            if (model.items.all { it.id == id }) { capture("list"); listScreenshot = true }
            stage = "native_list_and_toggle"
            onUi {
                val controls = SessionStorageControls(activity)
                check(!controls.findViewWithTag<MaterialSwitch>("session-history-save").isChecked)
                check(view("session-view-$id").performClick())
                check(model.busy)
                check(!view("session-export-$id").isEnabled)
            }
            await { !model.busy && model.page != null }
            capture("detail")
            if (exportRefreshRace) {
                exportReturningDuringRefresh(id, outputs, closeAfterExportReturn)
                checks++
            } else {
                stage = "latest_and_pagination"
                check(model.page!!.entries.first().index == 60)
                touchPageAction("session-next")
                stage = "pagination_next_loaded"
                await { !model.busy && model.page?.page == 1 }
                check(model.page!!.entries.last().index == 1)
                stage = "pagination_next_viewport"
                capture("pagination-next")
                checkPageOpening(30)
                touchPageAction("session-previous", click = false)
                stage = "pagination_refresh_keeps_reading_position"
                onUi { model.refresh() }
                await { !model.busy && model.page?.page == 1 }
                test.waitForIdleSync()
                onUi {
                    val scroll = generateSequence<View>(view("session-previous")) { it.parent as? View }
                        .filterIsInstance<ScrollView>().first()
                    check(scroll.scrollY > 0) { "history_refresh_reset_reading_position" }
                }
                onUi { check(!view("session-next").isEnabled); check(view("session-previous").isEnabled) }
                touchPageAction("session-previous")
                stage = "pagination_previous_loaded"
                await { !model.busy && model.page?.page == 0 }
                stage = "pagination_previous_viewport"
                checkPageOpening(60)
                check(model.page!!.entries.first().index == 60); checks++
                stage = "storage_error_and_retry"
                check(file.delete())
                onUi { model.refresh() }
                await { !model.busy }
                check(model.failed)
                file.writeText(fixture); android.system.Os.chmod(file.absolutePath, 384)
                onUi { view("session-retry").performClick() }
                await { !model.busy }
                check(!model.failed); checks++
                for (markdown in listOf(false, true)) {
                    stage = if (markdown) "markdown_document_result" else "text_document_result"
                    val output = File(context.cacheDir, "history-export-$id.${if (markdown) "md" else "txt"}")
                    outputs += output
                    val exports = File(context.filesDir, "session-exports")
                    val existingExports = exports.listFiles().orEmpty().map { it.name }.toSet()
                    val filter = IntentFilter(Intent.ACTION_CREATE_DOCUMENT).apply { addCategory(Intent.CATEGORY_OPENABLE); addDataType(if (markdown) "text/markdown" else "text/plain") }
                    // Exercise the real ActivityResult + ContentResolver stream path;
                    // the system picker's returned destination is synthetic, not a claim about provider UI.
                    val monitor = test.addMonitor(filter, Instrumentation.ActivityResult(Activity.RESULT_OK,
                        Intent().setData(Uri.fromFile(output))), true)
                    try {
                        onUi { check(view("session-export-$id").performClick()) { "export_action_failed" } }
                        clickText(if (markdown) R.string.session_history_markdown else R.string.session_history_txt)
                        stage = if (markdown) "markdown_document_interception" else "text_document_interception"
                        await { monitor.hits >= 1 }
                        check(monitor.hits == 1) { "duplicate_document_launch" }
                        stage = if (markdown) "markdown_document_write" else "text_document_write"
                        await { !model.busy && output.exists() && output.length() > 0 }
                        stage = if (markdown) "markdown_document_content" else "text_document_content"
                        val result = output.readText()
                        check("Synthetic source 1" in result && "Synthetic translation 60" in result) { "export_bilingual_content_missing" }
                        check("Microphone" in result && "System" in result) { "export_source_labels_missing" }
                        check(exports.listFiles().orEmpty().map { it.name }.toSet() == existingExports) { "export_temporary_not_cleaned" }
                        checks++
                    } finally {
                        monitorHits = monitor.hits
                        onUi { exportBusy = model.busy }
                        exportExists = output.exists(); exportBytes = output.length()
                        test.removeMonitor(monitor)
                    }
                }
                stage = "export_error_preserves_existing_document"
                val existingDocument = File(context.cacheDir, "history-existing-$id.txt").apply {
                    writeText("Synthetic existing document")
                }
                outputs += existingDocument
                onUi { model.export(UUID.randomUUID().toString(), false, Uri.fromFile(existingDocument)) }
                await { !model.busy }
                check(existingDocument.readText() == "Synthetic existing document") { "existing_document_changed_after_format_failure" }
                checks++
                stage = "delete_confirmation"
                onUi { view("session-more-$id").performClick() }
                clickText(R.string.session_history_delete)
                check(file.exists())
                clickText(android.R.string.cancel)
                check(file.exists())
                onUi { view("session-more-$id").performClick() }
                clickText(R.string.session_history_delete)
                clickText(R.string.session_history_delete)
                await { !model.busy && !file.exists() }
                check(model.items.none { it.id == id }); checks++
            }
        } catch (error: Throwable) {
            val line = error.stackTrace.firstOrNull { it.className.startsWith(SavedSessionChecks::class.java.name) }?.lineNumber
            val label = error.message?.takeIf { it.matches(Regex("[a-z_]+")) } ?: "unexpected"
            failure = "$stage:${error.javaClass.simpleName}:line=${line ?: -1}:$label"
            // Only the synthetic detail can be captured; never expose a pre-existing archive list.
            runCatching {
                if (::model.isInitialized && model.selected?.id == id && ::activity.isInitialized && activity.hasWindowFocus()) {
                    capture("failure-$stage")
                }
            }
        } finally {
            runCatching { if (::activity.isInitialized) onUi { activity.finish() } }
            outputs.forEach { it.delete() }; file.delete()
            if (!existed && history.listFiles().orEmpty().isEmpty()) history.delete()
            prefs?.let { settings ->
                val edit = settings.edit()
                if (previous == null) edit.remove("retain_session_history") else edit.putBoolean("retain_session_history", previous!!)
                if (!edit.commit()) failure = failure ?: "restore_preference"
            }
        }
        test.finish(if (failure == null) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply {
            putString("stream", "SAVED_SESSIONS_RESULT passed=${failure == null}; checks=$checks; listScreenshot=$listScreenshot; listSelections=$listSelections; buttonSelections=$buttonSelections; lastTextClickable=$lastTextClickable; monitorHits=$monitorHits; exportBusy=$exportBusy; exportExists=$exportExists; exportBytes=$exportBytes; pageScrollSteps=$pageScrollSteps; pageScrollY=$pageScrollY; pageTap=$pageTap; exportRefreshStarted=$exportRefreshStarted; exportRefreshReturned=$exportRefreshReturned; exportRefreshNotice=$exportRefreshNotice; exportRefreshClosed=$exportRefreshClosed; exportObserverCalls=$exportObserverCalls; page=${if (::model.isInitialized) model.page?.page else null}; failure=${failure ?: "none"}; synthetic archive and document result; no provider, capture or real session changes.\n")
        })
    }
}
