package app.yuxino.mimi.android.capture

import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.provider.CREDENTIAL_AUTHENTICATION_FAILED
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

    @Test fun nativeAuthenticationFailureGivesAnActionableCredentialMessage() {
        assertEquals(R.string.service_authentication_failed, providerErrorMessageResource(CREDENTIAL_AUTHENTICATION_FAILED))
    }

    @Test fun networkFailuresAndUnrecognizedProviderLabelsStaySanitized() {
        for (code in listOf("", "transport_error", "provider_error", "private-provider-message", "credential_authentication_failed: private-token")) {
            assertEquals(R.string.capture_failed, providerErrorMessageResource(code))
        }
    }
}
