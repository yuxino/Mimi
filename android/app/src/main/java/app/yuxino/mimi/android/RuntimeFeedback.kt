package app.yuxino.mimi.android

import app.yuxino.mimi.android.provider.SubtitleBus

/** Localized presentation of shared runtime state; never schedules or retries work. */
internal fun runtimeFeedbackResource(feedback: SubtitleBus.RuntimeFeedback): Int? = when {
    feedback.connectionStatus == "connecting" -> R.string.runtime_connecting
    feedback.recoveryReason == "rateLimited" -> if (feedback.retryScheduled)
        R.string.runtime_rate_retry else R.string.runtime_rate_wait
    feedback.recoveryReason == "temporarilyUnavailable" -> if (feedback.retryScheduled)
        R.string.runtime_unavailable_retry else R.string.runtime_unavailable_wait
    feedback.translationTimedOut -> R.string.runtime_translation_delayed
    else -> null
}
