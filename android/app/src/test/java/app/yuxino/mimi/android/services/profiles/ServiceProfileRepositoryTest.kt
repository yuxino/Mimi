package app.yuxino.mimi.android.services.profiles

import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import app.yuxino.mimi.android.provider.TranslationConfiguration
import org.junit.Assert.*
import org.junit.Test

class ServiceProfileRepositoryTest {
    private class MemoryDisk : ProfilePersistence {
        var catalog: String? = null
        val secrets = mutableMapOf<String, String>()
        var failCatalog = false
        var failSecret = false
        var failReadback = false
        var failWrittenSecretReadback = false
        private var unreadableRevision: String? = null
        var failCleanup = false
        var failCatalogReadAfterWrite = false
        private var failNextCatalogRead = false
        override fun readCatalog(): String? {
            if (failNextCatalogRead) { failNextCatalogRead = false; error("synthetic_catalog_read_failure") }
            return catalog
        }
        override fun writeCatalog(value: String): Boolean {
            if (failCatalog) return false
            catalog = value; failNextCatalogRead = failCatalogReadAfterWrite
            return true
        }
        override fun readSecret(revision: String) = if (failReadback || revision == unreadableRevision) null else secrets[revision]
        override fun writeSecret(revision: String, value: String): Boolean {
            if (failSecret) return false
            secrets[revision] = value
            unreadableRevision = revision.takeIf { failWrittenSecretReadback }
            return true
        }
        override fun retainSecrets(revisions: Set<String>) { check(!failCleanup); secrets.keys.retainAll(revisions) }
    }
    private fun contents(key: String, model: String = "lite", mode: String = "asrMt") = ProfileContents(
        ServiceConfiguration(ServiceProvider.DASHSCOPE, mapOf("apiKey" to key), "", "speech-$key", mode, "workspace-$key"),
        TextTranslationProvider.DEEPL, mapOf(TextTranslationProvider.DEEPL to TranslationConfiguration(apiKey = "mt-$key", provider = TextTranslationProvider.DEEPL)), model)

