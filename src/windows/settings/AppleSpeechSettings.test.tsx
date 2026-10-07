// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { getAppleTranslationStatus, getAppleTranslationSupport, prepareAppleSpeechLanguage } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, type AppleSpeechSupport, type ServiceProfile, type SettingsSnapshot, type SourceLanguage } from "../../lib/types";
import { AppleSpeechSettings } from "./AppleSpeechSettings";
import { SettingsToastRegion } from "./SettingsToast";

vi.mock("../../lib/ipc", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  prepareAppleSpeechLanguage: vi.fn(),
  getAppleTranslationStatus: vi.fn(),
  getAppleTranslationSupport: vi.fn(),
}));
vi.mock("./AlibabaCredentialEditor", () => ({
  AlibabaCredentialEditor: ({ disabled, sourceLanguage, textConnectionCheck }: { disabled: boolean; sourceLanguage: SourceLanguage; textConnectionCheck?: unknown }) =>
    <div data-testid="translation-editor" aria-disabled={disabled} data-source-language={sourceLanguage} data-text-check={textConnectionCheck ? "available" : "unavailable"} />,
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
  vi.mocked(getAppleTranslationStatus).mockReset().mockResolvedValue("supported");
  vi.mocked(getAppleTranslationSupport).mockReset().mockResolvedValue({ available: true, sourceLanguages: ["en", "ja", "fr"], targetLanguages: ["en", "ja", "fr"] });
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

it.each(["en", "zh", "ja"] as const)("keeps one coherent recognition selector and a collapsed guide in %s", async locale => {
  setStoredUiLanguage(locale);
  await render();
  const section = host.querySelector<HTMLElement>("#apple-speech-resources")!;
  expect(section.tabIndex).toBe(-1);
  expect(section.querySelectorAll('[role="combobox"]')).toHaveLength(1);
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

it("downloads and applies the same selected language in one explicit action", async () => {
  const missing = { ...ready, languages: ready.languages.map(item => ({ ...item, installed: item.sourceLanguage !== "ja" })) };
  await render({ support: missing });
  await choose("ja");
  expect(host.querySelectorAll('[role="combobox"]')).toHaveLength(1);
  expect(host.textContent).toContain("ja-JP");
  expect(save).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("ja");
  expect(props.onPrepared).toHaveBeenCalledWith(ready);
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "ja" });
  expect(props.onSelectProfile).not.toHaveBeenCalled();
});

it("does not apply a language when preparation returns an unready resource", async () => {
  const missing = { ...ready, languages: ready.languages.map(item => ({ ...item, installed: false })) };
  vi.mocked(prepareAppleSpeechLanguage).mockResolvedValue(missing);
  await render({ support: missing });
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(save).not.toHaveBeenCalled();
  expect(props.onSelectProfile).not.toHaveBeenCalled();
  expect(host.textContent).toContain(I18N.settings.appleSpeechPrepareFailed);
  expect(button(I18N.settings.appleSpeechRetryDownload)?.disabled).toBe(false);
});

it("reports a save failure separately after a successful download", async () => {
  const missing = { ...ready, languages: ready.languages.map(item => ({ ...item, installed: false })) };
  save.mockRejectedValue(new Error("private-save-detail"));
  await render({ support: missing });
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  await render({ support: ready });
  expect(host.textContent).toContain(I18N.settings.languageSaveFailed);
  expect(host.textContent).not.toContain(I18N.settings.appleSpeechPrepareFailed);
  expect(host.textContent).not.toContain("private-save-detail");
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
  expect([...document.querySelectorAll('[role="option"]')].map(option => option.textContent)).toEqual(ready.languages.map(item => SOURCE_LANGUAGE_DISPLAY_NAMES[item.sourceLanguage]));
  await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  await render({ settings: { ...translated, targetLanguage: "original", languageCapabilities: { ...translated.languageCapabilities!, targetLanguage: "original", sourceLanguages: ["en", "fr"] } } });
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeDefined();
});

it("keeps an inactive profile's translator restrictions in its recognition choices", async () => {
  const other: ServiceProfile = { id: "other", name: "Other", provider: "googleGeminiLive", credentialState: "present" };
  const deepL = { ...profile, textTranslation: "deepLX" as const };
  await render({ profile: deepL, sourceLanguage: "th", support: { available: true, languages: [{ sourceLanguage: "en", locale: "en-US", installed: true }, { sourceLanguage: "th", locale: "th-TH", installed: true }] }, settings: { ...props.settings, profiles: [deepL, other], activeProfileId: other.id, targetLanguage: "zh", languageCapabilities: undefined } });
  expect(host.textContent).toContain(I18N.settings.appleSpeechLanguageRouteUnsupported);
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  await act(async () => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
  expect([...document.querySelectorAll('[role="option"]')].map(option => option.textContent)).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.th);
  await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  await render({ settings: { ...props.settings, targetLanguage: "original" } });
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeDefined();
  expect(props.onSelectProfile).not.toHaveBeenCalled();
});

it("keeps the selected pack and stop requirement together without another language selector", async () => {
  const missing = { ...ready, languages: ready.languages.map(item => ({ ...item, installed: false })) };
  await render({ requiresStop: true, support: missing, sourceLanguage: "ja", settings: { ...settings(), sourceLanguage: "ja" } });
  expect(host.textContent).toContain(I18N.settings.languageChangeRequiresStop);
  expect(host.textContent).toContain("ja-JP");
  expect(host.querySelectorAll('[role="combobox"]')).toHaveLength(1);
  expect(host.querySelector<HTMLButtonElement>('[role="combobox"]')!.disabled).toBe(true);
  expect(button(I18N.settings.appleSpeechDownloadAndUse)?.disabled).toBe(true);
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

it("lists ready and missing languages together without downloading on selection", async () => {
  const missing = { ...ready, languages: ready.languages.map(item => ({ ...item, installed: item.sourceLanguage === "en" })) };
  await render({ support: missing });
  await act(async () => host.querySelector<HTMLButtonElement>('[role="combobox"]')!.click());
  const choices = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(choices.map(option => option.textContent)).toEqual([
    SOURCE_LANGUAGE_DISPLAY_NAMES.en,
    `${SOURCE_LANGUAGE_DISPLAY_NAMES.ja} · ${I18N.settings.appleSpeechNotInstalled}`,
    `${SOURCE_LANGUAGE_DISPLAY_NAMES.fr} · ${I18N.settings.appleSpeechNotInstalled}`,
  ]);
  await act(async () => choices[1].click());
  expect(host.querySelector('[role="combobox"]')?.textContent).toBe(`${SOURCE_LANGUAGE_DISPLAY_NAMES.ja} · ${I18N.settings.appleSpeechNotInstalled}`);
  expect(host.textContent).toContain("ja-JP");
  expect(button(I18N.settings.appleSpeechDownloadAndUse)).toBeDefined();
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
});

it("serializes download clicks, preserves retry state, and keeps native failure details private", async () => {
  let reject!: (error: Error) => void;
  vi.mocked(prepareAppleSpeechLanguage).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  const unprepared = { ...ready, languages: ready.languages.map(language => ({ ...language, installed: language.sourceLanguage !== "ja" })) };
  await render({ support: unprepared });
  await choose("ja");
  const download = button(I18N.settings.appleSpeechDownloadAndUse)!;
  await act(async () => { download.click(); download.click(); });
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("ja");
  expect(download.disabled).toBe(true);
  expect(host.querySelector<HTMLButtonElement>('[role="combobox"]')?.disabled).toBe(true);
  expect(host.textContent).toContain(I18N.settings.appleSpeechPreparing);
  await act(async () => reject(new Error("private-native-error")));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.settings.appleSpeechPrepareFailed);
  expect(host.textContent).not.toContain("private-native-error");
  expect(button(I18N.settings.appleSpeechRetryDownload)?.disabled).toBe(false);
  await act(async () => button(I18N.settings.appleSpeechRetryDownload)!.click());
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledTimes(2);
  expect(props.onPrepared).toHaveBeenCalledWith(ready);
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "ja" });
  expect(props.onBusyChange).toHaveBeenLastCalledWith(false);
});

it("provides the add-language path with no downloaded languages and honors its stop lock", async () => {
  const missing = { ...ready, languages: ready.languages.map(language => ({ ...language, installed: false })) };
  await render({ support: missing, requiresStop: true });
  expect(host.querySelector<HTMLButtonElement>('[role="combobox"]')!.disabled).toBe(true);
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  expect(button(I18N.settings.appleSpeechDownloadAndUse)?.disabled).toBe(true);
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  await render({ requiresStop: false });
  expect(button(I18N.settings.appleSpeechDownloadAndUse)?.disabled).toBe(false);
});

it("keeps unsupported devices and empty capability lists out of download actions", async () => {
  for (const support of [{ available: false, languages: [] }, { available: true, languages: [] }]) {
    await render({ support });
    expect(host.textContent).toContain(I18N.settings.appleSpeechUnavailable);
    expect(button(I18N.settings.appleSpeechDownloadAndUse)).toBeUndefined();
    expect(host.querySelector('.apple-speech-pack-manager')).toBeNull();
    expect(button(I18N.settings.appleSpeechInstallHelp)).toBeDefined();
  }
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

it("checks the same language displayed by the selector and resource status", async () => {
  const connectionCheck = vi.fn().mockReturnValue(<span>Recognition check</span>);
  await render({ connectionCheck });
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined, "en");
  await choose("fr");
  expect(connectionCheck).toHaveBeenLastCalledWith(undefined, "fr");
  expect(host.textContent).toContain("fr-FR");
  expect(save).not.toHaveBeenCalled();


});

it("previews the selected Speech draft in translation without changing saved preferences", async () => {
  const textConnectionCheck = vi.fn();
  await render({ textConnectionCheck });
  const translation = host.querySelector('[data-testid="translation-editor"]')!;
  expect(translation.getAttribute("data-source-language")).toBe("en");
  expect(translation.getAttribute("data-text-check")).toBe("available");
  await choose("ja");
  expect(translation.getAttribute("data-source-language")).toBe("ja");
  // Native text checks use saved preferences and cannot yet override the source.
  expect(translation.getAttribute("data-text-check")).toBe("unavailable");
  expect(useStore.getState().settings.sourceLanguage).toBe("en");
  expect(save).not.toHaveBeenCalled();
  expect(props.onSelectProfile).not.toHaveBeenCalled();
  await render({ settings: { ...props.settings, sourceLanguage: "ja" } });
  expect(translation.getAttribute("data-source-language")).toBe("ja");
  expect(translation.getAttribute("data-text-check")).toBe("available");
});

it("uses the visible Speech language instead of another active provider's Auto source for translation", async () => {
  const google: ServiceProfile = { id: "google", name: "Google", provider: "googleGeminiLive", credentialState: "present" };
  const snapshot = { ...settings(), profiles: [profile, google], activeProfileId: google.id, sourceLanguage: "auto" as const, languageCapabilities: undefined };
  useStore.setState({ settings: snapshot });
  await render({ settings: snapshot, sourceLanguage: "auto", textConnectionCheck: vi.fn() });
  const translation = host.querySelector('[data-testid="translation-editor"]')!;
  expect(host.querySelector('.apple-speech-resource-status')?.textContent).toContain("en-US");
  expect(translation.getAttribute("data-source-language")).toBe("en");
  expect(translation.getAttribute("data-text-check")).toBe("unavailable");
  await choose("ja");
  expect(translation.getAttribute("data-source-language")).toBe("ja");
  expect(useStore.getState().settings.sourceLanguage).toBe("auto");
  expect(save).not.toHaveBeenCalled();
  expect(props.onSelectProfile).not.toHaveBeenCalled();
});


it("does not apply a download after the active profile changes", async () => {
  let resolve!: (support: AppleSpeechSupport) => void;
  vi.mocked(prepareAppleSpeechLanguage).mockReturnValue(new Promise(done => { resolve = done; }));
  const missing = { ...ready, languages: ready.languages.map(item => ({ ...item, installed: false })) };
  await render({ support: missing });
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  await render({ settings: { ...props.settings, activeProfileId: "changed" } });
  await act(async () => resolve(ready));
  expect(save).not.toHaveBeenCalled();
  expect(props.onSelectProfile).not.toHaveBeenCalled();
  expect(host.textContent).not.toContain(I18N.settings.appleSpeechLanguageSelected);
});


it("follows a source change from floating controls without reviving an old language draft", async () => {
  await render();
  await choose("ja");
  await render({ settings: { ...settings(), showSubtitleTimestamps: true } });
  expect(host.textContent).toContain("ja-JP");
  await render({ settings: { ...settings(), sourceLanguage: "fr" } });
  expect(host.querySelector('[role="combobox"]')?.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.fr);
  expect(host.querySelector('.apple-speech-resource-status')?.textContent).toContain("fr-FR");
  expect(button(I18N.settings.appleSpeechUseLanguage)).toBeUndefined();
  await render({ settings: settings() });
  expect(host.querySelector('[role="combobox"]')?.textContent).toBe(SOURCE_LANGUAGE_DISPLAY_NAMES.en);
  expect(save).not.toHaveBeenCalled();
});

async function renderMissingAppleTranslationSource(targetLanguage: SettingsSnapshot["targetLanguage"] = "fr") {
  const apple = { ...profile, textTranslation: "apple" as const };
  const translated = { ...settings(), profiles: [apple], targetLanguage,
    languageCapabilities: { ...settings().languageCapabilities!, textTranslation: "apple" as const, targetLanguage, sourceLanguages: ["en" as const], targetLanguages: ["original", "en", "ja", "fr"] as const } };
  await render({ profile: apple, settings: translated,
    support: { ...ready, languages: ready.languages.map(item => ({ ...item, installed: item.sourceLanguage !== "ja" })) } });
  await choose("ja");
}

it.each(["supported", "installed"] as const)("checks the Apple text pair before downloading Speech resources when %s", async status => {
  vi.mocked(getAppleTranslationStatus).mockResolvedValue(status);
  await renderMissingAppleTranslationSource();
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(getAppleTranslationStatus).toHaveBeenCalledExactlyOnceWith("ja", "fr");
  expect(vi.mocked(getAppleTranslationStatus).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(prepareAppleSpeechLanguage).mock.invocationCallOrder[0]!);
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("ja");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "ja" });
});

it.each(["unsupported", "unavailable"] as const)("does not download Speech resources for an Apple text pair that is %s", async status => {
  vi.mocked(getAppleTranslationStatus).mockResolvedValue(status);
  await renderMissingAppleTranslationSource();
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
  expect(host.querySelector('.settings-inline-feedback')?.textContent ?? host.textContent).toContain(status === "unsupported" ? I18N.settings.appleTranslationUnsupported : I18N.settings.appleTranslationUnavailable);
  expect(host.textContent).not.toContain(I18N.settings.appleSpeechPrepareFailed);
  expect(button(I18N.settings.appleSpeechRetryDownload)).toBeUndefined();
  expect(props.onBusyChange).toHaveBeenLastCalledWith(false);
});

it("fails closed with sanitized translation feedback when pair discovery fails", async () => {
  vi.mocked(getAppleTranslationStatus).mockRejectedValue(new Error("private-translation-status-detail"));
  await renderMissingAppleTranslationSource();
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
  expect(host.textContent).toContain(I18N.settings.appleTranslationStatusFailed);
  expect(host.textContent).not.toContain("private-translation-status-detail");
});

it("allows Original mode to download Speech resources without querying text translation", async () => {
  await renderMissingAppleTranslationSource("original");
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(getAppleTranslationStatus).not.toHaveBeenCalled();
  expect(getAppleTranslationSupport).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("ja");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "ja" });
});

