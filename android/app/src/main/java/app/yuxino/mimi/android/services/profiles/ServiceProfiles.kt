package app.yuxino.mimi.android.services.profiles

import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/** Safe for lists and saved-instance state. Never contains a credential. */
data class ServiceProfile(val id: String, val name: String, val provider: ServiceProvider)

internal class ProfileContents(
    val speech: ServiceConfiguration,
    val selectedTranslation: TextTranslationProvider = if (speech.provider == ServiceProvider.XAI && speech.usesIndependentTextTranslation)
        TextTranslationProvider.XAI else if (speech.provider.supportsSpeechTranslationMode) TextTranslationProvider.OPENAI_COMPATIBLE
        else TextTranslationProvider.BUILTIN,
    val translations: Map<TextTranslationProvider, TranslationConfiguration> = emptyMap(),
    val qwenMtModel: String = "lite",
    val xaiIndependentTranslationMigrated: Boolean = true,
) {
    override fun toString() = "ProfileContents(redacted)"
    val effectiveTranslation: TextTranslationProvider get() =
        if (speech.provider.supportsSpeechTranslationMode && selectedTranslation == TextTranslationProvider.BUILTIN)
            TextTranslationProvider.OPENAI_COMPATIBLE else selectedTranslation

    /** One-time, same-profile compatibility projection. Ordinary route reads never borrow a speech key. */
    fun withIndependentXaiTranslation(): ProfileContents {
        if (speech.provider != ServiceProvider.XAI || xaiIndependentTranslationMigrated) return this
        if (selectedTranslation != TextTranslationProvider.BUILTIN) return ProfileContents(
            speech, selectedTranslation, translations, qwenMtModel, true)
        val routes = translations.toMutableMap()
        routes.putIfAbsent(TextTranslationProvider.XAI, TranslationConfiguration(
            model = speech.xaiTextModel, apiKey = speech.value("apiKey"), provider = TextTranslationProvider.XAI))
        return ProfileContents(speech, TextTranslationProvider.XAI, routes, qwenMtModel, true)
    }
}

/** A successful write must be durable; catalog replacement must be atomic. */
internal interface ProfilePersistence {
    fun readCatalog(): String?
    fun writeCatalog(value: String): Boolean
    fun readSecret(revision: String): String?
    fun writeSecret(revision: String, value: String): Boolean
    fun retainSecrets(revisions: Set<String>)
}

internal class ServiceProfileRepository(private val disk: ProfilePersistence) {
    companion object { const val LIMIT = 20 }
    private data class Entry(val profile: ServiceProfile, val revision: String, val details: JSONObject)
    private data class Catalog(val activeId: String?, val entries: List<Entry>)

    @Synchronized fun initialized(): Boolean = disk.readCatalog() != null
    @Synchronized fun profiles(): List<ServiceProfile> = catalog().entries.map { it.profile }
    @Synchronized fun activeId(): String? = catalog().activeId
    @Synchronized fun activeContents(): ProfileContents? = catalog().activeId?.let(::contents)

    @Synchronized fun contents(id: String): ProfileContents {
        val entry = catalog().entries.firstOrNull { it.profile.id == id }
            ?: throw IllegalArgumentException("profile_missing")
        try {
            val secret = disk.readSecret(entry.revision)?.let(::JSONObject)
                ?: throw IllegalStateException("profile_credentials_unavailable")
            val speech = entry.details.getJSONObject("speech")
            val credentials = secret.getJSONObject("speech").let { values -> values.keys().asSequence().associateWith(values::getString) }
            val translations = entry.details.getJSONObject("translations").let { values ->
                values.keys().asSequence().associate { key ->
                    val provider = TextTranslationProvider.entries.first { it.storageId == key }
                    val value = values.getJSONObject(key)
                    provider to TranslationConfiguration(value.getString("endpoint"), value.getString("model"),
                        secret.getJSONObject("translations").getString(key), value.getBoolean("localHttp"), provider)
                }
            }
            return ProfileContents(ServiceConfiguration(entry.profile.provider, credentials,
                speech.getString("endpoint"), speech.getString("model"), speech.optString("alibabaRealtimeMode", "asrMt"),
                speech.optString("alibabaWorkspaceId"), speech.optString("xaiSpeechMode", "voice"),
                speech.optString("xaiTextModel", "grok-4.3"), speech.optString("speechTranslationMode", "simultaneous")),
                TextTranslationProvider.fromStorageId(entry.details.getString("selectedTranslation")), translations,
                entry.details.getString("qwenMtModel"), entry.details.optBoolean("xaiIndependentTranslationMigrated", false))
        } catch (_: Exception) {
            throw IllegalStateException("profile_credentials_unavailable")
        }
    }

