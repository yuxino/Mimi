package app.yuxino.mimi.android.provider.recognition

import app.yuxino.mimi.android.provider.ServiceConfiguration
import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.buildSpeechProbeConfiguration
import app.yuxino.mimi.android.provider.resolveSpeechCredentialDraft
import org.junit.Assert.*
import org.junit.Test

/** Exercise the actual adapter and JNI resolver used by both Save and speech checks. */
class SpeechCredentialDraftTest {
    private val azure = mapOf("endpoint" to "https://saved.openai.azure.com", "deployment" to "translate",
        "transcriptionDeployment" to "transcribe", "apiKey" to "synthetic-saved-azure-key")
    private val tencent = mapOf("appId" to "123", "secretId" to "synthetic-saved-id", "secretKey" to "synthetic-saved-tencent-key")
    private val baidu = mapOf("appId" to "123", "appKey" to "synthetic-saved-baidu-key")

    private fun blanks(provider: ServiceProvider, values: Map<String, String>) =
        provider.fields.associate { it.id to if (it.secret) "" else values[it.id].orEmpty() }

    @Test fun unchangedOwnersRetainTheirSavedSecretsThroughTheUiAdapter() {
        for ((provider, values) in listOf(ServiceProvider.AZURE to azure, ServiceProvider.TENCENT to tencent, ServiceProvider.BAIDU to baidu)) {
            val saved = ServiceConfiguration(provider, values)
            val result = resolveSpeechCredentialDraft(saved, blanks(provider, values), "simultaneous")
            assertEquals(values, result)
            assertEquals(values, saved.credentials)
        }
    }

    @Test fun changingAzureResourceRequiresAnExplicitKeyButChangingDeploymentDoesNot() {
        val saved = ServiceConfiguration(ServiceProvider.AZURE, azure)
        val changed = blanks(ServiceProvider.AZURE, azure) + ("endpoint" to "https://replacement.openai.azure.com")
        assertThrows(IllegalArgumentException::class.java) { resolveSpeechCredentialDraft(saved, changed, "simultaneous") }
        val replacement = resolveSpeechCredentialDraft(saved, changed + ("apiKey" to "synthetic-replacement-key"), "simultaneous")
        assertEquals("https://replacement.openai.azure.com", replacement["endpoint"])
        assertEquals("synthetic-replacement-key", replacement["apiKey"])
        assertEquals(azure["apiKey"], resolveSpeechCredentialDraft(saved,
            blanks(ServiceProvider.AZURE, azure) + ("deployment" to "another-deployment"), "simultaneous")["apiKey"])
        assertEquals(azure, saved.credentials)
    }

    @Test fun tencentAppAndSecretIdentitiesCannotBorrowPreviousSecrets() {
        val saved = ServiceConfiguration(ServiceProvider.TENCENT, tencent)
        val changedApp = blanks(ServiceProvider.TENCENT, tencent) + ("appId" to "456")
        val changedId = blanks(ServiceProvider.TENCENT, tencent) + ("secretId" to "synthetic-replacement-id")
        for (draft in listOf(changedApp, changedId)) {
            assertThrows(IllegalArgumentException::class.java) { resolveSpeechCredentialDraft(saved, draft, "simultaneous") }
        }
        val explicit = changedApp + mapOf("secretId" to "synthetic-replacement-id", "secretKey" to "synthetic-replacement-key")
        assertEquals(explicit, resolveSpeechCredentialDraft(saved, explicit, "simultaneous"))
        assertEquals(tencent, saved.credentials)
    }

    @Test fun baiduAppChangesRequireANewAppKey() {
        val saved = ServiceConfiguration(ServiceProvider.BAIDU, baidu)
        val changed = blanks(ServiceProvider.BAIDU, baidu) + ("appId" to "456")
        assertThrows(IllegalArgumentException::class.java) { resolveSpeechCredentialDraft(saved, changed, "simultaneous") }
        val explicit = changed + ("appKey" to "synthetic-replacement-baidu-key")
        assertEquals(explicit, resolveSpeechCredentialDraft(saved, explicit, "simultaneous"))
        assertEquals(baidu, saved.credentials)
    }

    @Test fun singleKeyProvidersAndAzureRecognitionKeepTheirExistingDraftBehavior() {
        for (provider in listOf(ServiceProvider.DASHSCOPE, ServiceProvider.OPENAI, ServiceProvider.GEMINI, ServiceProvider.VOLCANO, ServiceProvider.XAI)) {
            val saved = ServiceConfiguration(provider, mapOf("apiKey" to "synthetic-saved-key"))
            assertEquals("synthetic-saved-key", resolveSpeechCredentialDraft(saved, mapOf("apiKey" to ""), "simultaneous")["apiKey"])
            assertEquals("synthetic-new-key", resolveSpeechCredentialDraft(saved, mapOf("apiKey" to " synthetic-new-key "), "simultaneous")["apiKey"])
        }
        val asr = ServiceConfiguration(ServiceProvider.AZURE, azure - "deployment", speechTranslationMode = "asrMt")
        val values = resolveSpeechCredentialDraft(asr, blanks(ServiceProvider.AZURE, asr.credentials), "asrMt")
        assertEquals("", values["deployment"])
        assertEquals(azure["apiKey"], values["apiKey"])
        assertEquals("", buildSpeechProbeConfiguration(ServiceConfiguration(ServiceProvider.AZURE, values,
            speechTranslationMode = "asrMt"), "en").getJSONObject("credentials").getString("deployment"))
    }
}
