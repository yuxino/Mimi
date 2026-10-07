// @vitest-environment jsdom
import languageCatalogs from "../../../shared/provider-language-catalogs.json";
import { SettingsToastRegion } from "./SettingsToast";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot } from "../../lib/types";
import { ProfileLanguageSettings } from "./ProfileLanguageSettings";

const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
let save: ReturnType<typeof vi.fn>;
let switchSource: ReturnType<typeof vi.fn>, switchTarget: ReturnType<typeof vi.fn>;
let settings: SettingsSnapshot;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  settings = { ...initial.settings, sourceLanguage: "auto", targetLanguage: "en", languageCapabilities: undefined,
    profiles: [{ id: "test", name: "Test", provider: "alibabaCloud", credentialState: "present" }], activeProfileId: "test" };
  save = vi.fn().mockResolvedValue(undefined);
  switchSource = vi.fn().mockResolvedValue(undefined); switchTarget = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, settings, saveSettings: save, switchSourceLanguage: switchSource, switchTargetLanguage: switchTarget }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});
async function render(disabled = false) {
  await act(async () => root.render(<><ProfileLanguageSettings settings={settings} disabled={disabled} /><SettingsToastRegion /></>));
}
async function choose(label: string, code: string) {
  const trigger = host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
  await act(() => trigger.click());
  const search = document.querySelector<HTMLInputElement>('input.mimi-select__search');
  if (search) await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, code);
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  const option = options.find(node => node.textContent === TARGET_LANGUAGE_DISPLAY_NAMES[code as keyof typeof TARGET_LANGUAGE_DISPLAY_NAMES]) ?? options[0];
  await act(async () => option.click());
}

it("keeps saved Apple text languages visible when the runtime catalog is unavailable", async () => {
  settings = { ...settings, sourceLanguage: "fr", targetLanguage: "ja", profiles: [{ ...settings.profiles[0], textTranslation: "apple" }] };
  await render();
  const source = host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!;
  const target = host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!;
  expect(source.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  expect(source.disabled).toBe(true);
  expect(target.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.ja);
  expect(target.disabled).toBe(false);
  expect(save).not.toHaveBeenCalled();
});

it("keeps one embedded target picker usable from original-only without a duplicate skip control", async () => {
  settings = { ...settings, sourceLanguage: "en", targetLanguage: "original", profiles: [{ ...settings.profiles[0], textTranslation: "apple" }],
    languageCapabilities: { profileId: "test", provider: "alibabaCloud", textTranslation: "apple", targetLanguage: "original", sourceLanguages: ["en"], targetLanguages: ["original", "zh", "ja"] } };
  await act(async () => root.render(<ProfileLanguageSettings settings={settings} disabled={false} embedded hideSourceLanguage />));
  expect(host.querySelector("section, h3, [role=switch]")).toBeNull();
  expect(host.querySelectorAll('[role="combobox"]')).toHaveLength(1);
  expect(host.querySelector<HTMLButtonElement>('[role="combobox"]')!.disabled).toBe(false);
  await choose(I18N.settings.translateTo, "ja");
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "ja" });
});

it("lets the embedded Apple target recover to original-only when the native catalog disappears", async () => {
  settings = { ...settings, sourceLanguage: "en", targetLanguage: "ja", profiles: [{ ...settings.profiles[0], textTranslation: "apple" }] };
  await act(async () => root.render(<ProfileLanguageSettings settings={settings} disabled={false} embedded hideSourceLanguage />));
  await choose(I18N.settings.translateTo, "original");
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "original" });
});

it("lets Apple Speech fall back to original-only when Apple translation is unavailable", async () => {
  settings = { ...settings, sourceLanguage: "en", targetLanguage: "ja", profiles: [{ ...settings.profiles[0], provider: "appleSpeech", textTranslation: "apple" }] };
  await act(async () => root.render(<ProfileLanguageSettings settings={settings} disabled={false} hideSourceLanguage />));
  const skip = host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.skipTranslation}"]`)!;
  expect(skip).not.toBeNull();
  expect(skip.disabled).toBe(false);
  await act(async () => skip.click());
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "original" });
});

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
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!.click());
  const choices = [...document.querySelectorAll<HTMLButtonElement>('[role="option"]')];
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
  const selected = picker();
  expect(selected.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.en);
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
  settings = { ...settings, sourceLanguage: "ja", profiles: [{ ...settings.profiles[0], provider: "volcanoEngine" }] };
  await render();
  const targetPicker = host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!;
  await act(async () => targetPicker.click());
  const button = [...document.querySelectorAll<HTMLButtonElement>('[role="option"]')].find(node => node.textContent === TARGET_LANGUAGE_DISPLAY_NAMES.zh)!;
  await act(() => { button.click(); button.click(); });
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "zh" });
  expect(host.querySelector("section")?.getAttribute("aria-busy")).toBe("true");
  expect(targetPicker.disabled).toBe(true);
  await act(async () => fail(new Error("synthetic-private-provider-body")));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.languageSaveFailed);
  expect(host.textContent).not.toContain("synthetic-private-provider-body");
  expect(targetPicker.textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.en);
  expect(targetPicker.disabled).toBe(false);
});

it("disables language controls during a transition without asking to stop an already transitioning session", async () => {
  await render(true);
  expect(host.querySelectorAll('button:disabled')).toHaveLength(3);
  expect(host.textContent).not.toContain(I18N.settings.languageChangeRequiresStop);
  expect(save).not.toHaveBeenCalled();
});