    /** Only Save crosses this boundary. A provider picker never calls it. */
    @Synchronized fun save(id: String?, name: String, contents: ProfileContents, activate: Boolean = false): String? =
        saveWithId(id, name, contents, activate)

    /** Explicit identity for recoverable imports; existing profiles are never overwritten. */
    @Synchronized fun create(id: String, name: String, contents: ProfileContents): String? {
        if (id.isBlank() || catalog().entries.any { it.profile.id == id }) return null
        return saveWithId(null, name, contents, false, id)
    }

    private fun saveWithId(id: String?, name: String, contents: ProfileContents, activate: Boolean, creationId: String? = null): String? {
        val current = catalog()
        val old = id?.let { existing -> current.entries.firstOrNull { it.profile.id == existing } ?: return null }
        if (old == null && current.entries.size >= LIMIT) return null
        val cleaned = name.trim()
        if (cleaned.isEmpty() || cleaned.length > 80 || cleaned.any(Char::isISOControl)) return null
        if (old != null && old.profile.provider != contents.speech.provider) return null
        val profile = ServiceProfile(id ?: creationId ?: UUID.randomUUID().toString(), cleaned, contents.speech.provider)
        val entry = prepare(profile, contents) ?: return null
        val entries = if (old == null) current.entries + entry else current.entries.map { if (it.profile.id == id) entry else it }
        val active = if (activate || current.activeId == null) profile.id else current.activeId
        return if (publish(Catalog(active, entries))) profile.id else null
    }

    @Synchronized fun migrate(profiles: List<Pair<ServiceProfile, ProfileContents>>, activeId: String?): Boolean {
        if (initialized()) return true
        require(profiles.size <= LIMIT && profiles.map { it.first.id }.distinct().size == profiles.size) { "profile_migration" }
        val entries = profiles.map { (profile, contents) -> prepare(profile, contents) ?: return false }
        return publish(Catalog(activeId?.takeIf { id -> entries.any { it.profile.id == id } } ?: entries.firstOrNull()?.profile?.id, entries))
    }

    /** Old catalogs explicitly wrote builtin, so a separate nonsecret checkpoint distinguishes them. */
    @Synchronized fun migrateXaiAsrTranslations(): Boolean {
        val current = catalog()
        var changed = false
        val entries = current.entries.map { entry ->
            if (entry.profile.provider != ServiceProvider.XAI || entry.details.optBoolean("xaiIndependentTranslationMigrated", false) ||
                entry.details.getJSONObject("speech").optString("xaiSpeechMode", "voice") != "asrMt") return@map entry
            changed = true
            // prepare verifies the new secret before publish can expose its metadata/checkpoint.
            prepare(entry.profile, contents(entry.profile.id).withIndependentXaiTranslation()) ?: return false
        }
        return !changed || publish(current.copy(entries = entries))
    }

    @Synchronized fun activate(id: String): Boolean {
        val current = catalog()
        if (current.entries.none { it.profile.id == id }) return false
        return publish(current.copy(activeId = id))
    }

