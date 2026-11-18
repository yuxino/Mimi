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
    var busy = false; private set
    var failed = false; private set
    var notice: Int? = null
    var initialized = false; private set

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
        if (item == null) run({ sessions.list() }, { items = it; initialized = true })
        else loadPage(page?.page ?: 0)
    }
    fun open(item: SavedSessionItem) {
        if (busy) return
        selected = item; page = null; loadPage(0)
    }
    fun list() {
        if (busy) return
        selected = null; page = null; refresh()
    }
    fun loadPage(number: Int) {
        val id = selected?.id ?: return
        run({ sessions.page(id, number.coerceAtLeast(0)) }, { page = it })
    }
    fun delete(item: SavedSessionItem) = run({ sessions.delete(item.id); sessions.list() }, {
        items = it; if (selected?.id == item.id) { selected = null; page = null }
        notice = R.string.session_history_deleted
    }, { notice = R.string.session_history_delete_error })

    fun export(id: String, markdown: Boolean, destination: Uri) {
        if (cleared) return
        // A document result can arrive while onStart's archive refresh is active.
        // Retain only one accepted destination; never replace it silently.
        if (pendingExport != null) {
            notice = R.string.session_history_export_error
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
        { notice = R.string.session_history_exported }, { notice = R.string.session_history_export_error })

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