    @Test fun viewingAndCancelledDraftLeaveFreshStoreEmpty() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        assertTrue(repository.profiles().isEmpty()); assertNull(repository.activeId())
        assertNull(disk.catalog); assertTrue(disk.secrets.isEmpty())
        assertTrue(repository.migrate(emptyList(), null)); assertTrue(repository.profiles().isEmpty())
    }

    @Test fun realtimeModeRoundTripsAndLegacyProfilesKeepSimultaneousWithIndependentDrafts() {
        for (provider in listOf(ServiceProvider.OPENAI, ServiceProvider.GEMINI, ServiceProvider.AZURE)) {
            val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
            val credentials = provider.fields.associate { it.id to "synthetic-${it.id}" }
            val text = TranslationConfiguration(endpoint = "https://synthetic.example.invalid/v1", model = "independent-model",
                apiKey = "synthetic-text-key", provider = TextTranslationProvider.OPENAI_COMPATIBLE)
            val initial = ProfileContents(ServiceConfiguration(provider, credentials))
            assertEquals("simultaneous", initial.speech.speechTranslationMode)
            assertEquals(TextTranslationProvider.OPENAI_COMPATIBLE, initial.effectiveTranslation)
            val id = repository.save(null, "Simultaneous", initial)!!
            val independent = ProfileContents(ServiceConfiguration(provider, credentials, speechTranslationMode = "asrMt"),
                TextTranslationProvider.OPENAI_COMPATIBLE, mapOf(TextTranslationProvider.OPENAI_COMPATIBLE to text))
            assertEquals(id, repository.save(id, "Independent", independent))
            val copied = repository.duplicate(id, "Copy")!!
            assertEquals("asrMt", ServiceProfileRepository(disk).contents(copied).speech.speechTranslationMode)
            assertEquals("synthetic-text-key", repository.contents(copied).translations.getValue(TextTranslationProvider.OPENAI_COMPATIBLE).apiKey)
            assertEquals(id, repository.save(id, "Simultaneous", ProfileContents(initial.speech,
                independent.selectedTranslation, independent.translations)))
            assertEquals("synthetic-text-key", repository.contents(id).translations.getValue(TextTranslationProvider.OPENAI_COMPATIBLE).apiKey)
            val catalog = org.json.JSONObject(disk.catalog!!)
            val details = catalog.getJSONArray("profiles").getJSONObject(0).getJSONObject("details")
            details.getJSONObject("speech").remove("speechTranslationMode")
            details.put("selectedTranslation", TextTranslationProvider.BUILTIN.storageId)
            disk.catalog = catalog.toString()
            val restored = ServiceProfileRepository(disk).contents(id)
            assertEquals("simultaneous", restored.speech.speechTranslationMode)
            assertEquals(TextTranslationProvider.OPENAI_COMPATIBLE, restored.effectiveTranslation)
            assertEquals("synthetic-apiKey", restored.speech.value("apiKey"))
            assertFalse(disk.catalog!!.contains("synthetic-apiKey"))
            assertFalse(disk.catalog!!.contains("synthetic-text-key"))
        }
    }

    @Test fun xaiModesAndModelsPersistDuplicateAndPreserveLegacyVoice() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val contents = ProfileContents(ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "synthetic-xai-key"),
            xaiSpeechMode = "asrMt", xaiTextModel = "grok-custom-text"))
        val id = repository.save(null, "Streaming", contents)!!
        val copied = repository.duplicate(id, "Copy")!!
        val restored = ServiceProfileRepository(disk).contents(copied).speech
        assertEquals("asrMt", restored.xaiSpeechMode)
        assertEquals("grok-custom-text", restored.xaiTextModel)
        assertEquals("synthetic-xai-key", restored.value("apiKey"))
        val catalog = org.json.JSONObject(disk.catalog!!)
        val entries = catalog.getJSONArray("profiles")
        for (index in 0 until entries.length()) {
            entries.getJSONObject(index).getJSONObject("details").getJSONObject("speech").apply {
                remove("xaiSpeechMode"); remove("xaiTextModel")
            }
        }
        disk.catalog = catalog.toString()
        val legacy = ServiceProfileRepository(disk).contents(id).speech
        assertEquals("voice", legacy.xaiSpeechMode)
        assertEquals("grok-4.3", legacy.xaiTextModel)
        assertEquals("voice", LegacyServiceProfiles.read(mapOf("api_key_xai" to "synthetic-legacy-key"))
            .profiles.single().second.speech.xaiSpeechMode)
    }

    @Test fun legacyXaiAsrMigrationIsDurableRetryableAndScopedToItsOwnProfile() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val old = ProfileContents(ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "old-speech-key"),
            xaiSpeechMode = "asrMt", xaiTextModel = "old-grok-model"), selectedTranslation = TextTranslationProvider.BUILTIN,
            xaiIndependentTranslationMigrated = false)
        val id = repository.save(null, "Old streaming", old)!!
        val other = repository.save(null, "Other", ProfileContents(ServiceConfiguration(ServiceProvider.XAI,
            mapOf("apiKey" to "other-speech-key"), xaiSpeechMode = "asrMt"), TextTranslationProvider.XAI,
            mapOf(TextTranslationProvider.XAI to TranslationConfiguration(provider = TextTranslationProvider.XAI,
                model = "other-model", apiKey = "other-text-key"))))!!
        // Reproduce the published catalog shape: selectedTranslation was explicitly builtin, without a checkpoint.
        val catalog = org.json.JSONObject(disk.catalog!!)
        catalog.getJSONArray("profiles").getJSONObject(0).getJSONObject("details").remove("xaiIndependentTranslationMigrated")
        disk.catalog = catalog.toString()
        val before = disk.catalog
        disk.failSecret = true
        assertFalse(repository.migrateXaiAsrTranslations()); assertEquals(before, disk.catalog)
        disk.failSecret = false; disk.failWrittenSecretReadback = true
        assertFalse(repository.migrateXaiAsrTranslations()); assertEquals(before, disk.catalog)
        disk.failWrittenSecretReadback = false; disk.failCatalog = true
        assertFalse(repository.migrateXaiAsrTranslations()); assertEquals(before, disk.catalog)
        assertEquals(TextTranslationProvider.BUILTIN, repository.contents(id).selectedTranslation)
        disk.failCatalog = false
        assertTrue(repository.migrateXaiAsrTranslations())
        val migrated = ServiceProfileRepository(disk).contents(id)
        assertTrue(migrated.xaiIndependentTranslationMigrated)
        assertEquals(TextTranslationProvider.XAI, migrated.selectedTranslation)
        assertEquals("old-speech-key", migrated.speech.value("apiKey"))
        assertEquals("old-speech-key", migrated.translations.getValue(TextTranslationProvider.XAI).apiKey)
        assertEquals("old-grok-model", migrated.translations.getValue(TextTranslationProvider.XAI).model)
        assertEquals("other-text-key", repository.contents(other).translations.getValue(TextTranslationProvider.XAI).apiKey)
        assertEquals(id, repository.activeId())
        assertFalse(disk.catalog!!.contains("old-speech-key"))
        val after = disk.catalog
        assertTrue(repository.migrateXaiAsrTranslations()); assertEquals(after, disk.catalog)
    }

    @Test fun explicitOriginalAndDeletedGrokKeyNeverBorrowSpeechKeyAgain() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val speech = ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "speech-only-key"), xaiSpeechMode = "asrMt")
        val grok = TranslationConfiguration(provider = TextTranslationProvider.XAI, model = "grok-custom", apiKey = "text-only-key")
        val generic = TranslationConfiguration(provider = TextTranslationProvider.OPENAI_COMPATIBLE,
            endpoint = "https://generic.example.invalid/v1", model = "generic-model", apiKey = "generic-key")
        val id = repository.save(null, "Independent", ProfileContents(speech, TextTranslationProvider.XAI,
            mapOf(TextTranslationProvider.XAI to grok, TextTranslationProvider.OPENAI_COMPATIBLE to generic)))!!
        val original = ProfileContents(speech, TextTranslationProvider.NONE,
            repository.contents(id).translations + (TextTranslationProvider.XAI to grok.copy(apiKey = "")))
        assertEquals(id, repository.save(id, "Independent", original))
        assertTrue(repository.migrateXaiAsrTranslations())
        var reloaded = ServiceProfileRepository(disk).contents(id).withIndependentXaiTranslation()
        assertEquals(TextTranslationProvider.NONE, reloaded.selectedTranslation)
        assertEquals("", reloaded.translations.getValue(TextTranslationProvider.XAI).apiKey)
        assertEquals("generic-key", reloaded.translations.getValue(TextTranslationProvider.OPENAI_COMPATIBLE).apiKey)
        assertEquals("speech-only-key", reloaded.speech.value("apiKey"))
        assertEquals(id, repository.save(id, "Independent", ProfileContents(speech, TextTranslationProvider.XAI,
            reloaded.translations, xaiIndependentTranslationMigrated = reloaded.xaiIndependentTranslationMigrated)))
        assertTrue(repository.migrateXaiAsrTranslations())
        reloaded = ServiceProfileRepository(disk).contents(id)
        assertEquals("", reloaded.translations.getValue(TextTranslationProvider.XAI).apiKey)
    }

    @Test fun oldVoiceMigrationStaysAnUnsavedProjectionUntilAnExplicitAsrSave() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val voice = ServiceConfiguration(ServiceProvider.XAI, mapOf("apiKey" to "old-voice-key"), xaiTextModel = "old-voice-model")
        val id = repository.save(null, "Voice", ProfileContents(voice, xaiIndependentTranslationMigrated = false))!!
        val before = disk.catalog
        assertTrue(repository.migrateXaiAsrTranslations()); assertEquals(before, disk.catalog)
        val draft = repository.contents(id).withIndependentXaiTranslation()
        assertEquals(TextTranslationProvider.XAI, draft.selectedTranslation)
        assertEquals("old-voice-key", draft.translations.getValue(TextTranslationProvider.XAI).apiKey)
        assertEquals(before, disk.catalog)
        assertEquals("voice", repository.contents(id).speech.xaiSpeechMode)
        assertFalse(repository.contents(id).xaiIndependentTranslationMigrated)
        val asr = ServiceConfiguration(ServiceProvider.XAI, voice.credentials, xaiSpeechMode = "asrMt", xaiTextModel = voice.xaiTextModel)
        assertEquals(id, repository.save(id, "Streaming", ProfileContents(asr, draft.selectedTranslation, draft.translations,
            xaiIndependentTranslationMigrated = draft.xaiIndependentTranslationMigrated)))
        val saved = ServiceProfileRepository(disk).contents(id)
        assertTrue(saved.xaiIndependentTranslationMigrated)
        assertEquals("old-voice-model", saved.translations.getValue(TextTranslationProvider.XAI).model)
        val fresh = ProfileContents(asr)
        assertEquals(TextTranslationProvider.XAI, fresh.selectedTranslation)
        assertTrue(fresh.translations.isEmpty())
        assertSame(fresh, fresh.withIndependentXaiTranslation())
    }

    @Test fun sameProviderProfilesKeepKeysModelsModesAndTranslatorsIndependentAfterReload() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val first = repository.save(null, "ASR", contents("one", "plus"))!!
        val second = repository.save(null, "Live", contents("two", "flash", "liveTranslate"))!!
        assertEquals(first, repository.activeId())
        assertTrue(repository.activate(second))
        val reloaded = ServiceProfileRepository(disk)
        assertEquals(second, reloaded.activeId())
        assertEquals("one", reloaded.contents(first).speech.value("apiKey"))
        assertEquals("mt-two", reloaded.contents(second).translations[TextTranslationProvider.DEEPL]!!.apiKey)
        assertEquals("plus", reloaded.contents(first).qwenMtModel)
        assertEquals("flash", reloaded.contents(second).qwenMtModel)
        assertEquals("liveTranslate", reloaded.contents(second).speech.alibabaRealtimeMode)
        assertEquals("workspace-two", reloaded.contents(second).speech.alibabaWorkspaceId)
        assertFalse(disk.catalog!!.contains("mt-one")); assertFalse(disk.catalog!!.contains("\"apiKey\""))
    }

    @Test fun editingInactiveProfileNeverSwitchesActiveOrTouchesOtherSecrets() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val first = repository.save(null, "First", contents("one"))!!
        val second = repository.save(null, "Second", contents("two"))!!
        assertEquals(second, repository.save(second, "Renamed", contents("changed")))
        assertEquals(first, repository.activeId()); assertEquals("one", repository.contents(first).speech.value("apiKey"))
        assertEquals(2, disk.secrets.size)
    }

    @Test fun duplicateDeleteAndRenameAreIndependentAndKeepStableOrder() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val first = repository.save(null, "First", contents("one"))!!
        val copy = repository.duplicate(first, "Copy")!!
        assertNotEquals(first, copy); assertEquals(first, repository.activeId())
        assertTrue(repository.rename(copy, "Second"))
        assertEquals(listOf("First", "Second"), repository.profiles().map { it.name })
        assertTrue(repository.delete(first)); assertEquals(copy, repository.activeId())
        assertEquals("one", repository.contents(copy).speech.value("apiKey")); assertEquals(1, disk.secrets.size)
        assertTrue(repository.delete(copy)); assertNull(repository.activeId()); assertTrue(disk.secrets.isEmpty())
    }

    @Test fun credentialWriteFailureReadbackFailureAndCatalogFailureKeepPreviousDataAndSelection() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val first = repository.save(null, "First", contents("one"))!!
        val before = disk.catalog
        disk.failSecret = true
        assertNull(repository.save(first, "Changed", contents("two"))); assertEquals(before, disk.catalog)
        disk.failSecret = false; disk.failReadback = true
        assertNull(repository.save(first, "Changed", contents("two"))); assertEquals(before, disk.catalog)
        disk.failReadback = false; disk.failCatalog = true
        assertNull(repository.save(null, "New", contents("three"), true)); assertEquals(before, disk.catalog)
        assertFalse(repository.delete(first)); assertFalse(repository.rename(first, "Changed"))
        assertEquals("one", repository.contents(first).speech.value("apiKey")); assertEquals(first, repository.activeId())
        disk.failCatalog = false
        assertTrue(repository.rename(first, "Final")); assertEquals(1, disk.secrets.size)
    }

    @Test fun failedActivationDoesNotLoseThePreviousSelection() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val first = repository.save(null, "First", contents("one"))!!
        val second = repository.save(null, "Second", contents("two"))!!
        disk.failCatalog = true
        assertFalse(repository.activate(second)); assertEquals(first, repository.activeId())
        assertFalse(repository.activate("missing"))
    }

    @Test fun cleanupFailureDoesNotTurnSuccessfulSaveIntoFailure() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val first = repository.save(null, "First", contents("one"))!!
        disk.failCleanup = true
        assertEquals(first, repository.save(first, "Changed", contents("two")))
        assertEquals("two", ServiceProfileRepository(disk).contents(first).speech.value("apiKey"))
    }

    @Test fun twentyProfileLimitAlsoAppliesToDuplicateButNotEdits() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val ids = (1..20).map { repository.save(null, "Profile $it", contents("$it"))!! }
        assertNull(repository.save(null, "Twenty one", contents("extra")))
        assertNull(repository.duplicate(ids.first(), "Copy"))
        assertEquals(ids.last(), repository.save(ids.last(), "Edited", contents("changed")))
        assertEquals(20, repository.profiles().size)
    }

    @Test fun corruptedCatalogIsNeverReplacedWithEmptyStore() {
        val disk = MemoryDisk().apply { catalog = "not-json" }
        val repository = ServiceProfileRepository(disk)
        assertThrows(IllegalStateException::class.java) { repository.profiles() }
        assertThrows(IllegalStateException::class.java) { repository.save(null, "New", contents("new")) }
        assertEquals("not-json", disk.catalog)
    }

    @Test fun migrationIsAtomicRetryableAndDoesNotReimportDeletedProfiles() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val profiles = listOf(ServiceProfile("legacy-dashscope", "Ali", ServiceProvider.DASHSCOPE) to contents("old"))
        disk.failCatalog = true
        assertFalse(repository.migrate(profiles, "legacy-dashscope")); assertNull(disk.catalog)
        disk.failCatalog = false
        assertTrue(repository.migrate(profiles, "legacy-dashscope"))
        assertEquals("old", repository.contents("legacy-dashscope").speech.value("apiKey"))
        assertTrue(repository.delete("legacy-dashscope"))
        assertTrue(repository.migrate(profiles, "legacy-dashscope")); assertTrue(repository.profiles().isEmpty())
    }

    @Test fun legacyMigrationPreservesPartialServicesAllTranslatorDraftsAndOriginalSharedKeyOwner() {
        val migration = LegacyServiceProfiles.read(mapOf(
            "provider" to "tencent", "api_key" to "original-shared", "credential_tencent_appId" to "partial-id",
            "api_key_openai" to "openai-key", "base_url_gemini" to "wss://example.invalid",
            "text_translation_provider" to "deepL", "text_translation_deepL_api_key" to "deepl-key",
            "chatmock_endpoint" to "https://example.invalid/v1", "chatmock_model" to "model", "chatmock_api_key" to "compatible-key",
            "text_translation_chatMock_endpoint" to "http://127.0.0.1:8000/v1", "text_translation_chatMock_api_key" to "separate-key",
            "qwen_mt_model_dashscope" to "plus", "font_size" to 18))
        assertEquals("legacy-tencent", migration.activeId)
        assertEquals(setOf(ServiceProvider.DASHSCOPE, ServiceProvider.OPENAI, ServiceProvider.GEMINI, ServiceProvider.TENCENT), migration.profiles.map { it.first.provider }.toSet())
        val ali = migration.profiles.first { it.first.provider == ServiceProvider.DASHSCOPE }.second
        assertEquals("", ali.speech.value("apiKey")); assertEquals("asrMt", ali.speech.alibabaRealtimeMode)
        assertEquals("plus", ali.qwenMtModel); assertEquals(TextTranslationProvider.DEEPL, ali.selectedTranslation)
        assertEquals("compatible-key", ali.translations[TextTranslationProvider.OPENAI_COMPATIBLE]!!.apiKey)
        assertEquals("separate-key", ali.translations[TextTranslationProvider.CHAT_MOCK]!!.apiKey)
        assertEquals("partial-id", migration.profiles.first { it.first.provider == ServiceProvider.TENCENT }.second.speech.value("appId"))
        assertTrue(LegacyServiceProfiles.read(mapOf("font_size" to 18)).profiles.isEmpty())
    }

    @Test fun sharedLegacyKeyGoesOnlyToSelectedApiKeyProviderAndScopedKeyWins() {
        val migrated = LegacyServiceProfiles.read(mapOf("provider" to "openai", "api_key" to "shared", "api_key_dashscope" to "ali"))
        assertEquals("shared", migrated.profiles.first { it.first.provider == ServiceProvider.OPENAI }.second.speech.value("apiKey"))
        assertEquals("ali", migrated.profiles.first { it.first.provider == ServiceProvider.DASHSCOPE }.second.speech.value("apiKey"))
        val scoped = LegacyServiceProfiles.read(mapOf("provider" to "openai", "api_key" to "shared", "api_key_openai" to "scoped"))
        assertEquals("scoped", scoped.profiles.single().second.speech.value("apiKey"))
    }
    @Test fun damagedCredentialRevisionFailsClosedWithoutExposingOrReplacingIt() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val id = repository.save(null, "First", contents("one"))!!
        disk.secrets[disk.secrets.keys.single()] = "invalid synthetic secret payload"
        val error = assertThrows(IllegalStateException::class.java) { repository.contents(id) }
        assertEquals("profile_credentials_unavailable", error.message)
        assertEquals("invalid synthetic secret payload", disk.secrets.values.single())
    }

    @Test fun legacyTextModelFallbackNeverChangesSpeechModelOrUnknownFutureKeys() {
        for (value in listOf("", "unknown", "plus")) {
            val migrated = LegacyServiceProfiles.read(mapOf("model_dashscope" to "legacy-realtime", "qwen_mt_model_dashscope" to value)).profiles.single().second
            assertEquals(if (value == "plus") "plus" else "lite", migrated.qwenMtModel)
            assertEquals("legacy-realtime", migrated.speech.model)
        }
        assertFalse(LegacyServiceProfiles.ownsKey("credential_futureProvider_apiKey"))
        assertFalse(LegacyServiceProfiles.ownsKey("text_translation_futureProvider_api_key"))
        assertFalse(LegacyServiceProfiles.ownsKey("font_size"))
    }

    @Test fun uncertainPostCommitReadbackKeepsBothCredentialGenerationsUntilNextVerifiedWrite() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val id = repository.save(null, "First", contents("one"))!!
        disk.failCatalogReadAfterWrite = true
        assertThrows(IllegalStateException::class.java) { repository.save(id, "Changed", contents("two")) }
        assertEquals(2, disk.secrets.size)
        // The rename already committed; both old and new secrets remain recoverable.
        assertEquals("two", repository.contents(id).speech.value("apiKey"))
        disk.failCatalogReadAfterWrite = false
        assertTrue(repository.rename(id, "Verified")); assertEquals(1, disk.secrets.size)
    }

    @Test fun activeContentsKeepsSpeechTranslationAndModelTogetherDuringSelectionChanges() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val first = repository.save(null, "First", contents("one", "plus"))!!
        val second = repository.save(null, "Second", contents("two", "flash"))!!
        val failure = java.util.concurrent.atomic.AtomicReference<Throwable?>()
        val selector = Thread {
            try { repeat(200) { assertTrue(repository.activate(if (it % 2 == 0) first else second)) } }
            catch (error: Throwable) { failure.set(error) }
        }
        selector.start()
        repeat(200) {
            val snapshot = repository.activeContents()!!
            val key = snapshot.speech.value("apiKey")
            assertEquals("mt-$key", snapshot.translations[TextTranslationProvider.DEEPL]!!.apiKey)
            assertEquals(if (key == "one") "plus" else "flash", snapshot.qwenMtModel)
        }
        selector.join(); failure.get()?.let { throw it }
    }

    @Test fun explicitIdentityAppendRetriesWithoutDuplicatesOrOverwritingExistingProfiles() {
        val disk = MemoryDisk(); val repository = ServiceProfileRepository(disk)
        val original = repository.save(null, "Original", contents("original"))!!
        val importedId = "desktop-import-id"
        disk.failCatalog = true
        assertNull(repository.create(importedId, "Imported", contents("imported")))
        disk.failCatalog = false
        assertEquals(importedId, repository.create(importedId, "Imported", contents("imported")))
        assertNull(repository.create(importedId, "Replacement", contents("replacement")))
        assertEquals("imported", repository.contents(importedId).speech.value("apiKey"))
        assertEquals(original, repository.activeId()); assertEquals(2, repository.profiles().size)
    }

}