    @Synchronized fun rename(id: String, name: String): Boolean {
        val current = catalog()
        val cleaned = name.trim()
        if (cleaned.isEmpty() || cleaned.length > 80 || cleaned.any(Char::isISOControl) || current.entries.none { it.profile.id == id }) return false
        return publish(current.copy(entries = current.entries.map { if (it.profile.id == id) it.copy(profile = it.profile.copy(name = cleaned)) else it }))
    }

    @Synchronized fun duplicate(id: String, name: String): String? = save(null, name, contents(id))

    @Synchronized fun delete(id: String): Boolean {
        val current = catalog()
        val entries = current.entries.filterNot { it.profile.id == id }
        if (entries.size == current.entries.size) return false
        return publish(Catalog(if (current.activeId == id) entries.firstOrNull()?.profile?.id else current.activeId, entries))
    }

    private fun prepare(profile: ServiceProfile, contents: ProfileContents): Entry? {
        val revision = UUID.randomUUID().toString()
        val secrets = JSONObject().put("speech", JSONObject(contents.speech.credentials))
            .put("translations", JSONObject().apply { contents.translations.forEach { (provider, value) -> put(provider.storageId, value.apiKey) } })
            .toString()
        if (!disk.writeSecret(revision, secrets) || disk.readSecret(revision) != secrets) return null
        val speech = contents.speech
        val details = JSONObject().put("speech", JSONObject().put("endpoint", speech.endpoint).put("model", speech.model)
            .put("alibabaRealtimeMode", speech.alibabaRealtimeMode).put("alibabaWorkspaceId", speech.alibabaWorkspaceId)
            .put("xaiSpeechMode", speech.xaiSpeechMode).put("xaiTextModel", speech.xaiTextModel).put("speechTranslationMode", speech.speechTranslationMode))
            .put("selectedTranslation", contents.selectedTranslation.storageId).put("qwenMtModel", contents.qwenMtModel)
            .put("xaiIndependentTranslationMigrated", contents.xaiIndependentTranslationMigrated)
            .put("translations", JSONObject().apply { contents.translations.forEach { (provider, value) ->
                put(provider.storageId, JSONObject().put("endpoint", value.endpoint).put("model", value.model).put("localHttp", value.allowLocalHttp))
            } })
        return Entry(profile, revision, details)
    }

    private fun publish(catalog: Catalog): Boolean {
        val value = JSONObject().put("version", 1).put("activeId", catalog.activeId ?: JSONObject.NULL)
            .put("profiles", JSONArray().apply { catalog.entries.forEach { entry ->
                put(JSONObject().put("id", entry.profile.id).put("name", entry.profile.name).put("provider", entry.profile.provider.id)
                    .put("revision", entry.revision).put("details", entry.details))
            } }).toString()
        if (!disk.writeCatalog(value) || disk.readCatalog() != value) return false
        // Failed cleanup leaves encrypted orphans only; never roll back a durable catalog.
        runCatching { disk.retainSecrets(catalog.entries.map { it.revision }.toSet()) }
        return true
    }

    private fun catalog(): Catalog {
        val raw = disk.readCatalog() ?: return Catalog(null, emptyList())
        try {
            val root = JSONObject(raw)
            require(root.getInt("version") == 1)
            val array = root.getJSONArray("profiles")
            require(array.length() <= LIMIT)
            val entries = (0 until array.length()).map { index ->
                val item = array.getJSONObject(index)
                val provider = ServiceProvider.entries.first { it.id == item.getString("provider") }
                Entry(ServiceProfile(item.getString("id"), item.getString("name"), provider), item.getString("revision"), item.getJSONObject("details"))
            }
            require(entries.map { it.profile.id }.distinct().size == entries.size)
            val active = if (root.isNull("activeId")) null else root.getString("activeId")
            require((entries.isEmpty() && active == null) || entries.any { it.profile.id == active })
            return Catalog(active, entries)
        } catch (_: Exception) {
            throw IllegalStateException("profile_catalog_unavailable")
        }
    }
}
