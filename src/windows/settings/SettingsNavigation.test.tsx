// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import type { SettingsDraft } from "../../lib/types";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES } from "../../lib/types";
import { sourceLanguagesForSettings, targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { profileRevealCredential } from "../../lib/ipc";
import { SettingsView } from "./SettingsView";

vi.mock("../../lib/ipc", async (original) => ({ ...await original<typeof import("../../lib/ipc")>(), profileRevealCredential: vi.fn() }));

const initial = useStore.getState();
let host: HTMLDivElement;
let root: Root;
let saveProfileCredentials: ReturnType<typeof vi.fn>;
let saveSettings: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  Element.prototype.scrollTo = vi.fn();
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  window.history.replaceState(null, "", "#subtitle-settings");
  saveProfileCredentials = vi.fn().mockResolvedValue(undefined);
  saveSettings = vi.fn().mockResolvedValue(undefined);
  vi.mocked(profileRevealCredential).mockReset();
  useStore.setState({ ...initial, saveProfileCredentials, saveSettings,
    settings: { ...initial.settings, profiles: initial.settings.profiles.map((profile) => ({ ...profile, credentialState: "present" })) },
    session: { ...initial.session, status: { kind: "idle" }, isActive: false, isPaused: false },
  }, true);
  host = document.createElement("div"); document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initial, true);
  setStoredUiLanguage("system");
  window.history.replaceState(null, "", window.location.pathname);
  vi.unstubAllGlobals();
});

async function mount() { await act(async () => root.render(<SettingsView />)); }
async function select(category: string) {
  const button = host.querySelector<HTMLButtonElement>(`#settings-category-${category}`)!;
  await act(async () => { button.focus(); button.click(); });
  expect(document.activeElement).toBe(button);
  expect(button.getAttribute("aria-current")).toBe("page");
  const panel = host.querySelector(`#${button.getAttribute("aria-controls")}`)!;
  expect(panel.classList.contains("is-inactive")).toBe(false);
}

it.each(["zh", "en", "ja"] as const)("offers five categories in the expected order and keeps subtitle operations in the relevant pages in %s", async (language) => {
  setStoredUiLanguage(language);
  await mount();
  expect([...host.querySelectorAll(".settings-category-nav button")].map((button) => button.textContent)).toEqual([
    I18N.settings.subtitleTitle, I18N.settings.serviceProfilesTitle, I18N.settings.sessionExportTitle,
    I18N.settings.applicationTitle, I18N.settings.diagnosticsTitle,
  ]);
  expect(host.querySelector("#subtitle-settings .source-language-grid")).toBeNull();
  expect(host.querySelector("#subtitle-settings [aria-label=\"" + I18N.settings.translateTo + "\"]")).toBeNull();
  expect(host.querySelector("#translation-languages")).toBeNull();
  expect(host.querySelector("#service-profiles-panel #network-proxy")).toBeNull();
  expect(host.querySelector(".settings-sidebar .settings-support-diagnostics")).toBeNull();
  for (const [category, description] of [
    ["subtitles", I18N.settings.subtitlePageDescription], ["service", I18N.settings.servicePageDescription],
    ["export", I18N.settings.exportPageDescription], ["general", I18N.settings.generalPageDescription],
    ["diagnostics", I18N.settings.diagnosticsPageDescription],
  ] as const) {
    await select(category);
    expect(host.querySelector(".settings-session-card") !== null).toBe(category === "subtitles");
    expect(host.querySelector(".settings-page-header [role=switch], .settings-page-header kbd")).toBeNull();
    expect(host.querySelector(".settings-page-header .settings-help-control__description")?.textContent).toBe(description);
    expect(host.querySelector(".settings-page-header p")).toBeNull();
  }
  expect(window.location.hash).toBe("#diagnostics");
  await act(async () => host.querySelector<HTMLButtonElement>(".settings-guide-entry")!.click());
  expect(host.querySelector(".quick-start-guide")).not.toBeNull();
  expect(host.querySelector(".settings-session-card")).toBeNull();
  expect(host.querySelector(".settings-quit-button")).not.toBeNull();
  expect(saveSettings).not.toHaveBeenCalled();
  expect(saveProfileCredentials).not.toHaveBeenCalled();
});

