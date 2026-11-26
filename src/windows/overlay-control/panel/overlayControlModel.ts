import {
  sourceLanguagesForSettings,
  targetLanguagesForSettings,
} from "../../../lib/providers/providerCapabilities";
import type {
  SettingsSnapshot,
  SourceLanguage,
  TargetLanguage,
} from "../../../lib/types";

export interface OverlayControlPanelModel {
  sourceOptions: readonly SourceLanguage[];
  targetOptions: readonly TargetLanguage[];
  immersiveModeEnabled: boolean;
  overlayLocked: boolean;
}

/** Resolve both controls from the same route-aware catalog as settings. */
export function overlayControlPanelModel(
  settings: SettingsSnapshot,
): OverlayControlPanelModel {
  return {
    sourceOptions: sourceLanguagesForSettings(settings),
    targetOptions: targetLanguagesForSettings(settings),
    immersiveModeEnabled: settings.subtitleBlendsWithBackground,
    overlayLocked: settings.isOverlayLocked,
  };
}
