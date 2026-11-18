package app.yuxino.mimi.android.services.setup

import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider
import org.junit.Assert.*
import org.junit.Test

class TextTranslationSetupTest {
    @Test fun onlyKnownCatalogIdentitiesResolveBrowserDestinations() {
        assertNull(textTranslationSetupUrl("", TextTranslationSetupPage.GUIDE))
        assertNull(textTranslationSetupUrl("https://untrusted.example/setup", TextTranslationSetupPage.API_KEY))
        val cloud = TextTranslationPresets.entries.first { it.id == "openai" }
        assertEquals(cloud.apiKeyUrl, textTranslationSetupUrl(cloud.id, TextTranslationSetupPage.API_KEY))
        assertEquals(cloud.guideUrl, textTranslationSetupUrl(cloud.id, TextTranslationSetupPage.GUIDE))
    }

    @Test fun localAndKeylessServicesNeverOfferAnApiKeyDestination() {
        for (preset in TextTranslationPresets.entries.filter { it.setupKind != "cloud" }) {
            assertNull(textTranslationSetupUrl(preset.id, TextTranslationSetupPage.API_KEY))
            assertEquals(preset.guideUrl, textTranslationSetupUrl(preset.id, TextTranslationSetupPage.GUIDE))
        }
    }

    @Test fun everyBuiltInSpeechServiceHasItsOwnCredentialDestination() {
        for (provider in ServiceProvider.entries) {
            assertNotNull(provider.id, serviceSetupUrl(provider.wireProvider, TextTranslationSetupPage.API_KEY))
        }
        assertNull(serviceSetupUrl("https://untrusted.example/setup", TextTranslationSetupPage.API_KEY))
        assertNull(serviceSetupUrl("customOpenAIASR", TextTranslationSetupPage.API_KEY))
        assertTrue(serviceSetupUrl("volcanoEngine", TextTranslationSetupPage.API_KEY)!!.contains("/speech/"))
        assertTrue(serviceSetupUrl("baiduTranslate", TextTranslationSetupPage.API_KEY)!!.contains("machinetranslation"))
        assertTrue(serviceSetupUrl("tencentCloud", TextTranslationSetupPage.API_KEY)!!.endsWith("/cam/capi"))
    }

    @Test fun dedicatedTextServicesUseTheirOwnSetupWithoutGuessingCustomProviders() {
        for (provider in listOf(TextTranslationProvider.XAI, TextTranslationProvider.DEEPL)) {
            val id = requireNotNull(textTranslationServiceSetupId(provider))
            assertNotNull(serviceSetupUrl(id, TextTranslationSetupPage.API_KEY))
        }
        for (provider in listOf(TextTranslationProvider.OPENAI_COMPATIBLE, TextTranslationProvider.CHAT_MOCK,
                TextTranslationProvider.DEEPLX, TextTranslationProvider.NONE, TextTranslationProvider.BUILTIN)) {
            assertNull(textTranslationServiceSetupId(provider))
        }
    }
}