it.each(["openAIRealtime", "volcanoEngine", "tencentCloud", "baiduTranslate"] as const)("groups selectable %s languages inside service details", async (provider) => {
  const settings = { ...useStore.getState().settings, sourceLanguage: "auto" as const, profiles: useStore.getState().settings.profiles.map(profile => ({ ...profile, provider })) };
  useStore.setState({ settings });
  await mount(); await select("service");
  expect(host.querySelector("#translation-languages")).toBeNull();
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const groups = [...host.querySelectorAll(".service-detail #translation-languages [role=group]")];
  expect(groups.map(group => [...group.querySelectorAll("button span")].map(node => node.textContent))).toEqual([
    sourceLanguagesForSettings(settings).map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]),
    targetLanguagesForSettings(settings).map(language => TARGET_LANGUAGE_DISPLAY_NAMES[language]),
  ]);
  expect(saveSettings).not.toHaveBeenCalled();
});

it("saves language choices and blocks them for active and paused subtitle sessions", async () => {
  useStore.setState({ settings: { ...useStore.getState().settings, profiles: useStore.getState().settings.profiles.map(profile => ({ ...profile, provider: "openAIRealtime" })) } });
  await mount(); await select("service");
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const target = [...host.querySelectorAll<HTMLButtonElement>("#translation-languages [role=group]")[1].querySelectorAll("button")].find(button => button.textContent === TARGET_LANGUAGE_DISPLAY_NAMES.ja)!;
  await act(async () => { target.focus(); target.click(); });
  expect(saveSettings).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "ja" });
  expect(document.activeElement).toBe(target);
  for (const state of [{ isActive: true, isPaused: false }, { isActive: false, isPaused: true }]) {
    await act(() => useStore.setState({ session: { ...initial.session, status: { kind: "listening" }, ...state } }));
    expect([...host.querySelectorAll<HTMLButtonElement>("#translation-languages [role=group] button")].every(button => button.disabled)).toBe(true);
    expect(host.querySelector("#translation-languages")?.textContent).toContain(I18N.settings.languageChangeRequiresStop);
  }
});

it("places independent proxy controls inside a service profile and blocks changes while subtitles are paused", async () => {
  await mount(); await select("service");
  expect(host.querySelector("#application-settings-panel #network-proxy")).toBeNull();
  expect(host.querySelector(".service-proxies")).toBeNull();
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector("#service-profiles-panel .service-proxies")?.textContent).toContain(I18N.settings.networkProxySpeechScope);
  expect(host.querySelectorAll('.service-proxies [role="combobox"]')).toHaveLength(2);
  const selector = host.querySelector<HTMLButtonElement>('.service-proxies [role="combobox"]')!;
  expect(selector.disabled).toBe(false);
  await act(() => selector.click());
  await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === I18N.settings.networkProxyDirect)!.click());
  expect(saveSettings).not.toHaveBeenCalled();
  expect(useStore.getState().settings.profiles[0]!.speechNetworkProxy).toEqual({ mode: "direct", url: null });
  expect(useStore.getState().settings.profiles[0]!.textNetworkProxy).toBeUndefined();
  await act(() => useStore.setState({ session: { ...initial.session, status: { kind: "listening" }, isActive: false, isPaused: true } }));
  expect(selector.disabled).toBe(true);
  expect(host.querySelector(".service-proxies")?.textContent).toContain(I18N.settings.networkProxyLocked);
});

