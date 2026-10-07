import {
  effectiveTranslationModeForSettings,
  sourceLanguagesForSettings,
  targetLanguagesForSettings,
  translationModesForSettings,
} from "../../lib/providerCapabilities";
import {
  targetLanguageTranslatesAudio,
  type SettingsSnapshot,
  type SourceLanguage,
  type TargetLanguage,
  type TranslationMode,
} from "../../lib/types";

export interface OverlayControlPanelModel {
  sourceOptions: readonly SourceLanguage[];
  targetOptions: readonly TargetLanguage[];
  translationModeOptions: readonly TranslationMode[];
  effectiveTranslationMode: TranslationMode;
  immersiveModeEnabled: boolean;
  overlayLocked: boolean;
  canSkipTranslation: boolean;
}

/** Resolve both controls from the same route-aware catalog as settings. */
export function overlayControlPanelModel(
  settings: SettingsSnapshot,
): OverlayControlPanelModel {
  const sourceLanguages = sourceLanguagesForSettings(settings);
  const translationModes = translationModesForSettings(settings);
  return {
    sourceOptions: sourceLanguages,
    targetOptions: targetLanguagesForSettings(settings),
    translationModeOptions:
      targetLanguageTranslatesAudio(settings.targetLanguage) &&
      translationModes.length > 1
        ? translationModes
        : [],
    effectiveTranslationMode: effectiveTranslationModeForSettings(settings),
    immersiveModeEnabled: settings.subtitleBlendsWithBackground,
    overlayLocked: settings.isOverlayLocked,
    canSkipTranslation: targetLanguagesForSettings(settings).includes("original"),
  };
}
