// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot } from "../../lib/types";
import { ProfileLanguageSettings } from "./ProfileLanguageSettings";

const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
let save: ReturnType<typeof vi.fn>;
let settings: SettingsSnapshot;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  settings = { ...initial.settings, sourceLanguage: "auto", targetLanguage: "en", languageCapabilities: undefined,
    profiles: [{ id: "test", name: "Test", provider: "alibabaCloud", credentialState: "present" }], activeProfileId: "test" };
  save = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, settings, saveSettings: save }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});
async function render(disabled = false, requiresStop = false) {
  await act(async () => root.render(<><ProfileLanguageSettings settings={settings} disabled={disabled} requiresStop={requiresStop} /><SettingsToastRegion /></>));
}
async function choose(label: string, code: string) {
  const trigger = host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
  await act(() => trigger.click());
  const search = document.querySelector<HTMLInputElement>('input.mimi-select__search')!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, code);
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const option = document.querySelector<HTMLElement>('[role="option"]')!;
  await act(async () => option.click());
}

it("saves an explicit Chinese source without replacing the translation target", async () => {
  await render();
  await choose(I18N.settings.sourceLanguage, "zh");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "zh" });
  expect(host.textContent).toContain(I18N.settings.languageSaved);
});

it.each(["zh", "en", "ja"] as const)("keeps Apple source choices and explicit-language help consistent in %s settings", async locale => {
  setStoredUiLanguage(locale);
  settings = { ...settings, sourceLanguage: "en", targetLanguage: "original", activeProfileId: "apple",
    profiles: [{ id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "missing" }],
    languageCapabilities: { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original",
      sourceLanguages: ["en", "fr"], targetLanguages: ["original"] } };
  await render();
  const help = host.querySelector('.settings-row .settings-help-control__description')!;
  expect(help.textContent).toBe(I18N.settings.appleSpeechLanguageHelp);
  expect(help.textContent).not.toBe(I18N.settings.recognitionHintHelp);
  const group = host.querySelector(`[role="group"][aria-label="${I18N.settings.sourceLanguage}"]`)!;
  const choices = [...group.querySelectorAll<HTMLButtonElement>('button')];
  expect(choices.map(choice => choice.textContent)).toEqual([SOURCE_LANGUAGE_DISPLAY_NAMES.en, SOURCE_LANGUAGE_DISPLAY_NAMES.fr]);
  await act(async () => choices[1].click());
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "fr" });
});

it.each(["fr", "zh_tw"])("saves a searched %s target with its exact wire key", async code => {
  settings = { ...settings, sourceLanguage: "zh" };
  await render();
  await choose(I18N.settings.translateTo, code);
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: code });
});

it.each(["zh", "en", "ja"] as const)("preserves an unready Apple choice and lets the user select the only ready language in %s", async locale => {
  setStoredUiLanguage(locale);
  const capabilities = { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original", targetLanguages: ["original"] } as const;
  settings = { ...settings, sourceLanguage: "fr", targetLanguage: "original", activeProfileId: "apple",
    profiles: [{ id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "missing" }],
    languageCapabilities: { ...capabilities, sourceLanguages: [] } };
  const picker = () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!;
  await render();
  expect(picker().textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  expect(picker().disabled).toBe(true);
  expect(host.querySelector(".recognition-language-notice")?.textContent).toBe(I18N.settings.appleSpeechNoReadyLanguages);
  await act(async () => picker().click());
  expect(document.querySelector('[role="option"]')).toBeNull();
  expect(save).not.toHaveBeenCalled();

  settings = { ...settings, languageCapabilities: { ...capabilities, sourceLanguages: ["en"] } };
  await render();
  expect(picker().textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  expect(picker().disabled).toBe(false);
  expect(host.querySelector(".recognition-language-notice")).toBeNull();
  expect(save).not.toHaveBeenCalled();
  await act(async () => picker().click());
  const choices = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(choices.map(choice => choice.textContent)).toEqual([SOURCE_LANGUAGE_DISPLAY_NAMES.en]);
  expect(choices[0].getAttribute("aria-selected")).toBe("false");
  await act(async () => choices[0].click());
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "en" });

  settings = { ...settings, sourceLanguage: "en" };
  await render();
  const selected = host.querySelector<HTMLButtonElement>(`[role="group"][aria-label="${I18N.settings.sourceLanguage}"] button`)!;
  expect(selected.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.en);
  expect(selected.getAttribute("aria-pressed")).toBe("true");
  expect(selected.disabled).toBe(true);
});

it("uses the broader recognition list only for an original-only route", async () => {
  settings = { ...settings, targetLanguage: "original" };
  await render();
  await choose(I18N.settings.sourceLanguage, "Norwegian");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "no" });
});