it("clears a saved-value reveal when leaving services while preserving the unsaved replacement draft", async () => {
  await mount(); await select("service");
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const replace = [...host.querySelectorAll<HTMLButtonElement>(".credential-panel button")].find((button) => button.textContent === I18N.settings.replaceCredentials)!;
  await act(() => replace.click());
  const draft = host.querySelector<HTMLInputElement>('.credential-panel input[type="password"]')!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(draft, "synthetic-unsaved-replacement");
    draft.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(profileRevealCredential).not.toHaveBeenCalled();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-saved-preview");
  await act(async () => { host.querySelector<HTMLButtonElement>(".stored-credential-reveal button")!.click(); });
  expect(host.querySelector<HTMLInputElement>(".stored-credential-reveal input")!.value).toBe("synthetic-saved-preview");
  await select("general");
  expect(host.querySelector(".stored-credential-reveal")).toBeNull();
  await select("service");
  expect(host.querySelector(".stored-credential-reveal input")).toBeNull();
  expect(host.querySelector('.credential-panel input[type="password"]')).toBe(draft);
  expect(draft.value).toBe("synthetic-unsaved-replacement");
  expect(JSON.stringify(useStore.getState().settings)).not.toContain("synthetic-saved-preview");
  expect(saveProfileCredentials).not.toHaveBeenCalled();
});

it("restores diagnostics from a deep link and follows hash navigation without losing nav focus", async () => {
  window.history.replaceState(null, "", "#diagnostics");
  await mount();
  expect(host.querySelector(".settings-page-header h1")?.textContent).toBe(I18N.settings.diagnosticsTitle);
  expect(host.querySelector("#settings-category-diagnostics")?.getAttribute("aria-current")).toBe("page");
  expect(host.querySelector(".settings-session-card")).toBeNull();
  await act(async () => {
    window.history.replaceState(null, "", "#service-profiles");
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
  expect(host.querySelector("#settings-category-service")?.getAttribute("aria-current")).toBe("page");
  expect(host.querySelector(".settings-session-card--compact")).toBeNull();
});

it("blocks placeholder session and credential controls while loading settings and offers an explicit retry after timeout", async () => {
  const initialize = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ init: initialize, initializationStatus: "loading", hasSettingsSnapshot: false });
  await mount();
  expect(host.textContent).toContain(I18N.settings.settingsSnapshotLoading);
  expect(host.querySelector(".settings-session-card")).toBeNull();
  expect(host.querySelector(".service-rows")).toBeNull();
  expect(host.textContent).not.toContain(I18N.settings.credentialUnavailable);
  await act(() => useStore.setState({ initializationStatus: "error", initializationError: "timeout" }));
  expect(host.textContent).toContain(I18N.settings.settingsSnapshotTimeout);
  const retry = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === I18N.settings.retryLoadingSettings)!;
  await act(async () => { retry.focus(); retry.click(); });
  expect(initialize).toHaveBeenCalledOnce();
  expect(document.activeElement).toBe(retry);
  await act(async () => { useStore.setState({ initializationStatus: "ready", initializationError: null, hasSettingsSnapshot: true }); });
  expect(host.querySelector(".settings-session-card")).not.toBeNull();
  expect(host.textContent).not.toContain(I18N.settings.settingsSnapshotTimeout);
  expect(saveProfileCredentials).not.toHaveBeenCalled();
});

