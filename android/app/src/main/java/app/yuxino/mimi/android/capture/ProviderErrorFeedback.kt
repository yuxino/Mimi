package app.yuxino.mimi.android.capture

import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.provider.CREDENTIAL_AUTHENTICATION_FAILED

/** Provider messages are deliberately excluded from user-facing feedback. */
internal fun providerErrorMessageResource(code: String): Int = when (code) {
    CREDENTIAL_AUTHENTICATION_FAILED -> R.string.service_authentication_failed
    "openai_realtime_setup_expired", "openai_realtime_session_expired" -> R.string.openai_realtime_session_expired
    "openai_realtime_permission_denied" -> R.string.openai_realtime_permission_denied
    "openai_realtime_quota_exhausted" -> R.string.openai_realtime_quota_exhausted
    "openai_realtime_rate_limited" -> R.string.openai_realtime_rate_limited
    "openai_realtime_model_unavailable" -> R.string.openai_realtime_model_unavailable
    "openai_realtime_invalid_request" -> R.string.openai_realtime_invalid_request
    "openai_realtime_service_error", "openai_realtime_service_unavailable" -> R.string.openai_realtime_service_error
    "tencent_configuration_rejected" -> R.string.tencent_configuration_rejected
    "tencent_service_activation_required" -> R.string.tencent_service_activation_required
    "tencent_quota_exhausted" -> R.string.tencent_quota_exhausted
    "tencent_capacity_exceeded" -> R.string.tencent_capacity_exceeded
    "tencent_provider_rejected" -> R.string.tencent_provider_rejected
    else -> R.string.capture_failed
}
