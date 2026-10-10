// @vitest-environment jsdom
import { appOpenAudioPrivacySettings } from "../../lib/ipc";
import { systemAudioPermissionCopy } from "../../lib/systemAudioPermissions";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SettingsView } from "./SettingsView";

vi.mock("../../lib/ipc", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  appOpenAudioPrivacySettings: vi.fn(),
}));

let host: HTMLDivElement;
let root: Root;
const initial = useStore.getState();
const start = vi.fn().mockResolvedValue(undefined);
const saveSettings = vi.fn().mockResolvedValue(undefined);
const saveProfileCredentials = vi.fn().mockResolvedValue(undefined);

beforeEach(() => {
  vi.mocked(appOpenAudioPrivacySettings).mockReset().mockResolvedValue(undefined);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })));
  Element.prototype.scrollTo = vi.fn();
  window.history.replaceState(null, "", "#subtitle-settings");
  setStoredUiLanguage("en");
  useStore.setState({ ...initial, start, saveSettings, saveProfileCredentials }, true);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  useStore.setState(initial, true);
  setStoredUiLanguage("system");
  window.history.replaceState(null, "", window.location.pathname);
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

async function mount() { await act(async () => root.render(<SettingsView />)); }
async function clickGuide() {
  await act(async () => host.querySelector<HTMLButtonElement>(".settings-guide-entry")!.click());
}

it.each(["zh", "en", "ja"] as const)("offers sentence dividers off by default and saves an explicit choice in %s", async (language) => {
  setStoredUiLanguage(language);
  await mount();
  const divider = host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.subtitleDividers}"]`)!;
  expect(divider.getAttribute("aria-checked")).toBe("false");
  expect(host.textContent).toContain(I18N.settings.subtitleDividersHelp);
  expect(saveSettings).not.toHaveBeenCalled();
  await act(async () => divider.click());
  expect(saveSettings).toHaveBeenCalledExactlyOnceWith({ showSubtitleDividers: true });
  await act(async () => useStore.setState({ settings: { ...initial.settings, showSubtitleDividers: true } }));
  expect(divider.getAttribute("aria-checked")).toBe("true");
  await act(async () => divider.click());
  expect(saveSettings).toHaveBeenLastCalledWith({ showSubtitleDividers: false });
  expect(start).not.toHaveBeenCalled();
  expect(saveProfileCredentials).not.toHaveBeenCalled();
});

it("keeps the guide reachable across categories without starting capture or saving credentials", async () => {
  await mount();
  for (const category of ["subtitles", "service", "general", "export", "diagnostics"]) {
    await act(async () => host.querySelector<HTMLButtonElement>(`#settings-category-${category}`)!.click());
    expect(host.querySelector(".settings-guide-entry")?.textContent).toContain(I18N.settings.quickStartNav);
    await clickGuide();
    expect(host.querySelector(".settings-page-header h1")?.textContent).toBe(I18N.settings.quickStartTitle);
    expect(host.querySelectorAll(".quick-start-guide > li")).toHaveLength(3);
    expect(window.location.hash).toBe("#getting-started");
    expect(host.querySelector(".settings-session-card")).toBeNull();
  }
  expect(start).not.toHaveBeenCalled();
  expect(saveSettings).not.toHaveBeenCalled();
  expect(saveProfileCredentials).not.toHaveBeenCalled();
});

