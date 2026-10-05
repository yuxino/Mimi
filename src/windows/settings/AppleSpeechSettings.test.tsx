// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { prepareAppleSpeechLanguage } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, type AppleSpeechSupport, type ServiceProfile, type SettingsSnapshot, type SourceLanguage } from "../../lib/types";
import { AppleSpeechSettings } from "./AppleSpeechSettings";
import { SettingsToastRegion } from "./SettingsToast";

vi.mock("../../lib/ipc", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  prepareAppleSpeechLanguage: vi.fn(),
}));
vi.mock("./AlibabaCredentialEditor", () => ({
  AlibabaCredentialEditor: ({ disabled }: { disabled: boolean }) => <div data-testid="translation-editor" aria-disabled={disabled} />,
}));

const initial = useStore.getState();
const profile: ServiceProfile = { id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "missing", textTranslation: "followService" };
const ready: AppleSpeechSupport = { available: true, languages: [
  { sourceLanguage: "en", locale: "en-US", installed: true },
  { sourceLanguage: "ja", locale: "ja-JP", installed: true },
  { sourceLanguage: "fr", locale: "fr-FR", installed: true },
] };
let host: HTMLDivElement, root: Root;
let props: ComponentProps<typeof AppleSpeechSettings>;
let save: ReturnType<typeof vi.fn>;

function settings(): SettingsSnapshot {
  return { ...initial.settings, profiles: [profile], activeProfileId: profile.id, sourceLanguage: "en", targetLanguage: "original",
    languageCapabilities: { profileId: profile.id, provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original", sourceLanguages: ["en", "ja", "fr"], targetLanguages: ["original"], appleSpeechSupportRevision: 1 } };
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  save = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, settings: settings(), saveSettings: save }, true);
  vi.mocked(prepareAppleSpeechLanguage).mockReset().mockResolvedValue(ready);
  props = { profile, settings: settings(), sourceLanguage: "en", support: ready, loading: false, failed: false,
    inputId: "synthetic-apple", disabled: false, busy: false, feedback: null, confirmingDelete: false,
    onRetry: vi.fn().mockResolvedValue(undefined), onPrepared: vi.fn(), onBusyChange: vi.fn(), onSelectProfile: vi.fn().mockResolvedValue(undefined),
    onSave: vi.fn().mockResolvedValue(undefined), onRequestDelete: vi.fn(), onConfirmDelete: vi.fn().mockResolvedValue(undefined), onCancelDelete: vi.fn() };
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});
async function render(overrides: Partial<typeof props> = {}) {
  props = { ...props, ...overrides };
  await act(async () => root.render(<><AppleSpeechSettings {...props} /><SettingsToastRegion /></>));
}
function button(label: string) {
  return [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === label);
}
async function choose(source: SourceLanguage, label = I18N.settings.sourceLanguage) {
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${label}"]`)!.click());
  const choice = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent?.startsWith(SOURCE_LANGUAGE_DISPLAY_NAMES[source]))!;
  await act(async () => choice.click());
}

