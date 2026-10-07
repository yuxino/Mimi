import { I18N, providerDisplayName } from "./i18n";
import { activeServiceProfile, isCustomSpeechProvider, sourceLanguagesForSettings, textTranslationForProfile } from "./providerCapabilities";
import { textTranslationDisplayName } from "./textTranslationName";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot, type SourceLanguage, type TargetLanguage } from "./types";

type LanguageSettings = Pick<SettingsSnapshot, "profiles" | "activeProfileId" | "targetLanguage" | "languageCapabilities">;

/** Describe the actual request semantics, never infer a custom model's capabilities. */
export function speechLanguageGuidance(settings: LanguageSettings) {
  const profile = activeServiceProfile(settings);
  const provider = profile?.provider ?? "alibabaCloud";
  const custom = isCustomSpeechProvider(provider);
  const appleTranslation = profile && textTranslationForProfile(profile) === "apple" && settings.targetLanguage !== "original";
  const sources = sourceLanguagesForSettings(settings);
  const meaning = provider === "appleSpeech" ? I18N.settings.appleSpeechLanguageHelp : provider === "googleGeminiLive" ? I18N.settings.recognitionGeminiAutomaticHelp : provider === "volcanoEngine" ? I18N.settings.recognitionVolcanoHelp : custom ? I18N.settings.recognitionCustomHelp
    : provider === "alibabaCloud" || provider === "deepLX" || provider === "xAIRealtime"
      ? I18N.settings.recognitionHintHelp
      : sources.length === 1 && sources[0] === "auto"
        ? I18N.settings.recognitionAutomaticHelp : I18N.settings.recognitionExplicitHelp;
  const parameter = provider === "customDashScopeASR" ? I18N.settings.recognitionDashScopeParameter
    : provider === "customOpenAIASR" ? I18N.settings.recognitionOpenAIParameter : "";
  return {
    help: [meaning, parameter, appleTranslation ? I18N.settings.appleTranslationChooseSource : ""].filter(Boolean).join("\n"),
    notice: custom ? profile?.customSpeechSourceLanguages == null ? I18N.settings.recognitionCustomNotice : I18N.settings.recognitionDeclaredNotice : null,
    catalogHelp: provider === "appleSpeech" ? I18N.settings.appleSpeechLanguageHelp : I18N.settings.languageConfigurationHelp(providerDisplayName(provider), settings.targetLanguage === "original" ? I18N.settings.skipTranslation : profile ? textTranslationDisplayName(profile) : providerDisplayName(provider)),
    optionLabel: (source: SourceLanguage) => provider === "volcanoEngine" && source === "zh_en" ? I18N.settings.recognitionVolcanoBilingual : custom && source === "auto" ? I18N.settings.recognitionServiceDefault : SOURCE_LANGUAGE_DISPLAY_NAMES[source],
  };
}

/** The same stored mixed-language code has provider-specific direction semantics. */
export function targetLanguageOptionLabel(settings: LanguageSettings, target: TargetLanguage): string {
  return activeServiceProfile(settings)?.provider === "volcanoEngine" && target === "zh_en"
    ? I18N.settings.recognitionVolcanoBilingual : TARGET_LANGUAGE_DISPLAY_NAMES[target];
}