it("guards duplicate saves and reports a safe failure without pretending the choice was saved", async () => {
  let fail!: (reason: Error) => void;
  save.mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  settings = { ...settings, profiles: [{ ...settings.profiles[0], provider: "azureOpenAIRealtime" }] };
  await render();
  const button = [...host.querySelectorAll<HTMLButtonElement>('[role="group"] button')].find(node => node.textContent === "Japanese")!;
  await act(() => { button.click(); button.click(); });
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "ja" });
  expect(host.querySelector("section")?.getAttribute("aria-busy")).toBe("true");
  expect(button.disabled).toBe(true);
  await act(async () => fail(new Error("synthetic-private-provider-body")));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageSaveFailed);
  expect(host.textContent).not.toContain("synthetic-private-provider-body");
  expect(button.getAttribute("aria-pressed")).toBe("false");
  expect(button.disabled).toBe(false);
});

it("explains a session lock separately from a transient connection check", async () => {
  await render(true);
  expect(host.querySelectorAll('button:disabled')).toHaveLength(3);
  expect(host.textContent).not.toContain(I18N.settings.languageChangeRequiresStop);
  await render(true, true);
  expect(host.textContent).toContain(I18N.settings.languageChangeRequiresStop);
  expect(save).not.toHaveBeenCalled();
});

it("persists skipping translation as Original, restores the previous target, and keeps source-language help compact", async () => {
  await render();
  expect(host.querySelector('.settings-row .settings-help-control__description')?.textContent).toContain(I18N.settings.recognitionHintHelp);
  const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.skipTranslation}"]`)!;
  await act(async () => toggle().click());
  expect(save).toHaveBeenCalledWith({ targetLanguage: "original" });
  settings = { ...settings, targetLanguage: "original" }; await render();
  expect(toggle().getAttribute("aria-checked")).toBe("true");
  expect(host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.settings.translateTo}"]`)!.disabled).toBe(true);
  await act(async () => toggle().click());
  expect(save).toHaveBeenLastCalledWith({ targetLanguage: "en" });
});

it.each(["zh", "en", "ja"] as const)("explains unknown custom support and service-default omission in %s", async locale => {
  setStoredUiLanguage(locale);
  settings = { ...settings, targetLanguage: "original", profiles: [{ ...settings.profiles[0], provider: "customDashScopeASR" }] };
  await render();
  expect(host.querySelector(".recognition-language-notice")?.textContent).toBe(I18N.settings.recognitionCustomNotice);
  expect(host.querySelector('[aria-label="' + I18N.settings.sourceLanguage + '"]')?.textContent).toContain(I18N.settings.recognitionServiceDefault);
  expect(host.querySelector('.settings-row .settings-help-control__description')?.textContent).toContain(I18N.settings.recognitionDashScopeParameter);
  await choose(I18N.settings.sourceLanguage, "fr");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "fr" });
});
it("saves a newly supported official output language through the searchable picker", async () => {
  settings = { ...settings, profiles: [{ ...settings.profiles[0], provider: "openAIRealtime" }] };
  await render();
  await choose(I18N.settings.translateTo, "fr");
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "fr" });
});

it("explains when enabling text translation actually resets a custom source", async () => {
  settings = { ...settings, sourceLanguage: "fr", targetLanguage: "original", profiles: [{ ...settings.profiles[0], provider: "customDashScopeASR", textTranslation: "deepL" }] };
  save.mockImplementation(async () => { settings = { ...settings, sourceLanguage: "auto", targetLanguage: "zh" }; });
  await render();
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.skipTranslation}"]`)!.click());
  await render();
  expect(host.textContent).toContain(I18N.settings.recognitionLanguageAdjusted("French", I18N.settings.recognitionServiceDefault));
});

