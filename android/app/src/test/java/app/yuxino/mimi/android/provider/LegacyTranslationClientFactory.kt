package app.yuxino.mimi.android.provider

/** Historical wire fixtures only; this independent factory is not packaged. */
fun createTranslationClient(config: TranslationConfiguration): TranslationClient = when (config.provider) {
    TextTranslationProvider.CHAT_MOCK, TextTranslationProvider.OPENAI_COMPATIBLE -> OpenAITranslationClient(config)
    TextTranslationProvider.DEEPL -> DeepLTranslationClient(config)
    TextTranslationProvider.DEEPLX -> DeepLXTranslationClient(config)
    TextTranslationProvider.BUILTIN, TextTranslationProvider.NONE -> throw IllegalArgumentException("translation_provider")
}
