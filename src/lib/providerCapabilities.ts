import languageCatalogs from "../../shared/provider-language-catalogs.json";
import {
  AUDIO3_RECOGNITION_LANGUAGE_CODES,
  QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES,
  SOURCE_LANGUAGE_CODES,
  TARGET_LANGUAGE_CODES,
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
const CUSTOM_CONFIGURABLE_SOURCES: readonly SourceLanguage[] = ["auto", ...SOURCE_LANGUAGE_CODES];
const ALIBABA_RECOGNITION_SOURCES: readonly SourceLanguage[] = ["auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES];
const ALIBABA_TRANSLATION_SOURCES: readonly SourceLanguage[] = [
  "auto", ...AUDIO3_RECOGNITION_LANGUAGE_CODES.filter((code) => LITE_LANGUAGE_CODES.has(code)),
];
export const OPENAI_TRANSLATION_TARGETS = languageCatalogs.openAIRealtime.targetLanguages as readonly TargetLanguage[];
export const GEMINI_TRANSLATION_TARGETS = languageCatalogs.googleGeminiLive.targetLanguages as readonly TargetLanguage[];
export const XAI_RECOGNITION_SOURCES = languageCatalogs.xAIRealtime.sourceLanguages as readonly SourceLanguage[];
const ALIBABA_TARGETS: readonly TargetLanguage[] = ["original", ...QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES];
const CUSTOM_CONFIGURABLE_TARGETS: readonly TargetLanguage[] = ["original", ...TARGET_LANGUAGE_CODES];
const SOURCE_CODES = new Set<string>(CUSTOM_CONFIGURABLE_SOURCES);
const TARGET_CODES = new Set<string>(CUSTOM_CONFIGURABLE_TARGETS);

export function textTranslationSourceLanguages(route: "deepL" | "deepLX"): readonly SourceLanguage[] {
  return ["auto", ...languageCatalogs[route].sourceLanguages as SourceLanguage[]];
}

function textTranslationTargetLanguages(route: "deepL" | "deepLX", original: boolean): readonly TargetLanguage[] {
  return [...(original ? ["original" as const] : []), ...languageCatalogs[route].targetLanguages as TargetLanguage[]];
}

function intersectTranslationSources(sources: readonly SourceLanguage[], route: "deepL" | "deepLX"): readonly SourceLanguage[] {
  const supported = textTranslationSourceLanguages(route);
  return sources.filter(source => supported.includes(source));
}

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
  deepLX: { sourceLanguages: intersectTranslationSources(ALIBABA_RECOGNITION_SOURCES, "deepLX"), targetLanguages: textTranslationTargetLanguages("deepLX", false), translationModes: ["turbo"] },
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
    targetLanguages: OPENAI_TRANSLATION_TARGETS,
    translationModes: ["turbo"],
  },
  volcanoEngine: {
    sourceLanguages: languageCatalogs.volcanoEngine.sourceLanguages as SourceLanguage[],
    targetLanguages: languageCatalogs.volcanoEngine.targetLanguages as TargetLanguage[],
    translationModes: ["turbo"],
  },
  tencentCloud: {
    sourceLanguages: languageCatalogs.tencentCloud.sourceLanguages as SourceLanguage[],
    targetLanguages: languageCatalogs.tencentCloud.targetLanguages as TargetLanguage[],
    translationModes: ["turbo"],
  },
  baiduTranslate: {
    sourceLanguages: languageCatalogs.baiduTranslate.sourceLanguages as SourceLanguage[],
    targetLanguages: languageCatalogs.baiduTranslate.targetLanguages as TargetLanguage[],
    translationModes: ["turbo"],
  },
  xAIRealtime: {
    sourceLanguages: XAI_RECOGNITION_SOURCES,
    targetLanguages: languageCatalogs.xAIRealtime.targetLanguages as TargetLanguage[],
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
  return sources.filter(source => source === "auto" || declared == null || declared.includes(source));
}

export function isStandaloneAsrProvider(provider: ServiceProvider): boolean {
  return provider === "appleSpeech" || isCustomSpeechProvider(provider);
}

export function credentialStateForTarget(profile: ServiceProfile | undefined, target: TargetLanguage): CredentialState {
  return profile && isStandaloneAsrProvider(profile.provider) && target === "original"
    ? profile.speechCredentialState ?? profile.credentialState : profile?.credentialState ?? "missing";
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
  if (textTranslationForProfile(profile) === "apple" && (isStandaloneAsrProvider(profile.provider) || ["alibabaCloud", "deepLX"].includes(profile.provider))) {
    // Only the native runtime can certify Apple's catalog. An unavailable or
    // not-yet-loaded catalog must not borrow a cloud provider's language list.
    return {
      sourceLanguages: targetLanguage === "original" ? declaredCustomSources(profile, profile.provider === "appleSpeech" ? [] : isCustomSpeechProvider(profile.provider) ? CUSTOM_CONFIGURABLE_SOURCES : ALIBABA_RECOGNITION_SOURCES) : [],
      targetLanguages: ["original"],
      translationModes: ["turbo"],
    };
  }
  if (isStandaloneAsrProvider(profile.provider)) {
    const capabilities = capabilitiesForProvider(profile.provider);
    const route = textTranslationForProfile(profile);
    const generic = isChatCompletionsTranslation(route);
    return { ...capabilities,
      sourceLanguages: declaredCustomSources(profile, targetLanguage === "original" || route === "followService" || generic || route === "apple" ? capabilities.sourceLanguages : intersectTranslationSources(capabilities.sourceLanguages, route)),
      targetLanguages: route === "followService" ? capabilities.targetLanguages : generic ? CUSTOM_CONFIGURABLE_TARGETS : route === "apple" ? ["original"] : textTranslationTargetLanguages(route, true),
    };
  }
  if (profile.provider !== "alibabaCloud" && profile.provider !== "deepLX") {
    return capabilitiesForProvider(profile.provider);
  }
  const route = textTranslationForProfile(profile);
  if (isChatCompletionsTranslation(route)) return { sourceLanguages: ALIBABA_RECOGNITION_SOURCES, targetLanguages: CUSTOM_CONFIGURABLE_TARGETS, translationModes: TRANSLATION_MODE_CASES };
  if (route === "deepL") return { sourceLanguages: targetLanguage === "original" ? ALIBABA_RECOGNITION_SOURCES : intersectTranslationSources(ALIBABA_RECOGNITION_SOURCES, route), targetLanguages: textTranslationTargetLanguages(route, true), translationModes: TRANSLATION_MODE_CASES };
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
  // Apple needs a native catalog even for custom recognition. A declaration
  // stamp prevents an old catalog overriding a newly edited recognition list.
  const appleTranslation = profile && textTranslationForProfile(profile) === "apple";
  const customStampMatches = profile && JSON.stringify(native?.customSpeechSourceLanguages ?? null) === JSON.stringify(profile.customSpeechSourceLanguages ?? null);
  if (
    profile && (!isCustomSpeechProvider(profile.provider) || (appleTranslation && customStampMatches)) && native &&
    native.profileId === profile.id &&
    native.provider === profile.provider &&
    native.textTranslation === textTranslationForProfile(profile) &&
    native.targetLanguage === target &&
    Array.isArray(native.sourceLanguages) && (profile.provider === "appleSpeech" || appleTranslation || native.sourceLanguages.length > 0) &&
    Array.isArray(native.targetLanguages) && native.targetLanguages.length > 0 &&
    native.sourceLanguages.every((code) => SOURCE_CODES.has(code) && ((profile.provider !== "appleSpeech" && (!appleTranslation || target === "original")) || code !== "auto")) &&
    native.targetLanguages.every((code) => TARGET_CODES.has(code))
  ) {
    return {
      sourceLanguages: declaredCustomSources(profile, [...new Set(native.sourceLanguages)]),
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
  const provider = activeServiceProfile(settings)?.provider;
  if (provider === "tencentCloud" || provider === "volcanoEngine") {
    const pairs: Readonly<Record<string, readonly string[]>> = languageCatalogs[provider].targetsBySource;
    const supported = pairs[settings.sourceLanguage];
    return supported ? supported as readonly TargetLanguage[] : targetLanguages;
  }
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
  return source === target;
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
  const targets = targetLanguagesForSettings({ ...settings, sourceLanguage });
  return targets.includes(settings.targetLanguage)
    ? settings.targetLanguage
    : targets[0] ?? settings.targetLanguage;
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
