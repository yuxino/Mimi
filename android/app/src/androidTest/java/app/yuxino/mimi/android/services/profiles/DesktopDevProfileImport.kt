package app.yuxino.mimi.android.services.profiles

import android.content.Context
import android.content.pm.ApplicationInfo
import app.yuxino.mimi.android.capture.MimiService
import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import app.yuxino.mimi.android.settings.SettingsStore
import java.io.File
import java.nio.file.Files
import java.util.UUID
import org.json.JSONObject

/** Explicit one-time debug fixture. Never registered as application IPC or production UI. */
internal fun runDesktopDevProfileImport(context: Context): String {
    val input = File(context.filesDir, "desktop-dev-profile-import.json")
    var imported = 0
    var existing = 0
    try {
        check(context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0)
        check(context.packageName.endsWith(".dev"))
        check(!MimiService.isRunning && !MimiService.isStopping)
        check(!Files.isSymbolicLink(input.toPath()) && input.isFile && input.length() in 1..1_048_576)
        val root = JSONObject(input.readText(Charsets.UTF_8))
        check(root.getInt("version") == 1 && root.getString("source") == "mimi-desktop-dev")
        val array = root.getJSONArray("profiles")
        check(array.length() in 1..SettingsStore.MAX_SERVICE_PROFILES)
        val parsed = (0 until array.length()).map { parseDesktopDevProfile(array.getJSONObject(it)) }
        check(parsed.map { it.sourceId }.distinct().size == parsed.size)
        val activeSource = if (root.isNull("activeSourceId")) null else root.optString("activeSourceId").takeIf(String::isNotBlank)
        check(activeSource == null || parsed.any { it.sourceId == activeSource })
        SettingsStore.profiles(context)
        val repository = SettingsStore::class.java.getDeclaredField("profileRepository").run {
            isAccessible = true; get(SettingsStore) as ServiceProfileRepository
        }
        val currentIds = repository.profiles().map { it.id }.toSet()
        check(currentIds.size + parsed.count { it.targetId !in currentIds } <= SettingsStore.MAX_SERVICE_PROFILES)
        parsed.forEach { profile ->
            check(!MimiService.isRunning && !MimiService.isStopping)
            if (repository.profiles().any { it.id == profile.targetId }) {
                existing++
            } else {
                check(repository.create(profile.targetId, profile.name, profile.contents) == profile.targetId)
                imported++
            }
        }
        val activate = parsed.firstOrNull { it.sourceId == activeSource }
        // A partial import never switches an existing active profile. A retry only appends missing IDs.
        if (activate != null) {
            check(!MimiService.isRunning && !MimiService.isStopping)
            check(repository.activate(activate.targetId))
        }
        return "Desktop dev profiles: imported=$imported; alreadyPresent=$existing; activeCopied=${activate != null}; captureStarted=false"
    } catch (_: Exception) {
        // JSON, crypto and validation errors can contain private input. Report only progress counts.
        throw IllegalStateException("desktop_dev_import_failed_after_${imported}_profiles")
    } finally {
        check(!input.exists() || input.delete()) { "desktop_dev_import_file_cleanup_failed" }
    }
}

private class DesktopDevProfile(val sourceId: String, val name: String, val contents: ProfileContents) {
    val targetId: String = UUID.nameUUIDFromBytes("mimi-desktop-dev:$sourceId".toByteArray(Charsets.UTF_8)).toString()
    override fun toString() = "DesktopDevProfile(redacted)"
}

private fun parseDesktopDevProfile(value: JSONObject): DesktopDevProfile {
    fun text(objectValue: JSONObject, key: String, max: Int, fallback: String = ""): String {
        val result = if (objectValue.has(key)) objectValue.getString(key) else fallback
        check(result.length <= max && result.none(Char::isISOControl))
        return result
    }
    val sourceId = text(value, "sourceId", 128)
    val name = text(value, "name", 80).trim()
    check(sourceId.isNotBlank() && name.isNotBlank())
    val provider = ServiceProvider.entries.first { it.id == value.getString("provider") }
    val credentials = value.getJSONObject("credentials")
    check(credentials.keys().asSequence().all { key -> provider.fields.any { it.id == key } })
    val speech = ServiceConfiguration(provider, provider.fields.associate { it.id to text(credentials, it.id, 16_384) },
        text(value, "endpoint", 4096), text(value, "model", 512), text(value, "alibabaRealtimeMode", 32, "asrMt"),
        text(value, "alibabaWorkspaceId", 256))
    check(speech.alibabaRealtimeMode in listOf("asrMt", "liveTranslate"))
    val qwen = text(value, "qwenMtModel", 16, "lite")
    check(qwen in listOf("lite", "flash", "plus"))
    val selected = TextTranslationProvider.entries.first { it.storageId == value.getString("selectedTranslation") }
    val array = value.getJSONArray("translations")
    check(array.length() <= TextTranslationProvider.entries.size)
    val drafts = (0 until array.length()).map { index ->
        val item = array.getJSONObject(index)
        val translator = TextTranslationProvider.entries.first { it.storageId == item.getString("provider") }
        check(translator !in setOf(TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE))
        translator to TranslationConfiguration(text(item, "endpoint", 4096), text(item, "model", 512),
            text(item, "apiKey", 16_384), item.optBoolean("allowLocalHttp", false), translator)
    }
    check(drafts.map { it.first }.distinct().size == drafts.size)
    return DesktopDevProfile(sourceId, name, ProfileContents(speech, selected, drafts.toMap(), qwen))
}
