import { I18N, providerDisplayName } from "../../../lib/i18n";
import {
  isCustomSpeechProvider,
  isIndependentAsrProfile,
  textTranslationForProfile,
} from "../../../lib/providers/providerCapabilities";
import { speechRecognitionDisplayName } from "../../../lib/providers/speechRecognitionName";
import { textTranslationDisplayName } from "../../../lib/providers/textTranslationName";
import type { CredentialState, ServiceProfile, ServiceProvider } from "../../../lib/types";

function profileProviderName(profile: ServiceProfile): string {
  return providerDisplayName(profile.provider === "deepLX" ? "alibabaCloud" : profile.provider);
}

export function profileTitle(profile: ServiceProfile): string {
  return profile.provider === "deepLX" && profile.name === "DeepLX (Audio 3.0 ASR)"
    ? profileProviderName(profile)
    : profile.name;
}

export function profileDescription(profile: ServiceProfile): string {
  if(profile.provider === "appleSpeech") return I18N.settings.appleSpeechDescription;
  if(isCustomSpeechProvider(profile.provider)) {
    return [I18N.settings.customSpeechDescription, I18N.settings.customSpeechLanguages].join("\n");
  }
  const translation = textTranslationForProfile(profile);
  if(isIndependentAsrProfile(profile)) {
    return [
      `${I18N.settings.speechRecognition}: ${speechRecognitionDisplayName(profile)}`,
      `${I18N.settings.textTranslationLabel}: ${translation === "followService"
        ? I18N.settings.customSpeechNoTranslation
        : textTranslationDisplayName(profile)}`,
      I18N.settings.customSpeechTranslationHelp,
    ].join(
      "\n"
    );
  }
  return translation === "deepL"
    ? I18N.settings.deepLChain
    : translation === "deepLX"
      ? I18N.settings.deepLXChain
      : (translation === "openAICompatible" || translation === "chatMock")
        ? I18N.settings.openAICompatibleChain
        : providerDescription(profile.provider);
}

export function profileSecondaryLabel(profile: ServiceProfile): string | null {
  // The primary mark already identifies the provider. Only a user-defined
  // recognizer alias adds information beyond the configuration name and mark.
  const alias = profile.speechRecognitionName?.trim();
  return isCustomSpeechProvider(profile.provider) && alias
    && alias.toLowerCase() !== profileTitle(profile).trim().toLowerCase()
    ? alias
    : null;
}

export function providerDescription(provider: ServiceProvider): string {
  switch(provider) {
    case "appleSpeech":
      return I18N.settings.appleSpeechDescription;
    case "customDashScopeASR":
      return [I18N.settings.customSpeechRequirementsDashScope, I18N.settings.customSpeechLanguages].join("\n");
    case "customOpenAIASR":
      return [I18N.settings.customSpeechRequirementsOpenAI, I18N.settings.customSpeechLanguages].join("\n");
    case "alibabaCloud":
      return I18N.settings.providerAlibabaDescription;
    case "openAIRealtime":
      return I18N.settings.providerOpenAIDescription;
    case "googleGeminiLive":
      return I18N.settings.providerGoogleGeminiDescription;
    case "azureOpenAIRealtime":
      return I18N.settings.providerAzureOpenAIDescription;
    case "volcanoEngine":
      return I18N.settings.providerVolcanoEngineDescription;
    case "tencentCloud":
      return I18N.settings.providerTencentCloudDescription;
    case "baiduTranslate":
      return I18N.settings.providerBaiduTranslateDescription;
    case "deepLX":
      return I18N.settings.providerDeepLXDescription;
    case "xAIRealtime":
      return I18N.settings.providerXAIDescription;
  }
}

export function credentialStateText(state: CredentialState, nativeSpeech = false): string {
  if(nativeSpeech) {
    return state === "present"
      ? I18N.settings.appleSpeechServiceReady
      : state === "missing"
        ? I18N.settings.appleSpeechNeedsSetup
        : I18N.settings.appleSpeechUnavailable;
  }
  switch(state) {
    case "present":
      return I18N.settings.credentialPresent;
    case "missing":
      return I18N.settings.credentialMissing;
    case "unavailable":
      return I18N.settings.credentialUnavailable;
  }
}
