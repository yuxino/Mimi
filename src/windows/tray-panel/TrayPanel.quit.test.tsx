// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { TrayPanel } from "./TrayPanel";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot } from "../../lib/types";
import { sourceLanguagesForSettings } from "../../lib/providerCapabilities";

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
async function filter(query: string) {
  const search = document.querySelector<HTMLInputElement>("input.mimi-select__search")!;
  expect(search.getAttribute("aria-label")).toBe(I18N.settings.searchLanguages);
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, query);
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

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

it.each(["deepL", "deepLX", "openAICompatible"] as const)("keeps %s route limits and uses a non-searchable five-language tray picker", async route => {
  const settings = languageSettings();
  settings.profiles = [{ ...settings.profiles[0], textTranslation: route }];
  useStore.setState({ ...initial, settings }, true);
  await act(async () => root.render(<TrayPanel />));
  await act(async () => sourcePicker().click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options).toHaveLength(5);
  expect(options.map(option => option.textContent)).toEqual(sourceLanguagesForSettings(settings)
    .map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]));
  expect(document.querySelector("input.mimi-select__search")).toBeNull();
  expect(options.map(option => option.textContent)).not.toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
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
