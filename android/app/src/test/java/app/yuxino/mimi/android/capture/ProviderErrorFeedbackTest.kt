package app.yuxino.mimi.android.capture

import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.provider.websocketFailureCode
import org.junit.Assert.*
import org.junit.Test

class ProviderErrorFeedbackTest {
    @Test fun tencentFailuresExplainTheRequiredAccountOrConfigurationAction() {
        for ((code, message) in mapOf(
            "tencent_configuration_rejected" to R.string.tencent_configuration_rejected,
            "tencent_service_activation_required" to R.string.tencent_service_activation_required,
            "tencent_quota_exhausted" to R.string.tencent_quota_exhausted,
            "tencent_capacity_exceeded" to R.string.tencent_capacity_exceeded,
            "tencent_provider_rejected" to R.string.tencent_provider_rejected,
        )) assertEquals(message, providerErrorMessageResource(code))
    }

    @Test fun handshakeAuthenticationFailuresGiveAnActionableCredentialMessage() {
        for (status in listOf(401, 403)) {
            assertEquals(R.string.service_authentication_failed, providerErrorMessageResource(websocketFailureCode(status)))
        }
    }

    @Test fun networkFailuresAndUnrecognizedProviderLabelsStaySanitized() {
        for (status in listOf(429, 500, null)) {
            assertEquals(R.string.capture_failed, providerErrorMessageResource(websocketFailureCode(status)))
        }
        for (code in listOf("", "provider_error", "private-provider-message", "credential_authentication_failed: private-token")) {
            assertEquals(R.string.capture_failed, providerErrorMessageResource(code))
        }
    }
}
