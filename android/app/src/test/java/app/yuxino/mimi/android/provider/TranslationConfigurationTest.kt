package app.yuxino.mimi.android.provider

import org.junit.Assert.*
import org.junit.Test

class TranslationConfigurationTest {
    private fun configuration(endpoint: String, allowHttp: Boolean = false) =
        TranslationConfiguration(endpoint, "gpt-5.4", "", allowHttp)

    private fun rejects(expected: String, action: () -> Unit) {
        try {
            action()
            fail("Expected $expected")
        } catch (error: IllegalArgumentException) {
            assertEquals(expected, error.message)
        }
    }

    @Test fun baseAndFullEndpointsResolveToTheSameDestination() {
        val expected = "https://example.test/v1/chat/completions"
        listOf("https://example.test/v1", "https://example.test/v1/", expected).forEach {
            assertEquals(expected, normalizeTranslationEndpoint(configuration(it)))
        }
        assertEquals("https://example.test/chat/completions",
            normalizeTranslationEndpoint(configuration("https://example.test")))
        assertEquals("https://example.test/proxy/chat/completions",
            normalizeTranslationEndpoint(configuration("https://example.test/proxy")))
        assertEquals("https://example.test/proxy/v1/chat/completions",
            normalizeTranslationEndpoint(configuration("https://example.test/proxy/v1")))
    }

    @Test fun httpRequiresAnExplicitOptInAndAKnownLoopbackAddress() {
        listOf("127.0.0.1", "localhost", "10.0.2.2", "[::1]").forEach { host ->
            val address = "http://$host:8000/v1"
            rejects("translation_https_required") { normalizeTranslationEndpoint(configuration(address)) }
            assertTrue(normalizeTranslationEndpoint(configuration(address, true)).endsWith("/v1/chat/completions"))
        }
        listOf("example.test", "private.local", "localhost.example.test", "8.8.8.8", "172.32.0.1", "192.169.1.2", "[2001:db8::1]", "172.16.0.2", "192.168.1.5", "10.0.0.2", "127.0.0.2", "[fd00::1]").forEach { host ->
            rejects("translation_https_required") { normalizeTranslationEndpoint(configuration("http://$host/v1", true)) }
        }
    }

    @Test fun destinationCannotContainUserInfoQueryFragmentOrAnotherProtocol() {
        listOf("https://user:secret@example.test/v1", "https://@example.test/v1", "https://example.test/v1?key=secret",
            "https://example.test/v1#secret", "wss://example.test/v1", "https:example.test/v1").forEach {
            rejects("translation_endpoint") { normalizeTranslationEndpoint(configuration(it)) }
        }
    }

    @Test fun modelAndCredentialValidationDoesNotExposeTheValue() {
        val config = TranslationConfiguration("https://example.test/v1", "gpt-5.4", "private-secret")
        assertFalse(config.toString().contains("private-secret"))
        rejects("translation_model") { validateTranslationConfiguration(config.copy(model = "")) }
        rejects("translation_key") { validateTranslationConfiguration(config.copy(apiKey = "private-secret\r\nHeader: value")) }
        assertEquals("https://example.test/v1/chat/completions", validateTranslationConfiguration(config.copy(apiKey = "")))
    }

    @Test fun androidEndpointPolicyUsesSharedFixturesAcrossActualJni() {
        val fixtures = org.json.JSONObject(requireNotNull(javaClass.getResourceAsStream("/translation-contracts.json"))
            .bufferedReader().use { it.readText() }).getJSONArray("androidEndpoints")
        repeat(fixtures.length()) { index ->
            val case = fixtures.getJSONObject(index)
            val provider = TextTranslationProvider.entries.single { it.runtimeRoute() == case.getString("route") }
            val config = TranslationConfiguration(endpoint = case.getString("input"), provider = provider,
                allowLocalHttp = case.getBoolean("allowLocalHttp"))
            val result = runCatching { normalizeTranslationEndpoint(config) }
            if (case.isNull("expected")) {
                val error = result.exceptionOrNull()
                assertTrue(case.getString("id"), error is IllegalArgumentException)
                assertEquals(case.getString("id"), case.getString("error"), error?.message)
            } else {
                assertEquals(case.getString("id"), case.getString("expected"), result.getOrThrow())
            }
            if (case.optBoolean("runtimeAligned")) {
                assertEquals(case.getString("id"), !case.isNull("expected"),
                    NativeRuntimeConfiguration.textEndpointsEquivalent(provider, config.endpoint,
                        if (case.isNull("expected")) config.endpoint else case.getString("expected")))
                if (!case.isNull("expected")) {
                    assertEquals(case.getString("id"), config.endpoint.trim(),
                        config.copy(model = "gpt-5.4").runtimeCredentials()?.getString("endpoint"))
                }
            }
        }
    }

    @Test fun endpointErrorsPrecedeModelAndKeyErrorsAndBoundTheJniInput() {
        val invalid = configuration("http://example.test/v1", true).copy(model = "", apiKey = "bad\nkey")
        rejects("translation_https_required") { validateTranslationConfiguration(invalid) }
        rejects("translation_endpoint") { validateTranslationConfiguration(invalid.copy(endpoint = "https://example.test/v1?key=synthetic")) }
        rejects("translation_model") { validateTranslationConfiguration(invalid.copy(endpoint = "https://example.test/v1")) }
        rejects("translation_endpoint") { normalizeTranslationEndpoint(configuration("https://example.test/" + "x".repeat(2048))) }
        rejects("translation_endpoint") { normalizeTranslationEndpoint(configuration("https://example.test/" + "界".repeat(700))) }
    }

}