it("preserves a write-only unsaved credential draft while moving between categories", async () => {
  await mount();
  await select("service");
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const replace = [...host.querySelectorAll<HTMLButtonElement>(".credential-panel button")].find((button) => button.textContent === I18N.settings.replaceCredentials)!;
  await act(async () => replace.click());
  const input = host.querySelector<HTMLInputElement>('.credential-panel input[type="password"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "synthetic-unsaved-draft");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  for (const category of ["diagnostics", "general", "export", "subtitles", "service"]) await select(category);
  expect(host.querySelector('.credential-panel input[type="password"]')).toBe(input);
  expect(input.value).toBe("synthetic-unsaved-draft");
  expect(saveProfileCredentials).not.toHaveBeenCalled();
  expect(saveSettings).not.toHaveBeenCalled();
});

it("keeps an explicit subtitle save alive while the user navigates to another category", async () => {
  let complete!: () => void;
  saveSettings.mockImplementationOnce(async (draft: SettingsDraft) => {
    await new Promise<void>((resolve) => { complete = resolve; });
    useStore.setState({ settings: { ...useStore.getState().settings, ...draft } });
  });
  await mount();
  const size = host.querySelector<HTMLInputElement>('input[type="range"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(size, "20");
    size.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(saveSettings).toHaveBeenCalledExactlyOnceWith({ fontSize: 20 });
  await select("diagnostics");
  await act(async () => complete());
  await select("subtitles");
  expect(host.querySelector<HTMLInputElement>('input[type="range"]')?.value).toBe("20");
});

it("saves background transparency, previews it and preserves it in immersive mode", async () => {
  saveSettings.mockImplementation(async (draft: SettingsDraft) => {
    useStore.setState({ settings: { ...useStore.getState().settings, ...draft } });
  });
  await mount();
  const slider = host.querySelector<HTMLInputElement>(`input[aria-label="${I18N.settings.backgroundTransparency}"]`)!;
  expect(slider.value).toBe("20");
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(slider, "65");
    slider.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(saveSettings).toHaveBeenCalledExactlyOnceWith({ subtitleBackgroundOpacity: 35 });
  expect(host.querySelector<HTMLElement>(".subtitle-preview__text")!.style.background).toBe("rgba(0, 0, 0, 0.35)");
  await act(() => useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: true } })));
  expect(slider.disabled).toBe(true);
  expect(host.querySelector<HTMLElement>(".subtitle-preview__text")!.style.background).toBe("transparent");
  await act(() => useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: false } })));
  expect(slider.disabled).toBe(false);
  expect(slider.value).toBe("65");
});

it("restores subtitle opening and immersion controls and configures microphone color independently", async () => {
  const start = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ start, session: { ...initial.session, status: { kind: "idle" }, isActive: false } });
  await mount();
  const switches = () => host.querySelectorAll<HTMLButtonElement>('.settings-session-card [role="switch"]');
  expect(switches()).toHaveLength(2);
  expect(switches()[1].disabled).toBe(true);
  await act(async () => switches()[0].click());
  expect(start).toHaveBeenCalledOnce();
  await act(async () => useStore.setState({ session: { ...initial.session, status: { kind: "listening" }, isActive: true } }));
  await act(async () => switches()[1].click());
  expect(saveSettings).toHaveBeenCalledWith({ subtitleBlendsWithBackground: !initial.settings.subtitleBlendsWithBackground });
  await act(async () => useStore.setState({ settings: { ...useStore.getState().settings, microphoneInputAvailable: true } }));
  const microphone = host.querySelector(`[role="group"][aria-label="${I18N.settings.microphoneSubtitleColor}"]`)!;
  await act(async () => microphone.querySelectorAll<HTMLButtonElement>("button")[2]!.click());
  expect(saveSettings).toHaveBeenLastCalledWith({ microphoneSubtitleColor: "yellow" });
  expect(host.querySelector(`[role="group"][aria-label="${I18N.settings.systemSubtitleColor}"]`)).not.toBeNull();
});

it("keeps an in-flight subtitle start owned when switching settings categories", async () => {
  const start = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ start });
  await mount();
  const toggle = () => host.querySelector<HTMLButtonElement>('.settings-session-card [role="switch"]')!;
  await act(async () => toggle().click());
  expect(toggle().disabled).toBe(true);
  await select("service");
  expect(host.querySelector(".settings-session-card")).toBeNull();
  await select("subtitles");
  expect(toggle().disabled).toBe(true);
  await act(async () => toggle().click());
  expect(start).toHaveBeenCalledOnce();
  await act(async () => useStore.setState({ session: { ...initial.session, status: { kind: "listening" }, isActive: true } }));
  expect(toggle().disabled).toBe(false);
});


it("hides microphone color while retaining the system color control", async () => {
  await mount();
  expect(host.querySelector(`[role="group"][aria-label="${I18N.settings.microphoneSubtitleColor}"]`)).toBeNull();
  expect(host.querySelector(`[role="group"][aria-label="${I18N.settings.systemSubtitleColor}"]`)).not.toBeNull();
});