it("opens the existing service and subtitle pages only after clicking their guide actions", async () => {
  await mount();
  await clickGuide();
  await act(async () => host.querySelector<HTMLButtonElement>(".quick-start-guide li:first-child button")!.click());
  expect(window.location.hash).toBe("#service-profiles");
  expect(host.querySelector("#service-profiles-panel")).not.toBeNull();
  await clickGuide();
  await act(async () => host.querySelector<HTMLButtonElement>(".quick-start-guide li:last-child button")!.click());
  expect(window.location.hash).toBe("#subtitle-settings");
  expect(host.querySelector("#subtitle-settings-panel")).not.toBeNull();
  expect(start).not.toHaveBeenCalled();
  expect(saveSettings).not.toHaveBeenCalled();
  expect(saveProfileCredentials).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("restores the guide deep link and labels in %s", async (language) => {
  setStoredUiLanguage(language);
  window.history.replaceState(null, "", "#getting-started");
  await mount();
  expect(host.querySelector(".settings-page-header h1")?.textContent).toBe(I18N.settings.quickStartTitle);
  expect(host.querySelector(".settings-guide-entry")?.getAttribute("aria-current")).toBe("page");
  expect([...host.querySelectorAll(".quick-start-guide h2")].map((heading) => heading.textContent)).toEqual([
    I18N.settings.quickStartServiceTitle, I18N.settings.quickStartAudioTitle, I18N.settings.quickStartDisplayTitle,
  ]);
  expect(start).not.toHaveBeenCalled();
  expect(saveSettings).not.toHaveBeenCalled();
});

it("keeps the service settings header passive when the session has failed", async () => {
  window.history.replaceState(null, "", "#service-profiles");
  useStore.setState({
    settings: { ...initial.settings, profiles: initial.settings.profiles.map(profile => ({ ...profile, credentialState: "present" })) },
    session: { ...initial.session, status: { kind: "error", message: "Synthetic error" }, isActive: false },
  });
  await mount();
  expect(host.querySelector(".settings-session-card, #settings-session-status, .settings-session-control__actions")).toBeNull();
  expect(host.querySelector(".service-row__edit")).not.toBeNull();
  expect(start).not.toHaveBeenCalled();
  await act(() => useStore.setState({ session: { ...initial.session, status: { kind: "listening" }, isActive: true } }));
  expect(host.querySelector(".settings-session-card, #settings-session-status")).toBeNull();
  const details = host.querySelector<HTMLButtonElement>(".service-row__edit")!;
  expect(details.disabled).toBe(false);
  await act(async () => details.click());
  expect(host.querySelector<HTMLInputElement>('[id^="profile-name-"]')?.disabled).toBe(true);
  expect(start).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("keeps session recovery out of the settings header in %s", async (language) => {
  setStoredUiLanguage(language);
  window.history.replaceState(null, "", "#service-profiles");
  useStore.setState({
    settings: { ...initial.settings, profiles: initial.settings.profiles.map((profile) => ({ ...profile, credentialState: "present" })) },
    session: { ...initial.session, status: { kind: "listening" }, isActive: true, translationRecovery: { reason: "rateLimited", retryAfterMs: 900 } },
  });
  await mount();
  expect(host.querySelector("#settings-session-status, .settings-session-card")).toBeNull();
  await act(async () => useStore.setState({ session: { ...useStore.getState().session, translationRecovery: { reason: "temporarilyUnavailable", retryAfterMs: 600 } } }));
  expect(host.querySelector("#settings-session-status, .settings-session-card")).toBeNull();
  await act(async () => useStore.setState({ session: { ...useStore.getState().session, isPaused: true } }));
  expect(host.querySelector("#settings-session-status, .settings-session-card")).toBeNull();
  await act(async () => useStore.setState({ session: { ...useStore.getState().session, isPaused: false, translationRecovery: null } }));
  expect(host.querySelector("#settings-session-status, .settings-session-card")).toBeNull();
  expect(start).not.toHaveBeenCalled();
});

it.each(["zh", "zh-TW", "en", "ja", "de", "ko", "fr", "th"] as const)("explains macOS audio-only and existing grants without starting a session in %s", async language => {
  setStoredUiLanguage(language);
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Macintosh");
  window.history.replaceState(null, "", "#getting-started");
  await mount();
  expect(host.textContent).toContain(systemAudioPermissionCopy().guide);
  expect(host.textContent).toContain("14.2");
  const button = host.querySelector<HTMLButtonElement>(".quick-start-guide li:nth-child(2) button")!;
  expect(button.textContent).toBe(systemAudioPermissionCopy().open);
  expect(appOpenAudioPrivacySettings).not.toHaveBeenCalled();
  await act(async () => button.click());
  expect(appOpenAudioPrivacySettings).toHaveBeenCalledExactlyOnceWith();
  expect(saveProfileCredentials).not.toHaveBeenCalled();
  expect(start).not.toHaveBeenCalled();
  expect(saveSettings).not.toHaveBeenCalled();
  vi.restoreAllMocks();
});

it.each(["Windows NT 10.0", "Linux"])("omits the macOS permission guide on %s", async userAgent => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(userAgent);
  window.history.replaceState(null, "", "#getting-started");
  await mount();
  expect(host.textContent).not.toContain(systemAudioPermissionCopy().guide);
  expect(host.querySelector(".quick-start-guide li:nth-child(2) button")).toBeNull();
  expect(start).not.toHaveBeenCalled();
  vi.restoreAllMocks();
});

it("keeps a manual privacy path if opening settings from the guide fails", async () => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Macintosh");
  vi.mocked(appOpenAudioPrivacySettings).mockRejectedValueOnce(new Error("private-opener-detail"));
  window.history.replaceState(null, "", "#getting-started");
  await mount();
  const button = host.querySelector<HTMLButtonElement>(".quick-start-guide li:nth-child(2) button")!;
  await act(async () => button.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(systemAudioPermissionCopy().failed);
  expect(host.textContent).not.toContain("private-opener-detail");
  expect(button.disabled).toBe(false);
  expect(start).not.toHaveBeenCalled();
  expect(saveSettings).not.toHaveBeenCalled();
  expect(saveProfileCredentials).not.toHaveBeenCalled();
  vi.restoreAllMocks();
});
