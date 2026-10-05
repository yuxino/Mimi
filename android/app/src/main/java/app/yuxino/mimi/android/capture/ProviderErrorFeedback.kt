package app.yuxino.mimi.android.capture

import app.yuxino.mimi.android.R
import app.yuxino.mimi.android.provider.CREDENTIAL_AUTHENTICATION_FAILED

/** Provider messages are deliberately excluded from user-facing feedback. */
internal fun providerErrorMessageResource(code: String): Int = when (code) {
    CREDENTIAL_AUTHENTICATION_FAILED -> R.string.service_authentication_failed
    "tencent_configuration_rejected" -> R.string.tencent_configuration_rejected
    "tencent_service_activation_required" -> R.string.tencent_service_activation_required
    "tencent_quota_exhausted" -> R.string.tencent_quota_exhausted
    "tencent_capacity_exceeded" -> R.string.tencent_capacity_exceeded
    "tencent_provider_rejected" -> R.string.tencent_provider_rejected
    else -> R.string.capture_failed
}
