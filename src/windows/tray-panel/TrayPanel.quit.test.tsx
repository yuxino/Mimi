// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import languageCatalogs from "../../../shared/provider-language-catalogs.json";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { sessionActionErrorMessage, profileErrorMessage } from "../../lib/connectionDiagnostics";
import { useStore } from "../../lib/store";
import { TrayPanel } from "./TrayPanel";
import { AUDIO3_RECOGNITION_LANGUAGE_CODES, SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot } from "../../lib/types";
import { sourceLanguagesForSettings } from "../../lib/providerCapabilities";
import permissions from "../../../src-tauri/permissions/app.toml?raw";

let host: HTMLDivElement;
let root: Root;
const initial = useStore.getState();
const scrollIntoView = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollIntoView");

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  useStore.setState(initial, true);
  setStoredUiLanguage("system");
  vi.unstubAllGlobals();
  if (scrollIntoView) Object.defineProperty(HTMLElement.prototype, "scrollIntoView", scrollIntoView);
  else Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
});

it.each(["zh", "en", "ja"] as const)("keeps the %s exit action available in idle, working, paused, transition and error states", async (language) => {
  setStoredUiLanguage(language);
  const quit = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, quit }, true);
  await act(async () => root.render(<TrayPanel />));
  for (const kind of ["idle", "listening", "connecting", "stopping", "error"] as const) {
    for (const isPaused of [false, true]) {
      const status = kind === "error" ? { kind, message: "synthetic session error" } : { kind };
      await act(async () => useStore.setState({ session: { ...initial.session, status, isPaused } }));
      const button = host.querySelector<HTMLButtonElement>('[data-action="quit"]')!;
      expect(button.textContent).toBe(I18N.tray.quit);
      expect(button.disabled).toBe(false);
      expect(button.closest("footer")).not.toBeNull();
    }
  }
  expect(quit).not.toHaveBeenCalled();
  expect(host.textContent).not.toMatch(/Turbo|极速|最速/);
});

it("uses the normal quit action once, shows pending feedback and supports retry after failure", async () => {
  let reject!: (error: Error) => void;
  const quit = vi.fn(() => new Promise<void>((_resolve, failure) => { reject = failure; }));
  useStore.setState({ ...initial, quit }, true);
  await act(async () => root.render(<TrayPanel />));
  const button = host.querySelector<HTMLButtonElement>('[data-action="quit"]')!;
  await act(async () => { button.click(); button.click(); });
  expect(quit).toHaveBeenCalledOnce();
  expect(button.getAttribute("aria-busy")).toBe("true");
  expect(button.textContent).toBe(I18N.tray.quitting);
  expect(button.disabled).toBe(true);
  await act(async () => reject(new Error("private raw failure")));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.tray.quitFailed);
  expect(host.textContent).not.toContain("private raw failure");
  expect(button.disabled).toBe(false);
  await act(async () => button.focus());
  expect(document.activeElement).toBe(button);
  await act(async () => button.click());
  expect(quit).toHaveBeenCalledTimes(2);
  await act(async () => reject(new Error("synthetic retry failure")));
});

it.each(["zh", "en", "ja"] as const)("keeps the selected extended source visible in the same full %s list as settings", async (locale) => {
  setStoredUiLanguage(locale);
  useStore.setState({ ...initial, settings: languageSettings({ sourceLanguage: "fr" }) }, true);
  await act(async () => root.render(<TrayPanel />));
  const language = host.querySelector<HTMLButtonElement>('.tray-setting-row--language [role="combobox"]')!;
  expect(language.textContent).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  expect(language.textContent).not.toContain("fr");
  await act(async () => language.click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options).toHaveLength(25);
  expect(options.map(option => option.textContent)).toEqual(sourceLanguagesForSettings(useStore.getState().settings)
    .map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]));
  expect(document.querySelector("input.mimi-select__search")).not.toBeNull();
  expect(options.map((option) => option.textContent)).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  expect(options.find((option) => option.getAttribute("aria-selected") === "true")?.textContent)
    .toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
});

