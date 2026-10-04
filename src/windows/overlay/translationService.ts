import { textTranslationDisplayName } from "../../lib/textTranslationName";
import { I18N, providerDisplayName } from "../../lib/i18n";
import {
  activeServiceProfile,
  isCustomSpeechProvider,
  textTranslationForProfile,
} from "../../lib/providerCapabilities";
import type { ServiceProvider, SettingsSnapshot, TextTranslation } from "../../lib/types";

export interface TranslationService {
  provider: ServiceProvider | Exclude<TextTranslation, "followService"> | null;
  label: string;
  detail: string;
}

/** Describes the selected route using only credential-free snapshot metadata. */
export function translationService(
  settings: Pick<SettingsSnapshot, "profiles" | "activeProfileId" | "targetLanguage">,
): TranslationService | null {
  const profile = activeServiceProfile(settings);
  if (!profile) return null;

  const route = textTranslationForProfile(profile);
  const originalOnly = settings.targetLanguage === "original"
    || (isCustomSpeechProvider(profile.provider) && route === "followService");
  const provider = originalOnly ? null : route === "followService"
    ? (profile.provider === "deepLX" ? "alibabaCloud" : profile.provider)
    : route;
  const label = provider === null ? I18N.overlay.originalOnly
    : textTranslationDisplayName(profile);

  return {
    provider,
    label,
    detail: [
      `${I18N.settings.currentProfile}: ${profile.name}`,
      `${I18N.settings.speechRecognition}: ${providerDisplayName(profile.provider)}`,
      `${I18N.settings.textTranslationLabel}: ${label}`,
    ].join("\n"),
  };
}
