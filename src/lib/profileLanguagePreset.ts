import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type ServiceProfile, type SettingsSnapshot } from "./types";

export function profileSelectionChangesSettings(profile: ServiceProfile, settings: Pick<SettingsSnapshot, "activeProfileId" | "sourceLanguage" | "targetLanguage">): boolean {
  return profile.id !== settings.activeProfileId || (!!profile.languagePreset
    && (profile.languagePreset.sourceLanguage !== settings.sourceLanguage || profile.languagePreset.targetLanguage !== settings.targetLanguage));
}

export function profileSelectionLabel(profile: ServiceProfile): string {
  const preset = profile.languagePreset;
  return preset ? `${profile.name} · ${SOURCE_LANGUAGE_DISPLAY_NAMES[preset.sourceLanguage]} → ${TARGET_LANGUAGE_DISPLAY_NAMES[preset.targetLanguage]}` : profile.name;
}
