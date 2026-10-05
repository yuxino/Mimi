package app.yuxino.mimi.android

import app.yuxino.mimi.android.provider.ServiceProvider
import app.yuxino.mimi.android.provider.TextTranslationProvider

/** Public official help shared with desktop; Tencent reviewed 2026-10-05. */
internal data class ProviderSetupLink(val label: Int, val url: String)
internal data class ProviderHelp(val setup: Int, val documentation: String, val billing: String,
    val setupLinks: List<ProviderSetupLink> = emptyList())
internal fun providerHelp(provider: ServiceProvider): ProviderHelp = when (provider) {
    ServiceProvider.DASHSCOPE -> ProviderHelp(R.string.guide_help_dashscope,
        "https://help.aliyun.com/zh/model-studio/get-api-key",
        "https://help.aliyun.com/zh/model-studio/model-pricing")
    ServiceProvider.OPENAI -> ProviderHelp(R.string.guide_help_openai,
        "https://developers.openai.com/api/docs/guides/realtime-translation",
        "https://developers.openai.com/api/docs/pricing")
    ServiceProvider.GEMINI -> ProviderHelp(R.string.guide_help_gemini,
        "https://ai.google.dev/gemini-api/docs/api-key",
        "https://ai.google.dev/gemini-api/docs/pricing")
    ServiceProvider.AZURE -> ProviderHelp(R.string.guide_help_azure,
        "https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/realtime-audio",
        "https://azure.microsoft.com/en-us/pricing/details/cognitive-services/openai-service/")
    ServiceProvider.VOLCANO -> ProviderHelp(R.string.guide_help_volcano,
        "https://docs.volcengine.com/docs/DoubaoVoice/SimultaneousInterpretation20APIAccessDocumentation?lang=zh",
        "https://docs.volcengine.com/docs/DoubaoVoice/BillingOverview-15?lang=zh")
    ServiceProvider.TENCENT -> ProviderHelp(R.string.guide_help_tencent,
        "https://cloud.tencent.com/document/api/1093/127565",
        "https://cloud.tencent.com/document/product/1093/35686",
        listOf(
            ProviderSetupLink(R.string.guide_tencent_app_id, "https://console.cloud.tencent.com/developer"),
            ProviderSetupLink(R.string.guide_tencent_secret_pair, "https://console.cloud.tencent.com/cam/capi"),
            ProviderSetupLink(R.string.guide_tencent_asr, "https://console.cloud.tencent.com/asr"),
        ))
    ServiceProvider.BAIDU -> ProviderHelp(R.string.guide_help_baidu,
        "https://ai.baidu.com/ai-doc/MT/2l317egif",
        "https://ai.baidu.com/ai-doc/MT/Tl9pjqsym")
    ServiceProvider.XAI -> ProviderHelp(R.string.guide_help_xai,
        "https://docs.x.ai/developers/quickstart",
        "https://docs.x.ai/developers/pricing")
}

internal fun providerTitle(context: android.content.Context, provider: ServiceProvider): String {
    if (provider == ServiceProvider.DASHSCOPE) {
        when (val translation = runCatching { SettingsStore.textTranslationProvider(context) }.getOrDefault(TextTranslationProvider.BUILTIN)) {
            TextTranslationProvider.BUILTIN -> Unit
            TextTranslationProvider.NONE -> return context.getString(R.string.translation_service_original)
            else -> return context.getString(R.string.translation_service_name, context.getString(
                if (translation == TextTranslationProvider.OPENAI_COMPATIBLE) R.string.translation_service_custom else translationProviderLabel(translation)))
        }
    }
    return context.getString(when (provider) {
    ServiceProvider.DASHSCOPE -> R.string.guide_name_dashscope
    ServiceProvider.OPENAI -> R.string.guide_name_openai
    ServiceProvider.GEMINI -> R.string.guide_name_gemini
    ServiceProvider.AZURE -> R.string.guide_name_azure
    ServiceProvider.VOLCANO -> R.string.guide_name_volcano
    ServiceProvider.TENCENT -> R.string.guide_name_tencent
    ServiceProvider.BAIDU -> R.string.guide_name_baidu
    ServiceProvider.XAI -> R.string.guide_name_xai
})
}

internal fun providerDescription(context: android.content.Context, provider: ServiceProvider): String = context.getString(when (provider) {
    ServiceProvider.DASHSCOPE -> R.string.guide_detail_dashscope
    ServiceProvider.OPENAI -> R.string.guide_detail_openai
    ServiceProvider.GEMINI -> R.string.guide_detail_gemini
    ServiceProvider.AZURE -> R.string.guide_detail_azure
    ServiceProvider.VOLCANO -> R.string.guide_detail_volcano
    ServiceProvider.TENCENT -> R.string.guide_detail_tencent
    ServiceProvider.BAIDU -> R.string.guide_detail_baidu
    ServiceProvider.XAI -> R.string.guide_detail_xai
})
