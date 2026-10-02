package app.yuxino.mimi.android

import app.yuxino.mimi.android.provider.ServiceProvider

/** Public official help shared with desktop; reviewed 2026-09-30. */
internal data class ProviderHelp(val setup: Int, val documentation: String, val billing: String)
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
        "https://cloud.tencent.com/document/product/1093/35686")
    ServiceProvider.BAIDU -> ProviderHelp(R.string.guide_help_baidu,
        "https://ai.baidu.com/ai-doc/MT/2l317egif",
        "https://ai.baidu.com/ai-doc/MT/Tl9pjqsym")
    ServiceProvider.XAI -> ProviderHelp(R.string.guide_help_xai,
        "https://docs.x.ai/developers/quickstart",
        "https://docs.x.ai/developers/pricing")
}

internal fun providerTitle(context: android.content.Context, provider: ServiceProvider): String = context.getString(when (provider) {
    ServiceProvider.DASHSCOPE -> R.string.guide_name_dashscope
    ServiceProvider.OPENAI -> R.string.guide_name_openai
    ServiceProvider.GEMINI -> R.string.guide_name_gemini
    ServiceProvider.AZURE -> R.string.guide_name_azure
    ServiceProvider.VOLCANO -> R.string.guide_name_volcano
    ServiceProvider.TENCENT -> R.string.guide_name_tencent
    ServiceProvider.BAIDU -> R.string.guide_name_baidu
    ServiceProvider.XAI -> R.string.guide_name_xai
})

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
