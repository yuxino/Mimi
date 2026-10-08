import { afterEach, expect, it } from "vitest";
import { I18N, setStoredUiLanguage } from "./i18n";
import { sessionErrorSettingsTarget } from "./connectionDiagnostics";
import { windowsLiveCaptionsErrorMessage, windowsLiveCaptionsStatusText } from "./windowsLiveCaptions";
import { WINDOWS_LIVE_CAPTIONS_COPY } from "./windowsLiveCaptionsCopy";

afterEach(() => setStoredUiLanguage("system"));
it.each(["en", "zh", "zh-TW", "ja", "de", "fr", "ko"] as const)("keeps setup, privacy and safe recovery copy complete in %s", locale => {
  setStoredUiLanguage(locale);
  expect(Object.keys(WINDOWS_LIVE_CAPTIONS_COPY[locale]).sort()).toEqual(Object.keys(WINDOWS_LIVE_CAPTIONS_COPY.en).sort());
  expect(Object.values(WINDOWS_LIVE_CAPTIONS_COPY[locale]).every(text => text.trim())).toBe(true);
  expect(windowsLiveCaptionsStatusText({ available: false, status: "unsupported" })).toBe(I18N.settings.windowsLiveCaptionsUnsupported);
  expect(windowsLiveCaptionsErrorMessage("windows_live_captions_consent_required")).toBe(I18N.settings.windowsLiveCaptionsConsentRequired);
  expect(windowsLiveCaptionsErrorMessage("windows_live_captions_recording_unsupported")).toBe(I18N.settings.windowsLiveCaptionsAudioUnavailable);
});
it("routes fixed Windows failures to the active configuration without exposing arbitrary native content", () => {
  expect(sessionErrorSettingsTarget("windows_live_captions_setup_required")).toBe("activeProfile");
  expect(sessionErrorSettingsTarget("windows_live_captions_read_failed")).toBe("activeProfile");
  expect(windowsLiveCaptionsErrorMessage("windows_live_captions_read_failed: synthetic-private-caption")).toBeNull();
  expect(windowsLiveCaptionsErrorMessage("synthetic-private-caption")).toBeNull();
});
