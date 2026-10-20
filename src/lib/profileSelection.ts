import { I18N } from "./i18n";
import { profileLanguagePresetLabel } from "./profileLanguagePreset";
import { credentialStateForTarget, usesAlibabaRealtime } from "./providerCapabilities";
import type { ServiceProfile, TargetLanguage } from "./types";

/** Saved-profile switching uses snapshot metadata, never credential reads or probes. */
export function profileSelectionState(profile: ServiceProfile, targetLanguage: TargetLanguage) {
  const state = credentialStateForTarget(profile, profile.languagePreset?.targetLanguage ?? targetLanguage);
  let reason: string | undefined;
  if (state === "missing") {
    reason = profile.provider === "appleSpeech" ? I18N.settings.appleSpeechNeedsSetup : I18N.settings.credentialMissing;
  } else if (state === "unavailable") {
    // The aggregate state can refer to independent text credentials, even
    // while native recognition remains available for Original-only use.
    reason = profile.provider === "appleSpeech" && profile.speechCredentialState === "unavailable"
      ? I18N.settings.appleSpeechUnavailable : I18N.settings.credentialUnavailable;
  } else if (usesAlibabaRealtime(profile) && !profile.alibabaWorkspaceId?.trim()) {
    reason = I18N.settings.appleSpeechNeedsSetup;
  }
  return {
    disabled: reason !== undefined,
    description: [reason, profileLanguagePresetLabel(profile)].filter(Boolean).join(" · ") || undefined,
  };
}