it.each(["en", "zh", "ja"] as const)("keeps adding a language discoverable after the current language is downloaded in %s", async locale => {
  setStoredUiLanguage(locale);
  await render();
  const section = host.querySelector<HTMLElement>("#apple-speech-resources")!;
  expect(section.tabIndex).toBe(-1);
  expect(button(I18N.settings.appleSpeechAddLanguagePack)?.getAttribute("aria-expanded")).toBe("false");
  expect(button(I18N.settings.appleSpeechInstallHelp)?.getAttribute("aria-expanded")).toBe("false");
  expect(section.querySelector(".apple-speech-tutorial")).toBeNull();
  expect(section.querySelector(".apple-speech-pack-manager")).toBeNull();
  expect(section.textContent).toContain(I18N.settings.appleSpeechLanguageInUse);
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  expect(save).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

it("keeps the click-accessible installation guide available through loading and retry states", async () => {
  await render({ loading: true, support: null });
  const help = button(I18N.settings.appleSpeechInstallHelp)!;
  help.focus();
  await act(async () => help.click());
  expect(help.getAttribute("aria-expanded")).toBe("true");
  const tutorial = document.getElementById(help.getAttribute("aria-controls")!)!;
  expect(tutorial.querySelectorAll("li")).toHaveLength(3);
  expect(tutorial.textContent).toContain(I18N.settings.appleSpeechDownloadLocation);
  await render({ loading: false, failed: true });
  expect(host.textContent).toContain(I18N.settings.appleSpeechInstallStepDownload);
  expect(host.textContent).toContain(I18N.settings.appleSpeechLoadFailed);
  await act(async () => button(I18N.settings.retryLoadingSettings)!.click());
  expect(props.onRetry).toHaveBeenCalledOnce();
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  await act(async () => help.click());
  expect(document.getElementById(help.getAttribute("aria-controls")!)).toBeNull();
});

it("saves only after the explicit action and serializes duplicate clicks with resource controls", async () => {
  let resolve!: () => void;
  save.mockImplementation(() => new Promise<void>(done => { resolve = done; }));
  await render();
  await choose("ja");
  expect(save).not.toHaveBeenCalled();
  const use = button(I18N.settings.appleSpeechUseLanguage)!;
  await act(async () => { use.click(); use.click(); });
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "ja" });
  expect(use.disabled).toBe(true);
  expect(host.querySelector<HTMLButtonElement>('[role="combobox"]')!.disabled).toBe(true);
  expect(host.querySelector('[data-testid="translation-editor"]')?.getAttribute("aria-disabled")).toBe("true");
  expect(props.onBusyChange).toHaveBeenCalledExactlyOnceWith(true);
  await act(async () => resolve());
  expect(props.onBusyChange).toHaveBeenLastCalledWith(false);
  expect(host.textContent).toContain(I18N.settings.appleSpeechLanguageSelected);
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  await render({ settings: { ...props.settings, sourceLanguage: "ja" }, sourceLanguage: "ja" });
  expect(host.textContent).toContain(I18N.settings.appleSpeechLanguageInUse);
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
});

it("preparation alone never changes recognition and requires a ready snapshot before use", async () => {
  const unprepared = { ...ready, languages: ready.languages.map(language => ({ ...language, installed: language.sourceLanguage !== "ja" })) };
  await render({ support: unprepared, settings: { ...props.settings, languageCapabilities: { ...props.settings.languageCapabilities!, sourceLanguages: ["en", "fr"] } } });
  await act(async () => button(I18N.settings.appleSpeechAddLanguagePack)!.click());
  await choose("ja", I18N.settings.appleSpeechLanguagePack);
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  await act(async () => button(I18N.settings.appleSpeechPrepare)!.click());
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("ja");
  expect(props.onPrepared).toHaveBeenCalledWith(ready);
  expect(save).not.toHaveBeenCalled();
  expect(props.onSelectProfile).not.toHaveBeenCalled();
  expect(host.querySelector(".apple-speech-pack-manager")).toBeNull();
  await render({ support: ready, settings: settings() });
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeDefined();
  expect(save).not.toHaveBeenCalled();
});

it("uses the edited Apple profile and chosen language through one explicit action", async () => {
  const other: ServiceProfile = { id: "other", name: "Other", provider: "googleGeminiLive", credentialState: "present" };
  await render({ settings: { ...props.settings, profiles: [profile, other], activeProfileId: other.id, languageCapabilities: undefined } });
  await choose("ja");
  expect(props.onSelectProfile).not.toHaveBeenCalled();
  await act(async () => button(I18N.settings.appleSpeechUseLanguage)!.click());
  expect(props.onSelectProfile).toHaveBeenCalledExactlyOnceWith(profile.id, "ja");
  expect(save).not.toHaveBeenCalled();
});

