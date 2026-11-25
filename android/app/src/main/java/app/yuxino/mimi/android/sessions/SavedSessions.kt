package app.yuxino.mimi.android.sessions

import app.yuxino.mimi.android.provider.NativeRuntimeConfiguration
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Platform adapter only: Rust owns retention, parsing, pagination and formatting. */
internal class SavedSessions(private val filesDirectory: File) {
    private val directory = File(filesDirectory, "session-history")
    private val exports = File(filesDirectory, "session-exports")
    private fun request(operation: String, id: String? = null) = JSONObject()
        .put("operation", operation).put("directory", directory.absolutePath).apply { id?.let { put("id", it) } }
    fun list(): List<SavedSessionItem> {
        val items = NativeRuntimeConfiguration.exchange(request("history_list")).getJSONArray("items")
        return List(items.length()) { index ->
            val item = items.getJSONObject(index)
            SavedSessionItem(item.getString("id"), item.getLong("startedAtMs"), item.getInt("count"), item.getBoolean("limited"))
        }
    }
    fun page(id: String, page: Int): SavedSessionPage {
        val response = NativeRuntimeConfiguration.exchange(request("history_page", id).put("page", page).put("query", ""))
        val entries = response.getJSONArray("entries")
        return SavedSessionPage(response.getInt("total"), response.getInt("page"), List(entries.length()) { index ->
            val entry = entries.getJSONObject(index)
            SavedSessionCaption(entry.getString("audioSource"), entry.getString("source"), entry.getString("translation"), entry.getLong("createdAtMs"), entry.getInt("index"))
        })
    }
    fun delete(id: String) { NativeRuntimeConfiguration.exchange(request("history_delete", id)) }
    fun deleteMany(ids: List<String>): SavedSessionDeleteResult {
        val response = NativeRuntimeConfiguration.exchange(
            request("history_delete_many").put("ids", JSONArray(ids))
        )
        fun strings(field: String): List<String> {
            val array = response.getJSONArray(field)
            return List(array.length()) { array.getString(it) }
        }
        return SavedSessionDeleteResult(strings("deleted"), strings("failed"))
    }
    fun export(id: String, markdown: Boolean): File {
        val response = NativeRuntimeConfiguration.exchange(request("history_export", id)
            .put("exportDirectory", exports.absolutePath)
            .put("options", JSONObject().put("format", if (markdown) "markdown" else "txt")
                .put("content", "bilingual").put("includeTimestamps", true)))
        return File(response.getString("path")).also {
            check(it.parentFile?.canonicalFile == exports.canonicalFile && it.isFile) { "session_history_unavailable" }
        }
    }
}
internal data class SavedSessionItem(val id: String, val startedAtMs: Long, val count: Int, val limited: Boolean)
internal data class SavedSessionDeleteResult(val deleted: List<String>, val failed: List<String>)
internal data class SavedSessionSelection(val active: Boolean = false, val ids: Set<String> = emptySet()) {
    fun toggle(id: String, availableIds: Set<String>): SavedSessionSelection =
        if (!active || id !in availableIds) this
        else copy(ids = if (id in ids) ids - id else ids + id)
    fun toggleAll(availableIds: Set<String>): SavedSessionSelection =
        if (!active || availableIds.isEmpty()) this
        else copy(ids = if (availableIds.all { it in ids }) emptySet() else availableIds)
    fun prune(availableIds: Set<String>): SavedSessionSelection =
        if (availableIds.isEmpty()) SavedSessionSelection()
        else copy(ids = ids.intersect(availableIds))
}
internal data class SavedSessionPage(val total: Int, val page: Int, val entries: List<SavedSessionCaption>) {
    val hasNext get() = entries.lastOrNull()?.index?.let { it > 1 } ?: false
}
// No toString() that could accidentally include private subtitle text.
internal class SavedSessionCaption(val audioSource: String, val source: String, val translation: String, val createdAtMs: Long, val index: Int)

/** The picker may return an existing document. Only our private temporary file is disposable.
 * SAF writes are not atomic; a failed copy may leave partial content, but never deletes the destination. */
internal fun exportSavedSession(createFile: () -> File, writeFile: (File) -> Unit) {
    var temporary: File? = null
    try {
        temporary = createFile()
        writeFile(temporary)
    } finally {
        temporary?.delete()
    }
}