it("permits native same-language passthrough without requiring a translation pair model", async () => {
  vi.mocked(getAppleTranslationStatus).mockResolvedValue("unsupported");
  await renderMissingAppleTranslationSource("ja");
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(getAppleTranslationSupport).toHaveBeenCalledOnce();
  expect(getAppleTranslationStatus).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("ja");
  expect(save).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "ja" });
});

it.each([
  { available: false, sourceLanguages: ["ja"], targetLanguages: ["ja"] },
  { available: true, sourceLanguages: ["en"], targetLanguages: ["ja"] },
  { available: true, sourceLanguages: ["ja"], targetLanguages: ["en"] },
] as const)("requires actual native language support even for same-language passthrough", async support => {
  vi.mocked(getAppleTranslationSupport).mockResolvedValue({ available: support.available, sourceLanguages: [...support.sourceLanguages], targetLanguages: [...support.targetLanguages] });
  await renderMissingAppleTranslationSource("ja");
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
  expect(host.textContent).toContain(I18N.settings.appleTranslationUnavailable);
});

it("does not start a resource download after the target changes during the translation check", async () => {
  let resolve!: (status: "supported") => void;
  vi.mocked(getAppleTranslationStatus).mockReturnValue(new Promise(done => { resolve = done; }));
  await renderMissingAppleTranslationSource();
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  await render({ settings: { ...props.settings, targetLanguage: "en" } });
  await act(async () => resolve("supported"));
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
  expect(host.textContent).toContain(I18N.settings.languageSwitchSuperseded);
});

it("does not start a resource download after leaving during the translation check", async () => {
  let resolve!: (status: "supported") => void;
  vi.mocked(getAppleTranslationStatus).mockReturnValue(new Promise(done => { resolve = done; }));
  await renderMissingAppleTranslationSource();
  await act(async () => button(I18N.settings.appleSpeechDownloadAndUse)!.click());
  await act(async () => root.render(<SettingsToastRegion />));
  await act(async () => resolve("supported"));
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
});
