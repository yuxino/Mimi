import { textTranslationDisplayName } from "../../lib/textTranslationName";
import { speechRecognitionDisplayName } from "../../lib/speechRecognitionName";
import { I18N } from "../../lib/i18n";
import {
  activeServiceProfile,
  isCustomSpeechProvider,
  isIndependentAsrProfile,
  textTranslationForProfile,
} from "../../lib/providerCapabilities";
import type { ServiceProvider, SettingsSnapshot, TextTranslation } from "../../lib/types";

export interface OverlayServiceStage {
  role: "recognition" | "translation" | "combined";
  provider: ServiceProvider | Exclude<TextTranslation, "followService">;
  label: string;
}

export interface TranslationService {
  stages: OverlayServiceStage[];
  detail: string;
  showInProfileDetails: boolean;
}

/** Describes the selected route using only credential-free snapshot metadata. */
export function translationService(
  settings: Pick<SettingsSnapshot, "profiles" | "activeProfileId" | "targetLanguage">,
): TranslationService | null {
  const profile = activeServiceProfile(settings);
  if (!profile) return null;

  const route = textTranslationForProfile(profile);
  // Legacy DeepLX profiles use Alibaba recognition plus independent DeepLX
  // translation; DeepLX must never be presented as a speech recognizer.
  const speechProvider = profile.provider === "deepLX" ? "alibabaCloud" : profile.provider;
  const speechLabel = speechRecognitionDisplayName(profile);
  const originalOnly = settings.targetLanguage === "original"
    || (isIndependentAsrProfile(profile) && route === "followService");
  const translationLabel = originalOnly ? I18N.overlay.originalOnly : textTranslationDisplayName(profile);
  const sameXai = profile.provider === "xAIRealtime" && route === "xAI";
  const stages: OverlayServiceStage[] = [{
    role: !originalOnly && (route === "followService" || sameXai) ? "combined" : "recognition",
    provider: speechProvider,
    label: speechLabel,
  }];
  // Matching names or compatible protocols do not establish that two
  // independently configured endpoints are the same service.
  if (!originalOnly && route !== "followService" && !sameXai) {
    stages.push({ role: "translation", provider: route, label: translationLabel });
  }

  return {
    stages,
    // The selected profile already shows its built-in provider mark.
    showInProfileDetails: stages.length > 1 || isCustomSpeechProvider(profile.provider),
    detail: [
      `${I18N.settings.currentProfile}: ${profile.name}`,
      `${I18N.settings.speechRecognition}: ${speechLabel}`,
      `${I18N.settings.textTranslationLabel}: ${translationLabel}`,
    ].join("\n"),
  };
}
