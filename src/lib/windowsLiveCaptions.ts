import { I18N } from "./i18n";
import type { SettingsSnapshot, WindowsLiveCaptionsSupport } from "./types";

export function usesWindowsLiveCaptions(settings: Pick<SettingsSnapshot, "profiles" | "activeProfileId">): boolean {
  return settings.profiles?.find(profile => profile.id === settings.activeProfileId)?.provider === "windowsLiveCaptions";
}

export function windowsLiveCaptionsStatusText(support: WindowsLiveCaptionsSupport | null): string {
  if (!support?.available || support.status === "unsupported") return I18N.settings.windowsLiveCaptionsUnsupported;
  return {
    closed: I18N.settings.windowsLiveCaptionsClosed,
    setupRequired: I18N.settings.windowsLiveCaptionsSetupRequired,
    ready: I18N.settings.windowsLiveCaptionsReady,
    unreadable: I18N.settings.windowsLiveCaptionsUnreadable,
  }[support.status];
}

/** Only fixed backend labels enter product copy; never expose native UI text. */
export function windowsLiveCaptionsErrorMessage(error: string): string | null {
  switch (error) {
    case "windows_live_captions_consent_required": return I18N.settings.windowsLiveCaptionsConsentRequired;
    case "windows_live_captions_unsupported":
    case "windows_live_captions_ui_test_unavailable": return I18N.settings.windowsLiveCaptionsUnsupported;
    case "windows_live_captions_closed": return I18N.settings.windowsLiveCaptionsClosed;
    case "windows_live_captions_setup_required": return I18N.settings.windowsLiveCaptionsSetupRequired;
    case "windows_live_captions_audio_input_unsupported":
    case "windows_live_captions_recording_unsupported":
    case "windows_live_captions_application_target_unsupported": return I18N.settings.windowsLiveCaptionsAudioUnavailable;
    case "windows_live_captions_unreadable":
    case "windows_live_captions_setup_timeout":
    case "windows_live_captions_not_connected":
    case "windows_live_captions_result_backlog":
    case "windows_live_captions_read_failed":
    case "windows_live_captions_snapshot_limit": return I18N.settings.windowsLiveCaptionsUnreadable;
    default: return null;
  }
}