it.each([false, true])("uses the session switch commands while active or paused (paused=%s)", async isPaused => {
  useStore.setState({ session: { ...initial.session, isActive: !isPaused, isPaused, status: { kind: "listening" } } });
  await render();
  await choose(I18N.settings.sourceLanguage, "fr");
  await choose(I18N.settings.translateTo, "ja");
  expect(switchSource).toHaveBeenCalledExactlyOnceWith("fr");
  expect(switchTarget).toHaveBeenCalledExactlyOnceWith("ja");
  expect(save).not.toHaveBeenCalled();
  expect(host.textContent).not.toContain(I18N.settings.languageChangeRequiresStop);
});

it("retains the target and reports preparation failure when an active Apple target is not ready", async () => {
  settings = { ...settings, sourceLanguage: "en", targetLanguage: "zh", profiles: [{ ...settings.profiles[0], textTranslation: "apple" }],
    languageCapabilities: { profileId: "test", provider: "alibabaCloud", textTranslation: "apple", targetLanguage: "zh", sourceLanguages: ["en"], targetLanguages: ["original", "zh", "ja"] } };
  useStore.setState({ session: { ...initial.session, isActive: true, status: { kind: "listening" } } });
  switchTarget.mockRejectedValueOnce(new Error("apple_translation_assets_missing"));
  const onBusy = vi.fn();
  await act(async () => root.render(<><ProfileLanguageSettings settings={settings} disabled={false} embedded onBusyChange={onBusy} /><SettingsToastRegion /></>));
  await choose(I18N.settings.translateTo, "ja");
  expect(switchTarget).toHaveBeenCalledExactlyOnceWith("ja");
  expect(save).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.appleTranslationAssetsMissing);
  expect(host.textContent).not.toContain("apple_translation_assets_missing");
  expect(host.querySelector('[aria-label="' + I18N.settings.translateTo + '"]')?.textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.zh);
  expect(onBusy.mock.calls).toEqual([[true], [false]]);
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
  settings = { ...settings, sourceLanguage: "ak", targetLanguage: "original", profiles: [{ ...settings.profiles[0], provider: "customDashScopeASR", textTranslation: "deepL" }] };
  save.mockImplementation(async draft => { settings = { ...settings, sourceLanguage: "auto", targetLanguage: draft.targetLanguage }; });
  await render();
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.skipTranslation}"]`)!.click());
  await render();
  expect(host.textContent).toContain(I18N.settings.recognitionLanguageAdjusted("Akan", I18N.settings.recognitionServiceDefault));
});

it("shows the configured services in compact help and filters declared custom languages", async () => {
  settings = { ...settings, profiles: [{ ...settings.profiles[0], provider: "customOpenAIASR", textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: "Local MT" }, customSpeechSourceLanguages: ["en", "fr"] }] };
  await render();
  expect(host.querySelector(".profile-language-settings__heading .settings-help-control__description")?.textContent).toContain("Local MT");
  expect(host.querySelector(".recognition-language-notice")?.textContent).toBe(I18N.settings.recognitionDeclaredNotice);
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!.click());
  expect([...document.querySelectorAll('[role="option"]')].map(option => option.textContent)).toEqual([I18N.settings.recognitionServiceDefault, "English", "French"]);
});

it("shows the actual saved target in the aligned picker", async () => {
  settings = { ...settings, sourceLanguage: "ja", profiles: [{ ...settings.profiles[0], provider: "volcanoEngine" }] };
  await render();
  const picker = () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!;
  expect(picker().textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.en);
  settings = { ...settings, targetLanguage: "zh" }; await render();
  expect(picker().textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.zh);
  await act(async () => picker().click());
  expect(document.querySelector('[role="option"][aria-selected="true"]')?.textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.zh);
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
  await act(async () => root.render(<ProfileLanguageSettings settings={settings} disabled onOpenAppleResources={open} />));
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


it.each(["zh", "en", "ja"] as const)("shows every Gemini target and saves the exact regional code in %s", async locale => {
  setStoredUiLanguage(locale);
  settings = { ...settings, profiles: [{ ...settings.profiles[0], provider: "googleGeminiLive" }] };
  await render();
  const automatic = host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!;
  expect(automatic.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.auto);
  expect(automatic.disabled).toBe(true);
  await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${I18N.settings.translateTo}"]`)!.click());
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent))
    .toEqual(languageCatalogs.googleGeminiLive.targetLanguages.map(code => TARGET_LANGUAGE_DISPLAY_NAMES[code as keyof typeof TARGET_LANGUAGE_DISPLAY_NAMES]));
  await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${I18N.settings.translateTo}"]`)!.click());
  await choose(I18N.settings.translateTo, "pt-BR");
  expect(save).toHaveBeenCalledExactlyOnceWith({ targetLanguage: "pt-BR" });
});

it("keeps Tencent targets valid for the current source and labels mixed input explicitly", async () => {
  settings = { ...settings, sourceLanguage: "ru", targetLanguage: "en", profiles: [{ ...settings.profiles[0], provider: "tencentCloud" }] };
  await render();
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!.click());
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([TARGET_LANGUAGE_DISPLAY_NAMES.zh, TARGET_LANGUAGE_DISPLAY_NAMES.en, TARGET_LANGUAGE_DISPLAY_NAMES.ru]);
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!.click());
  await choose(I18N.settings.sourceLanguage, "zh_en");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "zh_en" });
});