it("shows the configured services in compact help and filters declared custom languages", async () => {
  settings = { ...settings, profiles: [{ ...settings.profiles[0], provider: "customOpenAIASR", textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: "Local MT" }, customSpeechSourceLanguages: ["en", "fr"] }] };
  await render();
  expect(host.querySelector(".profile-language-settings__heading .settings-help-control__description")?.textContent).toContain("Local MT");
  expect(host.querySelector(".recognition-language-notice")?.textContent).toBe(I18N.settings.recognitionDeclaredNotice);
  const sources = host.querySelector('[role="group"][aria-label="' + I18N.settings.sourceLanguage + '"]')!;
  expect([...sources.querySelectorAll("button")].map(button => button.textContent)).toEqual([I18N.settings.recognitionServiceDefault, "English", "French"]);
});

it("updates explicit selected classes with the actual saved language", async () => {
  settings = { ...settings, profiles: [{ ...settings.profiles[0], provider: "azureOpenAIRealtime" }] };
  await render();
  const choice = (name: string) => [...host.querySelectorAll<HTMLButtonElement>(".profile-language-choice")].find(button => button.textContent === name)!;
  expect(choice("English").classList.contains("is-selected")).toBe(true);
  settings = { ...settings, targetLanguage: "ja" }; await render();
  expect(choice("English").classList.contains("is-selected")).toBe(false);
  expect(choice("Japanese").classList.contains("is-selected")).toBe(true);
  expect(choice("Japanese").getAttribute("aria-pressed")).toBe("true");
});


it("explains a native session lock after a language save race without announcing success", async () => {
  save.mockRejectedValueOnce("Listening settings cannot be changed while a session is active.");
  await render();
  await choose(I18N.settings.sourceLanguage, "fr");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageChangeRequiresStop);
  expect(host.textContent).not.toContain(I18N.settings.languageSaved);
});

it("keeps Apple resource navigation reachable when no language is ready or the session blocks editing", async () => {
  settings = { ...settings, sourceLanguage: "en", targetLanguage: "original", activeProfileId: "apple",
    profiles: [{ id: "apple", name: "Apple", provider: "appleSpeech", credentialState: "present" }],
    languageCapabilities: { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original", sourceLanguages: [], targetLanguages: ["original"] } };
  const open = vi.fn();
  await act(async () => root.render(<ProfileLanguageSettings settings={settings} disabled requiresStop onOpenAppleResources={open} />));
  const resources = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.appleSpeechOpenResources)!;
  expect(resources.disabled).toBe(false);
  await act(async () => resources.click());
  expect(open).toHaveBeenCalledOnce();
  expect(save).not.toHaveBeenCalled();
});

it("omits a duplicate Apple source control while retaining independently configured translation controls", async () => {
  settings = { ...settings, sourceLanguage: "en", targetLanguage: "zh", activeProfileId: "apple",
    profiles: [{ id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "present", textTranslation: "deepL" }],
    languageCapabilities: { profileId: "apple", provider: "appleSpeech", textTranslation: "deepL", targetLanguage: "zh", sourceLanguages: ["en"], targetLanguages: ["original", "zh", "en", "ja"] } };
  await act(async () => root.render(<ProfileLanguageSettings settings={settings} disabled={false} hideSourceLanguage />));
  expect(host.querySelector(`[aria-label="${I18N.settings.sourceLanguage}"]`)).toBeNull();
  expect(host.querySelector(`[aria-label="${I18N.settings.translateTo}"]`)).not.toBeNull();
  settings = { ...settings, targetLanguage: "original", profiles: [{ ...settings.profiles[0], textTranslation: "followService" }], languageCapabilities: undefined };
  await act(async () => root.render(<ProfileLanguageSettings settings={settings} disabled={false} hideSourceLanguage />));
  expect(host.querySelector("section")).toBeNull();
});
