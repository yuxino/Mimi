package app.yuxino.mimi.android.sessions

import android.app.Application
import android.net.Uri
import android.os.Handler
import android.os.Looper
import androidx.lifecycle.AndroidViewModel
import app.yuxino.mimi.android.R
import java.util.concurrent.Executors

/** Retains one bounded page across rotation. Workers only hold the application context. */
internal class SavedSessionsViewModel(application: Application) : AndroidViewModel(application) {
    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor()
    private val sessions = SavedSessions(application.filesDir)
    private var cleared = false
    private class ExportRequest(val id: String, val markdown: Boolean, val destination: Uri)
    private var pendingExport: ExportRequest? = null
    var observer: (() -> Unit)? = null
    var items: List<SavedSessionItem> = emptyList(); private set
    var selected: SavedSessionItem? = null; private set
    var page: SavedSessionPage? = null; private set
    var selection: SavedSessionSelection = SavedSessionSelection(); private set
    val selecting get() = selection.active
    val selectedIds get() = selection.ids
    var busy = false; private set
    var failed = false; private set
    var notice: Int? = null
    var noticeArgs: List<Any> = emptyList()
    var initialized = false; private set

    private fun setNotice(resource: Int, vararg args: Any) {
        notice = resource
        noticeArgs = args.toList()
    }

    private fun availableIds(list: List<SavedSessionItem> = items): Set<String> = list.mapTo(mutableSetOf()) { it.id }

    private fun <T> run(work: () -> T, complete: (T) -> Unit, failure: (() -> Unit)? = null) {
        if (busy || cleared) return
        busy = true; failed = false; observer?.invoke()
        worker.execute {
            val result = runCatching(work)
            main.post {
                if (!cleared) {
                    busy = false
                    result.onSuccess(complete).onFailure {
                        if (failure == null) failed = true else failure()
                    }
                    observer?.invoke()
                    startPendingExport()
                }
            }
        }
    }
    fun refresh() {
        val item = selected
        if (item == null) run({ sessions.list() }, {
            items = it
            selection = selection.prune(availableIds(it))
            initialized = true
        })
        else loadPage(page?.page ?: 0)
    }
    fun open(item: SavedSessionItem) {
        if (busy) return
        selection = SavedSessionSelection()
        selected = item; page = null; loadPage(0)
    }
    fun list() {
        if (busy) return
        selected = null; page = null; refresh()
    }
    fun startSelecting() {
        if (busy || selected != null || items.isEmpty()) return
        selection = SavedSessionSelection(active = true)
        observer?.invoke()
    }
    fun stopSelecting() {
        if (busy || !selection.active) return
        selection = SavedSessionSelection()
        observer?.invoke()
    }
    fun toggleSelection(id: String) {
        if (busy || !selection.active) return
        selection = selection.toggle(id, availableIds())
        observer?.invoke()
    }
    fun toggleSelectAll() {
        if (busy || !selection.active) return
        selection = selection.toggleAll(availableIds())
        observer?.invoke()
    }
    fun loadPage(number: Int) {
        val id = selected?.id ?: return
        run({ sessions.page(id, number.coerceAtLeast(0)) }, { page = it })
    }
    fun delete(item: SavedSessionItem) = run({ sessions.delete(item.id); sessions.list() }, {
        items = it; selection = selection.prune(availableIds(it))
        if (selected?.id == item.id) { selected = null; page = null }
        setNotice(R.string.session_history_deleted)
    }, { setNotice(R.string.session_history_delete_error) })

    fun deleteMany(ids: List<String>) {
        if (ids.isEmpty()) return
        run({
            val report = sessions.deleteMany(ids)
            report to sessions.list()
        }, { (report, latest) ->
            items = latest
            val remaining = availableIds(latest)
            if (selected?.id in report.deleted) { selected = null; page = null }
            when {
                report.failed.isEmpty() -> {
                    selection = SavedSessionSelection()
                    if (report.deleted.isNotEmpty()) setNotice(R.string.session_history_deleted_many, report.deleted.size)
                }
                report.deleted.isNotEmpty() -> {
                    selection = if (remaining.isEmpty()) SavedSessionSelection()
                    else selection.copy(ids = report.failed.filterTo(mutableSetOf()) { it in remaining })
                    setNotice(R.string.session_history_delete_partial_error, report.deleted.size, report.failed.size)
                }
                else -> {
                    selection = selection.prune(remaining)
                    setNotice(R.string.session_history_delete_error)
                }
            }
        }, { setNotice(R.string.session_history_delete_error) })
    }

    fun export(id: String, markdown: Boolean, destination: Uri) {
        if (cleared) return
        // A document result can arrive while onStart's archive refresh is active.
        // Retain only one accepted destination; never replace it silently.
        if (pendingExport != null) {
            setNotice(R.string.session_history_export_error)
            observer?.invoke()
            return
        }
        val request = ExportRequest(id, markdown, destination)
        if (busy) pendingExport = request else startExport(request)
    }

    private fun startPendingExport() {
        if (busy || cleared) return
        val request = pendingExport ?: return
        pendingExport = null
        startExport(request)
    }

    private fun startExport(request: ExportRequest) = run({ copyExport(request) },
        { setNotice(R.string.session_history_exported) }, { setNotice(R.string.session_history_export_error) })

    private fun copyExport(request: ExportRequest) {
        val resolver = getApplication<Application>().contentResolver
        exportSavedSession({ sessions.export(request.id, request.markdown) }, { file ->
            checkNotNull(resolver.openOutputStream(request.destination, "wt")).use { output ->
                file.inputStream().use { input -> input.copyTo(output) }
            }
        })
    }

    override fun onCleared() {
        val acceptedExport = pendingExport
        cleared = true; pendingExport = null; observer = null; page = null; items = emptyList()
        // Back closes the reader, not an already accepted document result. Put
        // its bounded metadata on the same worker before shutdown, without any
        // completion callback retaining or reviving the closed Activity.
        try {
            if (acceptedExport != null) worker.execute { runCatching { copyExport(acceptedExport) } }
        } finally {
            // Both running and accepted copies still execute their file cleanup.
            worker.shutdown()
        }
    }
}