function languageSettings(draft: Partial<SettingsSnapshot> = {}): SettingsSnapshot {
  return { ...initial.settings, sourceLanguage: "auto", targetLanguage: "zh", languageCapabilities: undefined,
    profiles: [{ id: "ali", name: "Alibaba Cloud", provider: "alibabaCloud", credentialState: "present" }],
    activeProfileId: "ali", ...draft };
}
function sourcePicker() {
  return host.querySelector<HTMLButtonElement>('.tray-setting-row--language [role="combobox"]')!;
}

it.each(["zh", "en", "ja"] as const)("keeps Apple source choices and explicit-language help consistent in the %s tray", async locale => {
  setStoredUiLanguage(locale);
  const switchSourceLanguage = vi.fn().mockResolvedValue(undefined);
  const settings = languageSettings({ sourceLanguage: "en", targetLanguage: "original", activeProfileId: "apple",
    profiles: [{ id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "missing" }],
    languageCapabilities: { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original",
      sourceLanguages: ["en", "fr"], targetLanguages: ["original"] } });
  useStore.setState({ ...initial, settings, switchSourceLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  const help = host.querySelector('.tray-setting-row--language .settings-help-control__description')!;
  expect(help.textContent).toBe(I18N.settings.appleSpeechLanguageHelp);
  expect(help.textContent).not.toBe(I18N.settings.recognitionHintHelp);
  await act(async () => sourcePicker().click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options.map(option => option.textContent)).toEqual([SOURCE_LANGUAGE_DISPLAY_NAMES.en, SOURCE_LANGUAGE_DISPLAY_NAMES.fr]);
  await act(async () => options[1].click());
  expect(switchSourceLanguage).toHaveBeenCalledExactlyOnceWith("fr");
});
async function filter(query: string) {
  const search = document.querySelector<HTMLInputElement>("input.mimi-select__search")!;
  expect(search.getAttribute("aria-label")).toBe(I18N.settings.searchLanguages);
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, query);
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it.each(["zh", "en", "ja"] as const)("keeps an unready Apple selection readable and offers the sole ready recovery choice in the %s tray", async locale => {
  setStoredUiLanguage(locale);
  const switchSourceLanguage = vi.fn().mockResolvedValue(undefined);
  const capabilities = { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original", targetLanguages: ["original"] } as const;
  const settings = languageSettings({ sourceLanguage: "fr", targetLanguage: "original", activeProfileId: "apple",
    profiles: [{ id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "missing" }],
    languageCapabilities: { ...capabilities, sourceLanguages: [] } });
  useStore.setState({ ...initial, settings, switchSourceLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(sourcePicker().textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  expect(sourcePicker().disabled).toBe(true);
  await act(async () => sourcePicker().click());
  expect(document.querySelector('[role="option"]')).toBeNull();
  expect(switchSourceLanguage).not.toHaveBeenCalled();

  const readySettings = { ...settings, languageCapabilities: { ...capabilities, sourceLanguages: ["en"] as const } };
  await act(async () => useStore.setState({ settings: readySettings }));
  expect(sourcePicker().textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  expect(sourcePicker().disabled).toBe(false);
  expect(switchSourceLanguage).not.toHaveBeenCalled();
  await act(async () => sourcePicker().click());
  const choices = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(choices.map(choice => choice.textContent)).toEqual([SOURCE_LANGUAGE_DISPLAY_NAMES.en]);
  expect(choices[0].getAttribute("aria-selected")).toBe("false");
  await act(async () => choices[0].click());
  expect(switchSourceLanguage).toHaveBeenCalledExactlyOnceWith("en");

  await act(async () => useStore.setState({ settings: { ...readySettings, sourceLanguage: "en" } }));
  expect(sourcePicker().textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.en);
  expect(sourcePicker().disabled).toBe(true);
});

it.each(["zh", "en", "ja"] as const)("searches French in the %s tray and calls the real source action with its wire key", async locale => {
  setStoredUiLanguage(locale);
  const switchSourceLanguage = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, settings: languageSettings(), switchSourceLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  await act(async () => sourcePicker().click());
  await filter("fr");
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options).toHaveLength(1);
  expect(options[0].textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  await act(async () => options[0].click());
  expect(switchSourceLanguage).toHaveBeenCalledExactlyOnceWith("fr");
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});

it("offers 31 Original-mode sources and selects Norwegian after Chinese", async () => {
  const switchSourceLanguage = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, settings: languageSettings({ sourceLanguage: "zh", targetLanguage: "original" }), switchSourceLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(sourcePicker().textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.zh);
  await act(async () => sourcePicker().click());
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(31);
  await filter("Norwegian");
  const option = document.querySelector<HTMLElement>('[role="option"]')!;
  expect(option.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.no);
  await act(async () => option.click());
  expect(switchSourceLanguage).toHaveBeenCalledExactlyOnceWith("no");
});

it.each(["deepL", "deepLX"] as const)("keeps the complete %s route intersection searchable in the tray", async route => {
  const settings = languageSettings();
  settings.profiles = [{ ...settings.profiles[0], textTranslation: route }];
  useStore.setState({ ...initial, settings }, true);
  await act(async () => root.render(<TrayPanel />));
  await act(async () => sourcePicker().click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options.length).toBeGreaterThan(6);
  expect(options.map(option => option.textContent)).toEqual(sourceLanguagesForSettings(settings)
    .map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]));
  expect(document.querySelector("input.mimi-select__search")).not.toBeNull();
  expect(options.map(option => option.textContent)).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
});

it.each(["openAICompatible", "chatMock"] as const)("searches all 31 sources with the %s text route and sends the selected code from the tray", async route => {
  const settings = languageSettings();
  settings.profiles = [{ ...settings.profiles[0], textTranslation: route }];
  const switchSourceLanguage = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, settings, switchSourceLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  await act(async () => sourcePicker().click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options).toHaveLength(31);
  expect(options.map(option => option.textContent)).toEqual(["auto" as const, ...AUDIO3_RECOGNITION_LANGUAGE_CODES]
    .map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]));
  expect(document.querySelector("input.mimi-select__search")).not.toBeNull();
  await filter("Norwegian");
  const filtered = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(filtered.map(option => option.textContent)).toEqual([SOURCE_LANGUAGE_DISPLAY_NAMES.no]);
  await act(async () => filtered[0].click());
  expect(switchSourceLanguage).toHaveBeenCalledExactlyOnceWith("no");
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});

it("uses native options without search when small and falls back after a stale target stamp", async () => {
  const native = { profileId: "ali", provider: "alibabaCloud", textTranslation: "followService", targetLanguage: "zh",
    sourceLanguages: ["auto", "fr"], targetLanguages: ["original", "zh", "fr"] } as const;
  useStore.setState({ ...initial, settings: languageSettings({ languageCapabilities: native }) }, true);
  await act(async () => root.render(<TrayPanel />));
  await act(async () => sourcePicker().click());
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(2);
  expect(document.querySelector("input.mimi-select__search")).toBeNull();
  await act(async () => sourcePicker().click());
  await act(async () => useStore.setState({ settings: languageSettings({ targetLanguage: "original", languageCapabilities: native }) }));
  await act(async () => sourcePicker().click());
  expect(document.querySelectorAll('[role="option"]')).toHaveLength(31);
  expect(document.querySelector("input.mimi-select__search")).not.toBeNull();
});

it.each(["connecting", "stopping"] as const)("prevents language requests while the tray session is %s", async kind => {
  const switchSourceLanguage = vi.fn();
  useStore.setState({ ...initial, settings: languageSettings(), switchSourceLanguage,
    session: { ...initial.session, status: { kind } } }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(sourcePicker().disabled).toBe(true);
  await act(async () => sourcePicker().click());
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(switchSourceLanguage).not.toHaveBeenCalled();
});


it("keeps the interim switch usable while running, reports failure and saves a retry", async () => {
  const save = vi.fn(initial.saveSettings).mockRejectedValueOnce(new Error("synthetic-private-setting-error"));
  useStore.setState({ ...initial, saveSettings: save, session: { ...initial.session, isActive: true, status: { kind: "listening" } } }, true);
  await act(async () => root.render(<TrayPanel />));
  const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.showIntermediateSubtitles}"]`)!;
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  await act(async () => toggle().click());
  expect(save).toHaveBeenLastCalledWith({ showIntermediateSubtitles: false });
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.settingSaveFailed(I18N.settings.showIntermediateSubtitles));
  expect(host.textContent).not.toContain("synthetic-private-setting-error");
  await act(async () => toggle().click());
  expect(toggle().getAttribute("aria-checked")).toBe("false");
  expect(useStore.getState().session.isActive).toBe(true);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("shows custom service default and unknown support beside the tray picker", async () => {
  const settings = languageSettings({ targetLanguage: "original", sourceLanguage: "auto" });
  settings.profiles = [{ ...settings.profiles[0], provider: "customOpenAIASR" }];
  useStore.setState({ ...initial, settings }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(host.querySelector('.tray-setting-row--language [role="combobox"]')?.textContent).toContain(I18N.settings.recognitionServiceDefault);
  expect(host.querySelector('.recognition-language-notice')?.textContent).toBe(I18N.settings.recognitionCustomNotice);
  expect(host.textContent).toContain(I18N.settings.recognitionOpenAIParameter);
});


it("localizes a rejected tray language switch without exposing the IPC label", async () => {
  const switchSourceLanguage = vi.fn().mockRejectedValue(new Error("source_switch_save_failed"));
  useStore.setState({ ...initial, settings: languageSettings(), switchSourceLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  await act(async () => sourcePicker().click());
  await filter("fr");
  await act(async () => document.querySelector<HTMLElement>('[role="option"]')!.click());
  expect(switchSourceLanguage).toHaveBeenCalledExactlyOnceWith("fr");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageSaveFailed);
  expect(host.textContent).not.toContain("source_switch_save_failed");
  expect(sourcePicker().textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.auto);
});

function profileSettings(): SettingsSnapshot {
  return languageSettings({ profiles: [
    { id: "ali", name: "Alibaba Cloud", provider: "alibabaCloud", credentialState: "present" },
    { id: "custom", name: "My recognition model", provider: "openAIRealtime", credentialState: "present" },
  ] });
}
function profilePicker() {
  return host.querySelector<HTMLButtonElement>('.tray-setting-row--profile [role="combobox"]')!;
}
async function profileOption(name: string) {
  await act(async () => profilePicker().click());
  return [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === name)!;
}

it.each(["idle", "listening", "paused", "error"] as const)("switches saved profiles from the %s tray and follows the saved selection", async kind => {
  const settings = profileSettings();
  const selectProfile = vi.fn(async (activeProfileId: string) => {
    const snapshot = { ...settings, activeProfileId };
    useStore.setState({ settings: snapshot });
    return snapshot;
  });
  useStore.setState({ ...initial, settings, selectProfile, session: { ...initial.session,
    status: kind === "error" ? { kind, message: "synthetic-session-error" } : { kind: kind === "paused" ? "listening" : kind },
    isActive: kind === "listening" || kind === "paused", isPaused: kind === "paused" } }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(profilePicker().disabled).toBe(false);
  const current = await profileOption("Alibaba Cloud");
  await act(async () => current.click());
  expect(selectProfile).not.toHaveBeenCalled();
  const next = await profileOption("My recognition model");
  await act(async () => next.click());
  expect(selectProfile).toHaveBeenCalledExactlyOnceWith("custom");
  expect(profilePicker().textContent).toBe("My recognition model");
  expect(useStore.getState().session.isPaused).toBe(kind === "paused");
});

it.each(["connecting", "stopping"] as const)("blocks tray profile switching while %s", async kind => {
  const selectProfile = vi.fn();
  useStore.setState({ ...initial, settings: profileSettings(), selectProfile,
    session: { ...initial.session, status: { kind } } }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(profilePicker().disabled).toBe(true);
  await act(async () => profilePicker().click());
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(selectProfile).not.toHaveBeenCalled();
});

it("blocks duplicate tray profile switches, sanitizes errors and keeps the previous selection for retry", async () => {
  let reject!: (reason: Error) => void;
  const settings = profileSettings();
  const selectProfile = vi.fn()
    .mockImplementationOnce(() => new Promise<SettingsSnapshot>((_resolve, failure) => { reject = failure; }))
    .mockResolvedValue(settings);
  useStore.setState({ ...initial, settings, selectProfile }, true);
  await act(async () => root.render(<TrayPanel />));
  const next = await profileOption("My recognition model");
  await act(async () => { next.click(); next.click(); });
  expect(selectProfile).toHaveBeenCalledExactlyOnceWith("custom");
  expect(profilePicker().disabled).toBe(true);
  expect(sourcePicker().disabled).toBe(true);
  expect(host.querySelector('.tray-setting-row--profile')?.getAttribute("aria-busy")).toBe("true");
  await act(async () => reject(new Error("private-profile-switch-error")));
  expect(profilePicker().textContent).toBe("Alibaba Cloud");
  expect(profilePicker().disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(profileErrorMessage("private-profile-switch-error"));
  expect(host.textContent).not.toContain("private-profile-switch-error");
  const retry = await profileOption("My recognition model");
  await act(async () => retry.click());
  expect(selectProfile).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("grants saved-profile selection to both panels without exposing credential or profile editing commands", () => {
  for (const identifier of ["app-tray-panel", "app-overlay-control"]) {
    const scope = permissions.split("[[permission]]").find(entry => entry.includes(`identifier = "${identifier}"`))!;
    expect(scope).toContain('"profile_select"');
    expect(scope).toContain('"session_switch_target_language"');
    for (const command of ["profile_create", "profile_update", "profile_delete", "profile_save_credentials", "profile_reveal_credential"]) {
      expect(scope).not.toContain(`"${command}"`);
    }
  }
});


it("keeps a persisted tray profile selected when its reconnect fails", async () => {
  const settings = profileSettings();
  const error = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
  const selectProfile = vi.fn(async (activeProfileId: string) => {
    useStore.setState({ settings: { ...settings, activeProfileId } });
    throw error;
  });
  useStore.setState({ ...initial, settings, selectProfile }, true);
  await act(async () => root.render(<TrayPanel />));
  const next = await profileOption("My recognition model");
  await act(async () => next.click());
  expect(profilePicker().textContent).toBe("My recognition model");
  expect(profilePicker().disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(profileErrorMessage(error));
  expect(host.textContent).not.toContain(error);
  expect(selectProfile).toHaveBeenCalledExactlyOnceWith("custom");
});


it.each([false, true])("keeps tray resume feedback safe and ignores a later successful resume=%s", async superseded => {
  const error = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
  let reject!: (reason: string) => void;
  const togglePaused = vi.fn(() => new Promise<void>((_resolve, failure) => { reject = failure; }));
  useStore.setState({ ...initial, togglePaused, settings: profileSettings(), session: { ...initial.session,
    status: { kind: "listening" }, isActive: true, isPaused: true } }, true);
  await act(async () => root.render(<TrayPanel />));
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="resume"]')!.click());
  if (superseded) await act(async () => useStore.setState({ session: { ...useStore.getState().session, isPaused: false } }));
  await act(async () => reject(error));
  expect(host.querySelector('[role="alert"]')?.textContent ?? null).toBe(superseded ? null : sessionActionErrorMessage(error, "fallback"));
  expect(host.textContent).not.toContain(error);
});


it.each(["zh", "en", "ja"] as const)("uses the expanded xAI sources and current target label in the %s tray", async locale => {
  setStoredUiLanguage(locale);
  const switchSourceLanguage = vi.fn().mockResolvedValue(undefined);
  const settings = languageSettings({ targetLanguage: "pt-PT", profiles: [{ id: "ali", name: "xAI", provider: "xAIRealtime", credentialState: "present" }] });
  useStore.setState({ ...initial, settings, switchSourceLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(host.querySelector('.tray-setting-row--target [role="combobox"]')?.textContent).toContain(TARGET_LANGUAGE_DISPLAY_NAMES["pt-PT"]);
  await act(async () => sourcePicker().click());
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent))
    .toEqual(languageCatalogs.xAIRealtime.sourceLanguages.map(code => SOURCE_LANGUAGE_DISPLAY_NAMES[code as keyof typeof SOURCE_LANGUAGE_DISPLAY_NAMES]));
  await filter("pt-BR");
  const choice = document.querySelector<HTMLElement>('[role="option"]')!;
  expect(choice.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES["pt-BR"]);
  await act(async () => choice.click());
  expect(switchSourceLanguage).toHaveBeenCalledExactlyOnceWith("pt-BR");
});

it("keeps Gemini automatic recognition disabled while displaying an expanded target", async () => {
  const settings = languageSettings({ targetLanguage: "uk", profiles: [{ id: "ali", name: "Google", provider: "googleGeminiLive", credentialState: "present" }] });
  useStore.setState({ ...initial, settings }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(sourcePicker().textContent).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.auto);
  expect(sourcePicker().disabled).toBe(true);
  expect(host.querySelector('.tray-setting-row--target [role="combobox"]')?.textContent).toContain(TARGET_LANGUAGE_DISPLAY_NAMES.uk);
});

it.each(["zh", "en", "ja"] as const)("keeps the complete Gemini target menu beside automatic recognition in the %s tray", async locale => {
  setStoredUiLanguage(locale);
  const switchTargetLanguage = vi.fn().mockResolvedValue(undefined);
  const settings = languageSettings({ sourceLanguage: "auto", targetLanguage: "zh" });
  settings.profiles = [{ ...settings.profiles[0], provider: "googleGeminiLive" }];
  useStore.setState({ ...initial, settings, switchTargetLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  expect(sourcePicker().disabled).toBe(true);
  const target = host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!;
  await act(async () => target.click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options.map(option => option.textContent)).toEqual(languageCatalogs.googleGeminiLive.targetLanguages.map(code => TARGET_LANGUAGE_DISPLAY_NAMES[code as keyof typeof TARGET_LANGUAGE_DISPLAY_NAMES]));
  await act(async () => options.find(option => option.textContent === TARGET_LANGUAGE_DISPLAY_NAMES["pt-BR"])!.click());
  expect(switchTargetLanguage).toHaveBeenCalledExactlyOnceWith("pt-BR");
});

it("disables tray target changes in transitions and preserves the saved target after failure", async () => {
  const settings = languageSettings({ sourceLanguage: "auto", targetLanguage: "zh" });
  settings.profiles = [{ ...settings.profiles[0], provider: "googleGeminiLive" }];
  const switchTargetLanguage = vi.fn().mockRejectedValue("target_switch_unsupported");
  useStore.setState({ ...initial, settings, switchTargetLanguage }, true);
  await act(async () => root.render(<TrayPanel />));
  const target = () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!;
  for (const kind of ["connecting", "stopping"] as const) {
    await act(async () => useStore.setState({ session: { ...initial.session, status: { kind } } }));
    expect(target().disabled).toBe(true);
    await act(async () => target().click());
    expect(document.querySelector('[role="listbox"]')).toBeNull();
  }
  await act(async () => useStore.setState({ session: { ...initial.session, status: { kind: "listening" }, isActive: true } }));
  await act(async () => target().click());
  await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent === TARGET_LANGUAGE_DISPLAY_NAMES.fr)!.click());
  expect(switchTargetLanguage).toHaveBeenCalledExactlyOnceWith("fr");
  expect(target().textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.zh);
  expect(target().disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageSwitchUnsupported);
});