it("keeps route-incompatible prepared resources available without offering a failed source switch", async () => {
  const deepL = { ...profile, textTranslation: "deepL" as const };
  const translated: SettingsSnapshot = { ...props.settings, profiles: [deepL], targetLanguage: "en",
    languageCapabilities: { ...props.settings.languageCapabilities!, textTranslation: "deepL", targetLanguage: "en", sourceLanguages: ["en"], targetLanguages: ["original", "en"] } };
  await render({ profile: deepL, settings: { ...translated, targetLanguage: "original", languageCapabilities: { ...translated.languageCapabilities!, targetLanguage: "original", sourceLanguages: ["en", "fr"] } } });
  await choose("fr");
  await render({ settings: translated });
  expect(host.textContent).toContain("fr-FR");
  expect(host.textContent).toContain(I18N.settings.appleSpeechLanguageRouteUnsupported);
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  expect(save).not.toHaveBeenCalled();
  await act(async () => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
  expect([...document.querySelectorAll('[role="option"]')].map(option => option.textContent)).toEqual([SOURCE_LANGUAGE_DISPLAY_NAMES.en]);
  await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  await render({ settings: { ...translated, targetLanguage: "original", languageCapabilities: { ...translated.languageCapabilities!, targetLanguage: "original", sourceLanguages: ["en", "fr"] } } });
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeDefined();
});

it("keeps an inactive profile's translator restrictions in its recognition choices", async () => {
  const other: ServiceProfile = { id: "other", name: "Other", provider: "googleGeminiLive", credentialState: "present" };
  const deepL = { ...profile, textTranslation: "deepL" as const };
  await render({ profile: deepL, sourceLanguage: "fr", settings: { ...props.settings, profiles: [deepL, other], activeProfileId: other.id, targetLanguage: "zh", languageCapabilities: undefined } });
  expect(host.textContent).toContain(I18N.settings.appleSpeechLanguageRouteUnsupported);
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  await act(async () => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
  expect([...document.querySelectorAll('[role="option"]')].map(option => option.textContent)).not.toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  await render({ settings: { ...props.settings, targetLanguage: "original" } });
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeDefined();
  expect(props.onSelectProfile).not.toHaveBeenCalled();
});

it("explains the stop requirement while allowing users to read the installation guide and language list", async () => {
  await render({ requiresStop: true });
  expect(host.textContent).toContain(I18N.settings.languageChangeRequiresStop);
  expect(host.querySelector<HTMLButtonElement>('[role="combobox"]')!.disabled).toBe(true);
  await act(async () => button(I18N.settings.appleSpeechAddLanguagePack)!.click());
  expect(host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.appleSpeechLanguagePack}"]`)!.disabled).toBe(true);
  await act(async () => button(I18N.settings.appleSpeechInstallHelp)!.click());
  expect(host.textContent).toContain(I18N.settings.appleSpeechDownloadLocation);
  expect(save).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

it.each(["disabled", "busy"] as const)("honors the parent %s guard", async state => {
  await render();
  await choose("ja");
  await render({ [state]: true });
  const use = button(I18N.settings.appleSpeechUseLanguage)!;
  expect(use.disabled).toBe(true);
  await act(async () => use.click());
  expect(save).not.toHaveBeenCalled();
});

it.each(["apple_speech_assets_missing", "private-native-error"])("shows only safe retryable feedback for %s", async error => {
  save.mockRejectedValue(new Error(error));
  await render();
  await choose("ja");
  await act(async () => button(I18N.settings.appleSpeechUseLanguage)!.click());
  expect(host.textContent).toContain(error === "apple_speech_assets_missing" ? I18N.settings.appleSpeechAssetsMissing : I18N.settings.languageSaveFailed);
  expect(document.body.textContent).not.toContain(error);
  expect(host.textContent).not.toContain(I18N.settings.appleSpeechLanguageSelected);
  expect(button(I18N.settings.appleSpeechUseLanguage)?.disabled).toBe(false);
  expect(props.onBusyChange).toHaveBeenLastCalledWith(false);
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

it("does not revive successful language feedback after navigating away", async () => {
  let resolve!: () => void;
  save.mockImplementation(() => new Promise<void>(done => { resolve = done; }));
  await render();
  await choose("ja");
  await act(async () => button(I18N.settings.appleSpeechUseLanguage)!.click());
  await act(async () => root.render(<SettingsToastRegion />));
  await act(async () => resolve());
  expect(document.body.textContent).not.toContain(I18N.settings.appleSpeechLanguageSelected);
});

it("lists download status for every supported language without downloading on open or selection", async () => {
  const unprepared = { ...ready, languages: ready.languages.map(language => ({ ...language, installed: language.sourceLanguage === "en" })) };
  await render({ support: unprepared });
  await act(async () => button(I18N.settings.appleSpeechAddLanguagePack)!.click());
  const selector = host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.appleSpeechLanguagePack}"]`)!;
  await act(async () => selector.click());
  const choices = [...document.querySelectorAll<HTMLElement>('[role="option"]')].map(option => option.textContent);
  expect(choices).toContain(`${SOURCE_LANGUAGE_DISPLAY_NAMES.en} · ${I18N.settings.appleSpeechInstalled}`);
  expect(choices).toContain(`${SOURCE_LANGUAGE_DISPLAY_NAMES.ja} · ${I18N.settings.appleSpeechNotInstalled}`);
  const french = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent?.startsWith(SOURCE_LANGUAGE_DISPLAY_NAMES.fr))!;
  await act(async () => french.click());
  expect(host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!.textContent).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.en);
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
});

it("serializes download clicks, preserves retry state, and keeps native failure details private", async () => {
  let reject!: (error: Error) => void;
  vi.mocked(prepareAppleSpeechLanguage).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  const unprepared = { ...ready, languages: ready.languages.map(language => ({ ...language, installed: language.sourceLanguage !== "ja" })) };
  await render({ support: unprepared });
  await act(async () => button(I18N.settings.appleSpeechAddLanguagePack)!.click());
  const download = button(I18N.settings.appleSpeechPrepare)!;
  await act(async () => { download.click(); download.click(); });
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("ja");
  expect(download.disabled).toBe(true);
  expect(button(I18N.settings.appleSpeechAddLanguagePack)?.disabled).toBe(true);
  expect(host.textContent).toContain(I18N.settings.appleSpeechPreparing);
  await act(async () => reject(new Error("private-native-error")));
  expect(host.querySelector(".apple-speech-pack-manager")?.textContent).toContain(I18N.settings.appleSpeechPrepareFailed);
  expect(host.textContent).not.toContain("private-native-error");
  expect(button(I18N.settings.appleSpeechRetryDownload)?.disabled).toBe(false);
  await act(async () => button(I18N.settings.appleSpeechRetryDownload)!.click());
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledTimes(2);
  expect(props.onPrepared).toHaveBeenCalledWith(ready);
  expect(save).not.toHaveBeenCalled();
  expect(props.onBusyChange).toHaveBeenLastCalledWith(false);
});

it("provides the add-language path with no downloaded languages and honors its stop lock", async () => {
  const missing = { ...ready, languages: ready.languages.map(language => ({ ...language, installed: false })) };
  await render({ support: missing, requiresStop: true });
  expect(host.querySelector<HTMLButtonElement>('[role="combobox"]')!.disabled).toBe(true);
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  await act(async () => button(I18N.settings.appleSpeechAddLanguagePack)!.click());
  expect(button(I18N.settings.appleSpeechPrepare)?.disabled).toBe(true);
  await act(async () => button(I18N.settings.appleSpeechPrepare)!.click());
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  await render({ requiresStop: false });
  expect(button(I18N.settings.appleSpeechPrepare)?.disabled).toBe(false);
});

it("keeps unsupported devices and empty capability lists out of download actions", async () => {
  for (const support of [{ available: false, languages: [] }, { available: true, languages: [] }]) {
    await render({ support });
    expect(host.textContent).toContain(I18N.settings.appleSpeechUnavailable);
    expect(button(I18N.settings.appleSpeechPrepare)).toBeUndefined();
    expect(button(I18N.settings.appleSpeechAddLanguagePack)).toBeUndefined();
    expect(button(I18N.settings.appleSpeechInstallHelp)).toBeDefined();
  }
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

it("checks the chosen recognition language rather than an unrelated language-pack selection", async () => {
  const connectionCheck = vi.fn().mockReturnValue(<span>Recognition check</span>);
  await render({ connectionCheck });
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined, "en");
  await choose("fr");
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined, "fr");
  await act(async () => button(I18N.settings.appleSpeechAddLanguagePack)!.click());
  await choose("ja", I18N.settings.appleSpeechLanguagePack);
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined, "fr");
  expect(save).not.toHaveBeenCalled();
});
