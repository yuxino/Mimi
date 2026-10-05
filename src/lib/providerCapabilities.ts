import {
  AUDIO3_RECOGNITION_LANGUAGE_CODES,
  QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES,
  LEGACY_SOURCE_LANGUAGE_CASES,
  TRANSLATION_MODE_CASES,
  type ProviderCapabilities,
  type CredentialState,
  type ServiceProfile,
  type ServiceProvider,
  type SettingsSnapshot,
  type SourceLanguage,
  type TargetLanguage,
  type TextTranslation,
  type TranslationMode,
} from "./types";

type LanguageSettings = Pick<SettingsSnapshot, "profiles" | "activeProfileId"> &
  Partial<Pick<SettingsSnapshot, "targetLanguage" | "languageCapabilities">>;

const LITE_LANGUAGE_CODES = new Set<string>(QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES);
// Protocol values Mimi can encode. This is not any custom endpoint's model catalog.
const CUSTOM_CONFIGURABLE_SOURCES: readonly SourceLanguage[] = ["auto", "zh", "en", "ja", "ko", "vi", "th", "id", "ms", "tl", "hi", "ar", "fr", "de", "es", "pt", "ru", "it", "nl", "sv", "da", "fi", "no", "el", "pl", "cs", "hu", "ro", "bg", "hr", "sk"];
const ALIBABA_RECOGNITION_SOURCES: readonly SourceLanguage[] = ["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES];
const ALIBABA_TRANSLATION_SOURCES: readonly SourceLanguage[] = [
  "auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES.filter((code) => LITE_LANGUAGE_CODES.has(code)),
];
export const OPENAI_TRANSLATION_TARGETS: readonly TargetLanguage[] = ["zh", "en", "ja", "ko", "ru", "es", "fr", "pt", "de", "it", "vi", "id", "hi"];
export const GEMINI_TRANSLATION_TARGETS: readonly TargetLanguage[] = ["zh", "en", "ja", "zh_tw", "ko", "ru", "es", "fr", "de", "it", "th", "vi", "id", "ms", "ar", "hi", "he", "ur", "bn", "pl", "nl", "tr", "km", "cs", "sv", "hu", "da", "fi", "tl", "fa"];
export const XAI_RECOGNITION_SOURCES: readonly SourceLanguage[] = ["auto", "zh", "en", "ja", "ko", "vi", "id", "hi", "fr", "de", "ru", "it"];
const ALIBABA_TARGETS: readonly TargetLanguage[] = ["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES];
const LEGACY_ALIBABA_CAPABILITIES: ProviderCapabilities = {
  sourceLanguages: LEGACY_SOURCE_LANGUAGE_CASES,
  targetLanguages: ["original", "zh", "en", "ja"],
  translationModes: TRANSLATION_MODE_CASES,
};
const SOURCE_CODES = new Set<string>(ALIBABA_RECOGNITION_SOURCES);
const TARGET_CODES = new Set<string>(ALIBABA_TARGETS);

export const SERVICE_PROVIDERS: readonly ServiceProvider[] = [
  "alibabaCloud",
  "googleGeminiLive",
  "volcanoEngine",
  "tencentCloud",
  "baiduTranslate",
  "openAIRealtime",
  "azureOpenAIRealtime",
  "xAIRealtime",
  "appleSpeech",
  "customDashScopeASR",
  "customOpenAIASR",
];

const PROVIDER_CAPABILITIES: Readonly<
  Record<ServiceProvider, ProviderCapabilities>
> = {
  appleSpeech: { sourceLanguages: [], targetLanguages: ["original"], translationModes: ["turbo"] },
  customDashScopeASR: { sourceLanguages: CUSTOM_CONFIGURABLE_SOURCES, targetLanguages: ["original"], translationModes: ["turbo"] },
  customOpenAIASR: { sourceLanguages: CUSTOM_CONFIGURABLE_SOURCES, targetLanguages: ["original"], translationModes: ["turbo"] },
  deepLX: { sourceLanguages: LEGACY_SOURCE_LANGUAGE_CASES, targetLanguages: ["zh", "en", "ja"], translationModes: ["turbo"] },
  alibabaCloud: {
    sourceLanguages: ALIBABA_TRANSLATION_SOURCES,
    targetLanguages: ALIBABA_TARGETS,
    translationModes: TRANSLATION_MODE_CASES,
  },
  openAIRealtime: {
    sourceLanguages: ["auto"],
    targetLanguages: OPENAI_TRANSLATION_TARGETS,
    translationModes: ["turbo"],
  },
  googleGeminiLive: {
    sourceLanguages: ["auto"],
    targetLanguages: GEMINI_TRANSLATION_TARGETS,
    translationModes: ["turbo"],
  },
  azureOpenAIRealtime: {
    sourceLanguages: ["auto"],
    targetLanguages: ["zh", "en", "ja"],
    translationModes: ["turbo"],
  },
  volcanoEngine: {
    sourceLanguages: ["ja", "en", "zh"],
    targetLanguages: ["zh", "en", "ja"],
    translationModes: ["turbo"],
  },
  tencentCloud: {
    sourceLanguages: ["ja", "en", "ko", "zh"],
    targetLanguages: ["zh", "en", "ja"],
    translationModes: ["turbo"],
  },
  baiduTranslate: {
    sourceLanguages: ["ja", "en", "ko", "zh"],
    targetLanguages: ["zh", "en", "ja"],
    translationModes: ["turbo"],
  },
  xAIRealtime: {
    sourceLanguages: XAI_RECOGNITION_SOURCES,
    targetLanguages: ["zh", "en", "ja"],
    translationModes: ["turbo"],
  },
};

export function capabilitiesForProvider(
  provider: ServiceProvider,
): ProviderCapabilities {
  return PROVIDER_CAPABILITIES[provider];
}

export function activeServiceProfile(
  settings: Pick<SettingsSnapshot, "profiles" | "activeProfileId">,
): ServiceProfile | undefined {
  return settings.profiles.find(
    (profile) => profile.id === settings.activeProfileId,
  );
}

export function textTranslationForProfile(profile: ServiceProfile): TextTranslation {
  return profile.textTranslation ?? (profile.provider === "deepLX" ? "deepLX" : "followService");
}

export function isChatCompletionsTranslation(translation: TextTranslation): translation is "openAICompatible" | "chatMock" {
  return translation === "openAICompatible" || translation === "chatMock";
}

export function isCustomSpeechProvider(provider: ServiceProvider): boolean {
  return provider === "customDashScopeASR" || provider === "customOpenAIASR";
}

/** User declarations only narrow encodable options; they never certify model support. */
function declaredCustomSources(profile: ServiceProfile, sources: readonly SourceLanguage[]): readonly SourceLanguage[] {
  if (!isCustomSpeechProvider(profile.provider)) return sources;
  const declared = profile.customSpeechSourceLanguages;
  return ["auto", ...sources.filter(source => source !== "auto" && (declared == null || declared.includes(source)))];
}

export function isStandaloneAsrProvider(provider: ServiceProvider): boolean {
  return provider === "appleSpeech" || isCustomSpeechProvider(provider);
}

export function credentialStateForTarget(profile: ServiceProfile | undefined, target: TargetLanguage): CredentialState {
  return profile && isStandaloneAsrProvider(profile.provider) && target === "original"
    ? profile.speechCredentialState ?? profile.credentialState : profile?.credentialState ?? "unavailable";
}

export function effectiveProviderForProfile(profile: ServiceProfile): ServiceProvider {
  if (profile.provider !== "alibabaCloud" && profile.provider !== "deepLX") return profile.provider;
  return textTranslationForProfile(profile) === "deepLX" ? "deepLX" : "alibabaCloud";
}

/** Local fallback for older snapshots and inactive-profile details. Routes keep independent ranges. */
export function capabilitiesForProfile(
  profile: ServiceProfile,
  targetLanguage: TargetLanguage = "zh",
): ProviderCapabilities {
  if (isStandaloneAsrProvider(profile.provider)) {
    const capabilities = capabilitiesForProvider(profile.provider);
    const route = textTranslationForProfile(profile);
    const generic = isChatCompletionsTranslation(route);
    return { ...capabilities,
      sourceLanguages: declaredCustomSources(profile, profile.provider === "appleSpeech" || targetLanguage === "original" || route === "followService" || generic ? capabilities.sourceLanguages : LEGACY_SOURCE_LANGUAGE_CASES),
      targetLanguages: route === "followService" ? capabilities.targetLanguages : generic ? ALIBABA_TARGETS : ["original", "zh", "en", "ja"],
    };
  }
  if (profile.provider !== "alibabaCloud" && profile.provider !== "deepLX") {
    return capabilitiesForProvider(profile.provider);
  }
  const route = textTranslationForProfile(profile);
  if (isChatCompletionsTranslation(route)) return { ...LEGACY_ALIBABA_CAPABILITIES, sourceLanguages: ALIBABA_RECOGNITION_SOURCES, targetLanguages: ALIBABA_TARGETS };
  if (route === "deepL") return LEGACY_ALIBABA_CAPABILITIES;
  if (route === "deepLX") return PROVIDER_CAPABILITIES.deepLX;
  const capabilities = PROVIDER_CAPABILITIES.alibabaCloud;
  return targetLanguage === "original"
    ? { ...capabilities, sourceLanguages: ALIBABA_RECOGNITION_SOURCES }
    : capabilities;
}

function capabilitiesForSettings(
  settings: LanguageSettings,
): ProviderCapabilities {
  const profile = activeServiceProfile(settings);
  const target = settings.targetLanguage ?? "zh";
  const fallback = profile
    ? capabilitiesForProfile(profile, target)
    : target === "original"
      ? { ...PROVIDER_CAPABILITIES.alibabaCloud, sourceLanguages: ALIBABA_RECOGNITION_SOURCES }
      : PROVIDER_CAPABILITIES.alibabaCloud;
  const native = settings.languageCapabilities;
  // A profile can retain its ID while changing providers or text destinations.
  // Never borrow options stamped for a different route/target or stale profile.
  // Custom declarations are locally computable and absent from the native stamp;
  // an older narrow list must not override a newly expanded or cleared declaration.
  if (
    profile && !isCustomSpeechProvider(profile.provider) && native &&
    native.profileId === profile.id &&
    native.provider === profile.provider &&
    native.textTranslation === textTranslationForProfile(profile) &&
    native.targetLanguage === target &&
    Array.isArray(native.sourceLanguages) && (profile.provider === "appleSpeech" || native.sourceLanguages.length > 0) &&
    Array.isArray(native.targetLanguages) && native.targetLanguages.length > 0 &&
    native.sourceLanguages.every((code) => SOURCE_CODES.has(code) && (profile.provider !== "appleSpeech" || code !== "auto")) &&
    native.targetLanguages.every((code) => TARGET_CODES.has(code))
  ) {
    return {
      sourceLanguages: declaredCustomSources(profile, [...new Set(native.sourceLanguages)].filter(source =>
        profile.provider !== "appleSpeech" || target === "original"
        || !["deepL", "deepLX"].includes(textTranslationForProfile(profile))
        || LEGACY_SOURCE_LANGUAGE_CASES.includes(source))),
      targetLanguages: [...new Set(native.targetLanguages)],
      translationModes: fallback.translationModes,
    };
  }
  return fallback;
}

export function sourceLanguagesForSettings(
  settings: LanguageSettings,
): readonly SourceLanguage[] {
  return capabilitiesForSettings(settings).sourceLanguages;
}

export function targetLanguagesForSettings(
  settings: LanguageSettings & Pick<SettingsSnapshot, "sourceLanguage">,
): readonly TargetLanguage[] {
  const targetLanguages = capabilitiesForSettings(settings).targetLanguages;
  if (
    settings.sourceLanguage === "auto" ||
    targetLanguages.includes("original")
  ) {
    return targetLanguages;
  }
  return targetLanguages.filter(
    (target) => !sourceMatchesTarget(settings.sourceLanguage, target),
  );
}

function sourceMatchesTarget(
  source: SourceLanguage,
  target: TargetLanguage,
): boolean {
  return (
    (source === "zh" && target === "zh") ||
    (source === "en" && target === "en") ||
    (source === "ja" && target === "ja")
  );
}

export function targetLanguageAfterSourceSwitch(
  settings: Pick<
    SettingsSnapshot,
    | "profiles"
    | "activeProfileId"
    | "sourceLanguage"
    | "targetLanguage"
  >,
  sourceLanguage: SourceLanguage,
): TargetLanguage {
  const capabilities = capabilitiesForSettings(settings);
  if (
    capabilities.targetLanguages.includes(settings.targetLanguage) &&
    (capabilities.targetLanguages.includes("original") ||
      !sourceMatchesTarget(sourceLanguage, settings.targetLanguage))
  ) {
    return settings.targetLanguage;
  }

  return (
    capabilities.targetLanguages.find(
      (target) => !sourceMatchesTarget(sourceLanguage, target),
    ) ?? settings.targetLanguage
  );
}

export function translationModesForSettings(
  settings: Pick<
    SettingsSnapshot,
    "profiles" | "activeProfileId" | "sourceLanguage"
  >,
): readonly TranslationMode[] {
  return capabilitiesForSettings(settings).translationModes;
}

/** Legacy preferences remain readable; the active pipeline always uses Turbo. */
export function effectiveTranslationModeForSettings(
  settings: Pick<
    SettingsSnapshot,
    | "profiles"
    | "activeProfileId"
    | "sourceLanguage"
    | "translationMode"
  >,
): TranslationMode {
  return translationModesForSettings(settings)[0] ?? "turbo";
}

export function subtitlePreferencesChanged(
  before: Pick<
    SettingsSnapshot,
    "sourceLanguage" | "targetLanguage" | "translationMode"
  >,
  after: Pick<
    SettingsSnapshot,
    "sourceLanguage" | "targetLanguage" | "translationMode"
  >,
): boolean {
  return (
    before.sourceLanguage !== after.sourceLanguage ||
    before.targetLanguage !== after.targetLanguage ||
    before.translationMode !== after.translationMode
  );
}
