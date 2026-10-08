// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { diagnosticCopy, profileErrorMessage } from "../../lib/connectionDiagnostics";
import { speechLanguageGuidance, targetLanguageOptionLabel } from "../../lib/speechLanguageGuidance";
import { I18N, providerDisplayName, setStoredUiLanguage } from "../../lib/i18n";
import { getAppleSpeechSupport, prepareAppleSpeechLanguage, profileCredentialEditorState, profileRevealCredential, testProfileConnection } from "../../lib/ipc";
import { SERVICE_PROVIDERS, sourceLanguagesForSettings, targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES } from "../../lib/types";
import type { AppleSpeechSupport, ServiceProfile, SettingsSnapshot } from "../../lib/types";
import { ServiceProfiles } from "./ServiceProfiles";

const actions = vi.hoisted(() => ({
  createProfile: vi.fn(), updateProfile: vi.fn(), selectProfile: vi.fn(),
  saveSettings: vi.fn(), deleteProfile: vi.fn(), saveProfileCredentials: vi.fn(), deleteProfileAPIKey: vi.fn(),
}));
const boot = vi.hoisted(() => ({ initializationStatus: "ready" as "ready" | "loading" | "error", initializationError: null as "timeout" | "unavailable" | null, init: vi.fn(), nativeSettings: null as SettingsSnapshot | null }));
vi.mock("../../lib/store", () => ({ useStore: (select: (state: typeof actions & typeof boot & { settings: { windowsAudioSource?: string }; session: { isActive: boolean; isPaused: boolean } }) => unknown) => select({ ...actions, ...boot, settings: boot.nativeSettings ?? { windowsAudioSource: "" }, session: { isActive: false, isPaused: false } }) }));
vi.mock("../../lib/ipc", () => ({ isTauri: false, getAppleSpeechSupport: vi.fn(), prepareAppleSpeechLanguage: vi.fn(), testProfileConnection: vi.fn(), profileRevealCredential: vi.fn(), profileCredentialEditorState: vi.fn(), setOverlayPointerCursor: vi.fn() }));

const profile: ServiceProfile = { id: "synthetic", name: "Alibaba", provider: "alibabaCloud", credentialState: "unavailable" };
const settings: SettingsSnapshot = {
  profiles: [profile], activeProfileId: profile.id, sourceLanguage: "auto", targetLanguage: "zh",
  translationMode: "turbo", fontSize: 18, subtitleBackgroundOpacity: 80, subtitleColor: "white", subtitleAlignment: "center",
  subtitleDisplayMode: "translation", pulseAnimation: null, pulseStyle: "ribbon", subtitleAnimation: null,
  showSubtitleDividers: false,
  subtitleBlendsWithBackground: false, isOverlayLocked: false, uiLanguage: "en",
  retainSessionHistory: false, recordSessionAudio: false, audioInput: "system", windowsAudioSource: "", systemAudioTarget: { kind: "system" }, showInDock: false,
  networkProxy: { mode: "system", url: null },
};
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  Element.prototype.scrollIntoView = vi.fn();
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla Linux");
  for (const action of Object.values(actions)) action.mockReset();
  vi.mocked(testProfileConnection).mockReset();
  vi.mocked(profileRevealCredential).mockReset();
  vi.mocked(profileCredentialEditorState).mockReset().mockResolvedValue({ savedFields: ["apiKey"] });
  vi.mocked(getAppleSpeechSupport).mockReset().mockResolvedValue({ available: false, languages: [] });
  vi.mocked(prepareAppleSpeechLanguage).mockReset();
  boot.initializationStatus = "ready"; boot.initializationError = null; boot.init.mockReset().mockResolvedValue(undefined);
  boot.nativeSettings = null;
  setStoredUiLanguage("en");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals();
  vi.useRealTimers();
});
async function render(snapshot = settings, sessionStatusKind: "idle" | "error" = "idle") { await act(async () => root.render(<><ServiceProfiles settings={snapshot} sessionIsActive={false} sessionStatusKind={sessionStatusKind} /><SettingsToastRegion /></>)); }

it.each([undefined, "localFile"] as const)("saves the Alibaba text model for %s profiles without changing credentials", async credentialStorage => {
  const ready = { ...profile, credentialState: "present" as const, credentialStorage };
  const initial = { ...settings, profiles: [ready] };
  const changed = { ...initial, profiles: [{ ...ready, qwenMtModel: "flash" as const }] };
  actions.updateProfile.mockResolvedValue(changed);
  await render(initial);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const picker = host.querySelector<HTMLButtonElement>('[role="combobox"][aria-label="Translation model"]')!;
  expect(picker.textContent).toBe("Qwen-MT Lite（fast · streaming）");
  await act(async () => picker.click());
  const options = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(options.map(option => option.textContent)).toEqual(["Qwen-MT Lite（fast · streaming）", "Qwen-MT Flash（balanced · streaming）", "Qwen-MT Plus（quality · full text）"]);
  await act(async () => options[1].click());
  expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith(ready.id, undefined, { qwenMtModel: "flash" });
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
  await render(changed);
  expect(host.querySelector('[role="combobox"][aria-label="Translation model"]')!.textContent).toBe("Qwen-MT Flash（balanced · streaming）");
  expect(document.querySelector('.settings-toast')).toBeNull();
});

it("keeps the saved model and reports a failed model save through the toast", async () => {
  actions.updateProfile.mockRejectedValue(new Error("synthetic-private-error"));
  await render({ ...settings, profiles: [{ ...profile, credentialState: "present" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await act(async () => host.querySelector<HTMLButtonElement>('[role="combobox"][aria-label="Translation model"]')!.click());
  const plus = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent === "Qwen-MT Plus（quality · full text）")!;
  await act(async () => plus.click());
  expect(host.querySelector('[role="combobox"][aria-label="Translation model"]')!.textContent).toBe("Qwen-MT Lite（fast · streaming）");
  expect(document.body.textContent).toContain("Could not save the translation model. Try again.");
  expect(document.body.textContent).not.toContain("synthetic-private-error");
});

it.each([false, true])("disables the model picker while subtitles are active (paused=%s)", async paused => {
  await act(async () => root.render(<ServiceProfiles settings={settings} sessionIsActive={!paused} sessionIsPaused={paused} profileEditorRequest={1} />));
  expect(host.querySelector<HTMLButtonElement>('[role="combobox"][aria-label="Translation model"]')!.disabled).toBe(true);
});

const appleSupport: AppleSpeechSupport = { available: true, languages: [{ sourceLanguage: "en" as const, locale: "en-US", installed: false, status: "supported" }, { sourceLanguage: "ja" as const, locale: "ja-JP", installed: true }] };
const appleProfile: ServiceProfile = { id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "missing", speechCredentialState: "missing", textCredentialState: "missing", textTranslation: "followService" };
it("remembers current languages without switching and lets the active configuration restore them later", async () => {
  const ready = { ...profile, credentialState: "present" as const };
  const pair = { sourceLanguage: "ja" as const, targetLanguage: "zh" as const };
  const saved = { ...ready, languagePreset: pair };
  const initial = { ...settings, ...pair, profiles: [ready] };
  actions.updateProfile.mockResolvedValue({ ...initial, profiles: [saved] });
  await render(initial);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.profileLanguagesRemember);
  expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith(ready.id, undefined, { languagePreset: pair });
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(actions.saveSettings).not.toHaveBeenCalled();
  const temporary = { ...initial, sourceLanguage: "en" as const, profiles: [saved] };
  actions.selectProfile.mockResolvedValue({ ...initial, profiles: [saved] });
  await render(temporary);
  await click(I18N.settings.useProfile);
  expect(actions.selectProfile).toHaveBeenCalledExactlyOnceWith(ready.id);
  expect(actions.updateProfile).toHaveBeenCalledTimes(1);
});

function appleSettings(): SettingsSnapshot {
  return { ...settings, profiles: [appleProfile], activeProfileId: appleProfile.id, sourceLanguage: "en", targetLanguage: "original", languageCapabilities: { profileId: appleProfile.id, provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original", sourceLanguages: ["en", "ja"], targetLanguages: ["original"] } };
}

it("shows no network proxy or credential form for Apple recognition plus Apple translation", async () => {
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  const native = { ...appleProfile, textTranslation: "apple" as const, credentialState: "present" as const, textCredentialState: "present" as const };
  await render({ ...appleSettings(), targetLanguage: "zh", profiles: [native], languageCapabilities: { profileId: native.id, provider: native.provider, textTranslation: "apple", targetLanguage: "zh", sourceLanguages: ["en"], targetLanguages: ["original", "zh", "en"] } });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector(".service-proxies")).toBeNull();
  expect(host.querySelector('input[type="password"]')).toBeNull();
  expect(profileCredentialEditorState).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("retains only the recognition proxy when Alibaba uses local Apple translation", async () => {
  await render({ ...settings, sourceLanguage: "en", profiles: [{ ...profile, credentialState: "present", speechCredentialState: "present", textCredentialState: "present", textTranslation: "apple" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelectorAll(".service-proxies .network-proxy-form")).toHaveLength(1);
  expect(host.querySelector(".service-proxies")?.textContent).toContain(I18N.settings.speechRecognition);
  expect(host.querySelector(".service-proxies")?.textContent).not.toContain(I18N.settings.textTranslationLabel);
});

it("announces a local Apple route save without claiming credentials were stored", async () => {
  const native = { ...profile, credentialState: "missing" as const, speechCredentialState: "present" as const, textTranslation: "apple" as const };
  const snapshot = { ...settings, sourceLanguage: "en" as const, profiles: [native] };
  actions.saveProfileCredentials.mockResolvedValue(snapshot);
  actions.selectProfile.mockResolvedValue(snapshot);
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.saveAndUse);
  expect(actions.saveProfileCredentials).toHaveBeenCalledExactlyOnceWith(native.id, {
    kind: "alibabaTranslation", apiKey: "", textTranslation: "apple", endpoint: "", token: "", model: "",
  });
  expect(document.body.textContent).toContain(I18N.settings.translationConfigurationSaved);
  expect(document.body.textContent).not.toContain(I18N.settings.credentialsSaved);
});

it("offers Apple only after this Mac reports availability and does not create it on preview", async () => {
  let resolve!: (value: typeof appleSupport) => void;
  vi.mocked(getAppleSpeechSupport).mockReturnValue(new Promise(done => { resolve = done; }));
  await render();
  await click(I18N.settings.addProfile);
  expect(host.querySelector('[data-provider="appleSpeech"]')).toBeNull();
  await act(async () => resolve(appleSupport));
  expect(host.querySelector('.provider-option[data-provider="appleSpeech"] img')).not.toBeNull();
  await previewProvider("appleSpeech");
  expect(actions.createProfile).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

it("keeps Apple hidden after a failed check and exposes a sanitized retry", async () => {
  vi.mocked(getAppleSpeechSupport).mockRejectedValueOnce(new Error("private-native-detail"));
  await render();
  await click(I18N.settings.addProfile);
  expect(host.querySelector('.provider-option[data-provider="appleSpeech"]')).toBeNull();
  expect(host.textContent).toContain(I18N.settings.appleSpeechLoadFailed);
  expect(document.body.textContent).not.toContain("private-native-detail");
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  await click(I18N.settings.retryLoadingSettings);
  expect(host.querySelector('.provider-option[data-provider="appleSpeech"]')).not.toBeNull();
});

it("keeps a saved Apple profile editable but unusable on an unsupported device", async () => {
  await render({ ...settings, targetLanguage: "original", profiles: [profile, {
    ...appleProfile, credentialState: "present", speechCredentialState: "present",
  }] });
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  expect(host.querySelector(".service-detail__name input")).not.toBeNull();
  expect(host.textContent).toContain(I18N.settings.appleSpeechUnavailable);
  expect(host.querySelector(".service-detail__actions")?.textContent).not.toContain(I18N.settings.useProfile);
  expect(actions.selectProfile).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("shows local Apple resources and independent stage controls without speech credentials in %s", async language => {
  setStoredUiLanguage(language);
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  await render(appleSettings());
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector('.apple-speech-settings input[type="password"]')).toBeNull();
  expect(host.textContent).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.en);
  expect(host.textContent).not.toContain("en-US");
  expect(host.textContent).toContain(I18N.settings.appleSpeechNotInstalled);
  expect(host.textContent).toContain(I18N.settings.checkSpeechRecognition);
  expect(host.textContent).not.toContain(I18N.settings.checkTextTranslation);
  expect(host.querySelectorAll(".service-proxies .network-proxy-form").length).toBeLessThanOrEqual(1);
  expect(host.querySelector("#translation-languages")).toBeNull();
  expect(host.querySelectorAll(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)).toHaveLength(1);
  expect(host.textContent).toContain(I18N.settings.appleSpeechDownloadAndUse);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(actions.saveSettings).not.toHaveBeenCalled();
});

it("prepares only the explicitly chosen Apple language and blocks duplicate preparation", async () => {
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  let resolve!: (value: typeof appleSupport) => void;
  vi.mocked(prepareAppleSpeechLanguage).mockReturnValue(new Promise(done => { resolve = done; }));
  await render(appleSettings());
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.appleSpeechDownloadAndUse);
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("en");
  expect(host.textContent).toContain(I18N.settings.appleSpeechPreparing);
  expect(host.querySelector<HTMLButtonElement>(".service-back")?.disabled).toBe(true);
  const prepareButton = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.appleSpeechDownloadAndUse)!;
  expect(prepareButton.disabled).toBe(true);
  await act(async () => prepareButton.click());
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledOnce();
  await act(async () => resolve({ ...appleSupport, languages: appleSupport.languages.map(item => ({ ...item, installed: true, status: "installed" as const })) }));
  expect(host.textContent).toContain(I18N.settings.appleSpeechLanguageInUse);
  expect(host.querySelector<HTMLButtonElement>(".service-back")?.disabled).toBe(false);
  expect(actions.saveSettings).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "en" });
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it("retains a failed Apple preparation as retryable feedback without exposing native errors", async () => {
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  vi.mocked(prepareAppleSpeechLanguage).mockRejectedValue(new Error("private-native-path"));
  await render(appleSettings());
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.appleSpeechDownloadAndUse);
  expect(host.textContent).toContain(I18N.settings.appleSpeechPrepareFailed);
  expect(document.body.textContent).not.toContain("private-native-path");
  expect(host.querySelector<HTMLButtonElement>(".service-back")?.disabled).toBe(false);
  expect([...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.appleSpeechRetryDownload)?.disabled).toBe(false);
});

it("keeps a current Apple recognition result when its inventory broadcast changes another locale", async () => {
  const readySupport: AppleSpeechSupport = { ...appleSupport, languages: appleSupport.languages.map(item => ({ ...item, installed: true, status: "installed" })) };
  boot.nativeSettings = { ...appleSettings(), languageCapabilities: { ...appleSettings().languageCapabilities!, appleSpeechSupportRevision: 1 } };
  vi.mocked(getAppleSpeechSupport).mockResolvedValueOnce(readySupport);
  let finishCheck!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  vi.mocked(testProfileConnection).mockReturnValue(new Promise(done => { finishCheck = done; }));
  await render(boot.nativeSettings);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const check = host.querySelector<HTMLButtonElement>(".apple-speech-connection-check button.settings-button")!;
  await act(async () => check.click());
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith(appleProfile.id, "speech", undefined, "en");
  let finishRefresh!: (support: AppleSpeechSupport) => void;
  vi.mocked(getAppleSpeechSupport).mockReturnValueOnce(new Promise(done => { finishRefresh = done; }));
  boot.nativeSettings = { ...boot.nativeSettings, languageCapabilities: { ...boot.nativeSettings.languageCapabilities!, appleSpeechSupportRevision: 2 } };
  await render(boot.nativeSettings);
  expect(host.querySelector(".apple-speech-connection-check button.settings-button")).toBe(check);
  expect(check.disabled).toBe(true);
  await act(async () => finishRefresh({ ...readySupport, languages: readySupport.languages.map(item => item.sourceLanguage === "ja" ? { ...item, installed: false, status: "unknown" } : item) }));
  await act(async () => finishCheck({ credential: "present", service: "available", reason: null, elapsedMs: 540 }));
  expect(host.querySelector(".apple-speech-connection-check button.settings-button")).toBe(check);
  expect(host.querySelector(".apple-speech-connection-check")?.textContent).toContain(diagnosticCopy().available);
  expect(host.querySelector(".apple-speech-connection-check")?.textContent).toContain("540 ms");
  expect(check.disabled).toBe(false);
  expect(actions.saveSettings).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

it("downloads and applies the language chosen in the single recognition selector", async () => {
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  vi.mocked(prepareAppleSpeechLanguage).mockResolvedValue({ ...appleSupport, languages: appleSupport.languages.map(item => ({ ...item, installed: true, status: "installed" as const })) });
  await render({ ...appleSettings(), sourceLanguage: "ja" });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.textContent).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.ja);
  expect(host.textContent).not.toContain("ja-JP");
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!.click());
  await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(item => item.textContent?.startsWith(SOURCE_LANGUAGE_DISPLAY_NAMES.en))!.click());
  expect(host.textContent).toContain(SOURCE_LANGUAGE_DISPLAY_NAMES.en);
  expect(host.textContent).not.toContain("en-US");
  expect(actions.saveSettings).not.toHaveBeenCalled();
  await click(I18N.settings.appleSpeechDownloadAndUse);
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledExactlyOnceWith("en");
  expect(actions.saveSettings).toHaveBeenCalledExactlyOnceWith({ sourceLanguage: "en" });
});

it("discards late Apple support failures after leaving the service surface", async () => {
  let reject!: (error: Error) => void;
  vi.mocked(getAppleSpeechSupport).mockReturnValue(new Promise((_, fail) => { reject = fail; }));
  await render();
  await act(async () => root.render(<><ServiceProfiles settings={settings} sessionIsActive={false} visible={false} /><SettingsToastRegion /></>));
  await act(async () => reject(new Error("late-native-error")));
  expect(host.textContent).not.toContain(I18N.settings.appleSpeechLoadFailed);
  expect(host.querySelector(".settings-toast")).toBeNull();
});
it("shows a custom speech profile as ready for Original even when its independent translation key is missing", async () => {
  const custom: ServiceProfile = { ...profile, provider: "customDashScopeASR", credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing", textTranslation: "deepL" };
  await render({ ...settings, targetLanguage: "original", profiles: [custom] });
  expect(host.querySelector(".credential-badge")?.getAttribute("aria-label")).toBe(I18N.settings.credentialPresent);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector(".service-detail__title .credential-badge")?.getAttribute("aria-label")).toBe(I18N.settings.credentialPresent);
  expect(host.querySelector(".service-stage--translation")?.textContent).toContain("DeepL");
  expect(host.querySelector('input[id$="-speech-key"]')).toBeNull();
});
it("places a full back button beside the profile title and keeps global input controls in the overview", async () => {
  await act(async () => root.render(<ServiceProfiles settings={settings} sessionIsActive={false} overview={<div data-testid="audio-overview">Audio input</div>} />));
  expect(host.querySelector('[data-testid="audio-overview"]')).not.toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector('[data-testid="audio-overview"]')).toBeNull();
  const back = host.querySelector<HTMLButtonElement>(".service-detail__header .service-back")!;
  expect(back.classList.contains("settings-button")).toBe(true);
  expect(back.textContent).toBe(I18N.settings.backToServices);
  await act(async () => back.click());
  expect(host.querySelector(".service-detail")).toBeNull();
  expect(host.querySelector('[data-testid="audio-overview"]')).not.toBeNull();
});
it("keeps local-file translation configuration editable", async () => {
  const regular: ServiceProfile = { ...profile, id: "regular", credentialStorage: "localFile", credentialState: "present" };
  const snapshot: SettingsSnapshot = { ...settings, credentialStorage: "localFile", activeProfileId: regular.id, profiles: [regular] };
  const saved = { ...snapshot, activeProfileId: regular.id, profiles: [{ ...regular, textTranslation: "chatMock" as const }] };
  actions.saveProfileCredentials.mockResolvedValue(saved);
  actions.selectProfile.mockResolvedValue(saved);
  await render(snapshot);
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[0]!.click());
  const picker = host.querySelector<HTMLButtonElement>('.service-stage--translation [role="combobox"]')!;
  expect(picker.disabled).toBe(false);
  expect(host.querySelector('.service-stage__restriction')).toBeNull();
  expect(host.querySelector<HTMLInputElement>(`#profile-name-${regular.id}`)?.readOnly).toBe(false);
  await act(async () => picker.click());
  await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === "ChatMock")!.click());
  await change('.service-stage--translation input[id$="-model"]', "synthetic-chat-model");
  const save = host.querySelector<HTMLButtonElement>('.service-stages button[type="submit"]')!;
  expect(save.disabled).toBe(false);
  await act(async () => save.click());
  expect(actions.saveProfileCredentials).toHaveBeenCalledExactlyOnceWith(regular.id, { kind: "alibabaTranslation", apiKey: "", textTranslation: "chatMock", endpoint: "http://127.0.0.1:8000/v1", token: "", model: "synthetic-chat-model" });
  expect(actions.selectProfile).toHaveBeenCalledExactlyOnceWith(regular.id);
});
async function click(label: string) {
  const surface = document.querySelector('[role="alertdialog"], [role="dialog"]') ?? host;
  const button = [...surface.querySelectorAll("button")].find(node => node.textContent === label)
    ?? (label === diagnosticCopy().test ? host.querySelector<HTMLButtonElement>(".connection-check button") : null);
  if (!button) throw new Error(`Button not found: ${label}`);
  await act(async () => button.click());
}
async function previewProvider(provider: ServiceProfile["provider"]) {
  const button = host.querySelector<HTMLButtonElement>(`.provider-option[data-provider="${provider}"]`)!;
  expect(button).toBeTruthy();
  await act(async () => button.click());
}
async function change(selector: string, value: string) {
  const node = host.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(node instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(node, value);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function chooseCustomTranslation() {
  await act(async () => host.querySelector<HTMLButtonElement>('.service-stage--translation [role="combobox"]')!.click());
  const custom = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === I18N.settings.textTranslationCustom)!;
  await act(async () => custom.click());
}
async function submit() {
  await act(async () => host.querySelector(".credential-form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
}

it.each(["zh", "en", "ja"] as const)("keeps credential states as accessible icons with keyboard tooltips in %s", async language => {
  setStoredUiLanguage(language);
  for (const [credentialState, label] of [["present", I18N.settings.credentialPresent], ["missing", I18N.settings.credentialMissing], ["unavailable", I18N.settings.credentialUnavailable]] as const) {
    await render({ ...settings, profiles: [{ ...profile, credentialState }] });
    const badge = host.querySelector<HTMLElement>(".credential-badge")!;
    expect(badge.textContent).toBe("");
    expect(badge.getAttribute("role")).toBe("img");
    expect(badge.getAttribute("aria-label")).toBe(label);
    expect(badge.tabIndex).toBe(0);
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    const matches = badge.matches.bind(badge);
    const focusVisible = vi.spyOn(badge, "matches").mockImplementation(selector => selector === ":focus-visible" || matches(selector));
    await act(async () => badge.focus());
    const popup = document.querySelector<HTMLElement>('[role="tooltip"]')!;
    expect(popup.textContent).toBe(label);
    expect(badge.getAttribute("aria-describedby")).toBe(popup.id);
    await act(async () => badge.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    expect(document.activeElement).toBe(badge);
    await act(async () => badge.blur());
    focusVisible.mockRestore();
  }
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(actions.updateProfile).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it("shows loading or a retryable initialization timeout without claiming credentials are unavailable", async () => {
  const empty = { ...settings, profiles: [], activeProfileId: "" };
  boot.initializationStatus = "loading";
  await render(empty);
  expect(host.textContent).toContain(I18N.settings.settingsSnapshotLoading);
  expect(host.textContent).not.toContain(I18N.settings.credentialUnavailable);
  expect(host.textContent).not.toContain(I18N.settings.firstProfileTitle);
  expect(host.querySelector(".service-rows")).toBeNull();
  boot.initializationStatus = "error"; boot.initializationError = "timeout";
  await render(empty);
  expect(host.textContent).toContain(I18N.settings.settingsSnapshotTimeout);
  expect(host.textContent).not.toContain(I18N.settings.firstProfileTitle);
  await click(I18N.settings.retryLoadingSettings);
  expect(boot.init).toHaveBeenCalledOnce();
  expect(testProfileConnection).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("previews a provider and leaves settings unchanged when cancelled in %s", async language => {
  setStoredUiLanguage(language);
  await render();
  await click(I18N.settings.addProfile);
  const options = [...host.querySelectorAll<HTMLButtonElement>(".provider-option")];
  expect(options.map(option => option.dataset.provider)).toEqual(SERVICE_PROVIDERS.filter(provider => provider !== "appleSpeech"));
  expect(options.slice(-2).map(option => option.dataset.provider)).toEqual(["customDashScopeASR", "customOpenAIASR"]);
  expect(host.querySelector(".provider-picker small, .provider-picker p")).toBeNull();
  expect(host.querySelector(".provider-picker__heading .settings-help-control__description")?.textContent).toBe(I18N.settings.chooseProviderDescription);
  await previewProvider("customOpenAIASR");
  expect(document.querySelector(".provider-picker__preview h3")?.textContent).toBe(providerDisplayName("customOpenAIASR"));
  expect(document.querySelector(".provider-picker__preview .settings-help-control__description")?.textContent).toBe(`${I18N.settings.customSpeechRequirementsOpenAI}\n${I18N.settings.customSpeechLanguages}`);
  expect(actions.createProfile).not.toHaveBeenCalled();
  await click(I18N.settings.cancel);
  expect(document.querySelector(".provider-picker__preview")).toBeNull();
  expect(host.querySelectorAll(".provider-option")).toHaveLength(SERVICE_PROVIDERS.length - 1);
  await previewProvider("alibabaCloud");
  await click(I18N.settings.cancel);
  expect(document.querySelector('[role="alertdialog"], [role="dialog"]')).toBeNull();
  expect(actions.createProfile).not.toHaveBeenCalled();
  await click(I18N.settings.cancel);
  expect(host.querySelector(".provider-picker")).toBeNull();
  expect(host.querySelectorAll(".service-row")).toHaveLength(1);
  expect(actions.createProfile).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it.each([
  ["active", true, false, "listening"],
  ["paused", false, true, "listening"],
  ["connecting", false, false, "connecting"],
  ["stopping", false, false, "stopping"],
] as const)("keeps provider-picker cancellation available when the session becomes %s", async (_state, active, paused, status) => {
  await render();
  await click(I18N.settings.addProfile);
  await act(async () => root.render(<><ServiceProfiles settings={settings} sessionIsActive={active} sessionIsPaused={paused} sessionStatusKind={status} /><SettingsToastRegion /></>));
  const picker = host.querySelector(".provider-picker")!;
  for (const button of picker.querySelectorAll<HTMLButtonElement>(".provider-option")) {
    expect(button.disabled).toBe(true);
    await act(async () => button.click());
  }
  const hint = picker.querySelector('.settings-feedback[data-tone="info"]')!;
  expect(hint.textContent).toBe(I18N.settings.profileCreateRequiresStop);
  expect(hint.closest('[role="tooltip"], [aria-hidden="true"]')).toBeNull();
  const cancel = picker.querySelector<HTMLButtonElement>(".provider-picker__heading button.settings-link")!;
  expect(cancel.disabled).toBe(false);
  await act(async () => cancel.click());
  expect(host.querySelector(".provider-picker")).toBeNull();
  expect(host.querySelectorAll(".service-row")).toHaveLength(1);
  expect(host.querySelector<HTMLButtonElement>(".services-toolbar button.settings-button")?.disabled).toBe(true);
  expect(actions.createProfile).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
});

it("allows cancelling a provider preview after the session starts while keeping creation blocked", async () => {
  await render();
  await click(I18N.settings.addProfile);
  await previewProvider("alibabaCloud");
  await act(async () => root.render(<><ServiceProfiles settings={settings} sessionIsActive sessionStatusKind="listening" /><SettingsToastRegion /></>));
  const dialog = document.querySelector('[role="dialog"]')!;
  expect(dialog.querySelector('.settings-feedback[data-tone="info"]')?.textContent).toBe(I18N.settings.profileCreateRequiresStop);
  const confirm = dialog.querySelector<HTMLButtonElement>(".settings-confirmation__confirm")!;
  expect(confirm.disabled).toBe(true);
  await act(async () => confirm.click());
  expect(actions.createProfile).not.toHaveBeenCalled();
  await click(I18N.settings.cancel);
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(host.querySelector(".provider-picker")).not.toBeNull();
  await click(I18N.settings.cancel);
  expect(host.querySelector(".provider-picker")).toBeNull();
});

it("keeps provider-picker and preview cancellation blocked only while creation is pending", async () => {
  let reject!: (error: unknown) => void;
  actions.createProfile.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  await render();
  await click(I18N.settings.addProfile);
  await previewProvider("alibabaCloud");
  await click(I18N.settings.confirmAddProfile);
  expect(host.querySelector<HTMLButtonElement>(".provider-picker__heading button.settings-link")?.disabled).toBe(true);
  expect(document.querySelector('.settings-feedback[data-tone="info"]')).toBeNull();
  await click(I18N.settings.cancel);
  expect(document.querySelector(".provider-picker__preview")).not.toBeNull();
  await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(document.querySelector(".provider-picker__preview")).not.toBeNull();
  expect(actions.createProfile).toHaveBeenCalledOnce();
  await act(async () => reject("synthetic-failed-create"));
  await click(I18N.settings.cancel);
  expect(document.querySelector(".provider-picker__preview")).toBeNull();
  expect(host.querySelector<HTMLButtonElement>(".provider-picker__heading button.settings-link")?.disabled).toBe(false);
  await click(I18N.settings.cancel);
  expect(host.querySelector(".provider-picker")).toBeNull();
});

it("creates the selected provider only after confirmation and opens the created profile", async () => {
  const provider = "customDashScopeASR" as const;
  const created: ServiceProfile = { id: "created-synthetic", provider, name: providerDisplayName(provider), credentialState: "missing" };
  const next = { ...settings, profiles: [...settings.profiles, created] };
  actions.createProfile.mockResolvedValue(next);
  await render(); await click(I18N.settings.addProfile); await previewProvider(provider);
  expect(actions.createProfile).not.toHaveBeenCalled();
  await click(I18N.settings.confirmAddProfile);
  expect(actions.createProfile).toHaveBeenCalledExactlyOnceWith(provider, providerDisplayName(provider));
  expect(host.querySelector(".provider-picker")).toBeNull();
  await render(next);
  expect(host.querySelector(".service-detail__identity h2")?.textContent).toBe(created.name);
  expect(actions.selectProfile).not.toHaveBeenCalled();
});

it("starts with no service and creates the first chosen provider only after confirmation", async () => {
  const empty = { ...settings, profiles: [], activeProfileId: "" };
  const created: ServiceProfile = { id: "first", provider: "googleGeminiLive", name: providerDisplayName("googleGeminiLive"), credentialState: "missing" };
  const next = { ...empty, profiles: [created], activeProfileId: created.id };
  actions.createProfile.mockResolvedValue(next);
  await render(empty);
  expect(host.querySelector(".service-row")).toBeNull();
  expect(host.textContent).toContain(I18N.settings.firstProfileTitle);
  expect(host.textContent).toContain(I18N.settings.firstProfileHint);
  await click(I18N.settings.addProfile);
  await previewProvider(created.provider);
  expect(actions.createProfile).not.toHaveBeenCalled();
  await click(I18N.settings.cancel);
  expect(host.querySelector(".service-row")).toBeNull();
  await previewProvider(created.provider);
  await click(I18N.settings.confirmAddProfile);
  await render(next);
  expect(actions.createProfile).toHaveBeenCalledExactlyOnceWith(created.provider, created.name);
  expect(host.querySelector(".service-detail__identity h2")?.textContent).toBe(created.name);
  expect(actions.selectProfile).not.toHaveBeenCalled();
  await click(I18N.settings.backToServices);
  expect(host.textContent).not.toContain(I18N.settings.firstProfileTitle);
  expect(host.textContent).toContain(I18N.settings.profileCount(1));
});

it("guards duplicate confirmation synchronously and keeps a failed provider preview available for retry", async () => {
  let reject!: (error: unknown) => void;
  actions.createProfile.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  await render(); await click(I18N.settings.addProfile); await previewProvider("customOpenAIASR");
  const confirmation = [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.confirmAddProfile)!;
  await act(async () => { confirmation.click(); confirmation.click(); });
  expect(actions.createProfile).toHaveBeenCalledExactlyOnceWith("customOpenAIASR", providerDisplayName("customOpenAIASR"));
  expect(confirmation.disabled).toBe(true);
  await act(async () => { reject("synthetic-private-provider-failure"); });
  expect(document.querySelector(".provider-picker__preview h3")?.textContent).toBe(providerDisplayName("customOpenAIASR"));
  const dialog = document.querySelector('[role="alertdialog"], [role="dialog"]')!;
  expect(dialog.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(I18N.settings.profileActionFailed);
  expect(dialog.querySelector(".settings-feedback")).toBeNull();
  expect(dialog.textContent).not.toContain("synthetic-private-provider-failure");
  expect(confirmation.disabled).toBe(false);
  const created: ServiceProfile = { id: "retry-created", provider: "customOpenAIASR", name: providerDisplayName("customOpenAIASR"), credentialState: "missing" };
  actions.createProfile.mockResolvedValueOnce({ ...settings, profiles: [...settings.profiles, created] });
  await click(I18N.settings.confirmAddProfile);
  expect(actions.createProfile).toHaveBeenCalledTimes(2);
  expect(actions.createProfile).toHaveBeenLastCalledWith("customOpenAIASR", providerDisplayName("customOpenAIASR"));
  expect(host.querySelector(".provider-picker")).toBeNull();
});

it.each(["alibabaCloud", "customDashScopeASR", "customOpenAIASR"] as const)("checks %s recognition and translation separately and retains each stage's own outcome", async provider => {
  const configured: ServiceProfile = { ...profile, provider, credentialState: "present", speechCredentialState: "present", textCredentialState: "present", textTranslation: "deepLX" };
  vi.mocked(testProfileConnection).mockResolvedValueOnce({ credential: "present", service: "available", reason: null, elapsedMs: 123 })
    .mockResolvedValueOnce({ credential: "present", service: "unavailable", reason: "authenticationRejected", elapsedMs: 456 });
  await render({ ...settings, profiles: [configured] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const checks = [...host.querySelectorAll<HTMLDivElement>(".connection-check")];
  expect(checks).toHaveLength(2);
  await click(I18N.settings.checkSpeechRecognition);
  expect(testProfileConnection).toHaveBeenNthCalledWith(1, profile.id, "speech");
  expect(checks[0].querySelector('[data-tone="success"]')?.textContent).toContain(diagnosticCopy().available);
  expect(checks[1].querySelector(".settings-feedback")).toBeNull();
  await click(I18N.settings.checkTextTranslation);
  expect(testProfileConnection).toHaveBeenNthCalledWith(2, profile.id, "text");
  expect(checks[0].querySelector('[data-tone="success"]')?.textContent).toContain(diagnosticCopy().available);
  expect(checks[0].querySelector('[data-tone="error"]')).toBeNull();
  expect(checks[1].querySelector('[data-tone="error"]')?.textContent).toContain(diagnosticCopy().reasons.authenticationRejected);
  expect(checks[1].querySelector('[data-tone="success"]')).toBeNull();
});

it("does not publish a late translation check into a subsequent recognition check", async () => {
  vi.useFakeTimers();
  let finishText!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  let finishSpeech!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  vi.mocked(testProfileConnection).mockImplementationOnce(() => new Promise(resolve => { finishText = resolve; }))
    .mockImplementationOnce(() => new Promise(resolve => { finishSpeech = resolve; }));
  await render({ ...settings, profiles: [{ ...profile, credentialState: "present", textTranslation: "deepLX" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.checkTextTranslation);
  await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
  const checks = [...host.querySelectorAll<HTMLDivElement>(".connection-check")];
  expect(checks[1].querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.profileCheckTimedOut);
  await click(I18N.settings.checkSpeechRecognition);
  await act(async () => { finishText({ credential: "present", service: "available", reason: null, elapsedMs: 999 }); });
  expect(checks[0].querySelector(".settings-feedback")).toBeNull();
  expect(checks[0].querySelector("button")?.getAttribute("aria-busy")).toBe("true");
  expect(checks[1].querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.profileCheckTimedOut);
  await act(async () => { finishSpeech({ credential: "present", service: "available", reason: null, elapsedMs: 100 }); });
  expect(checks[0].querySelector('[data-tone="success"]')?.textContent).toContain(diagnosticCopy().available);
  expect(checks[1].querySelector('[data-tone="success"]')).toBeNull();
  expect(testProfileConnection).toHaveBeenNthCalledWith(1, profile.id, "text");
  expect(testProfileConnection).toHaveBeenNthCalledWith(2, profile.id, "speech");
});

it("waits for required draft fields and checks them without saving while leaving recognition available", async () => {
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await render({ ...settings, profiles: [{ ...profile, credentialState: "present", textTranslation: "deepLX" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.checkTextTranslation);
  const textCheck = host.querySelector<HTMLDivElement>(".service-stage--translation .connection-check")!;
  expect(textCheck.querySelector('[data-tone="success"]')).not.toBeNull();
  const selector = host.querySelector<HTMLButtonElement>('.service-stage--translation [role="combobox"]')!;
  await act(async () => selector.click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === "DeepL")!;
  await act(async () => option.click());
  expect(textCheck.querySelector<HTMLButtonElement>("button.settings-button")!.disabled).toBe(true);
  expect(textCheck.querySelector(".settings-help-control__description")).toBeNull();
  expect(textCheck.querySelector(".settings-feedback")).toBeNull();
  await act(async () => textCheck.querySelector<HTMLButtonElement>("button.settings-button")!.click());
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith(profile.id, "text");
  await click(I18N.settings.checkSpeechRecognition);
  expect(testProfileConnection).toHaveBeenLastCalledWith(profile.id, "speech");
  expect(testProfileConnection).toHaveBeenCalledTimes(2);
  await change('input[id$="-token"]', "synthetic-draft-key");
  expect(textCheck.querySelector<HTMLButtonElement>("button.settings-button")!.disabled).toBe(false);
  await click(I18N.settings.checkTextTranslation);
  expect(testProfileConnection).toHaveBeenLastCalledWith(profile.id, "text", {
    kind: "alibabaTranslation", apiKey: "", textTranslation: "deepL", endpoint: "", token: "synthetic-draft-key", model: "",
  });
  expect(textCheck.querySelector('[data-tone="success"]')).not.toBeNull();
  await change('input[id$="-token"]', "synthetic-new-key");
  expect(textCheck.querySelector(".settings-feedback")).toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it("releases a hung connection check after 30 seconds and ignores its late result during a manual retry", async () => {
  vi.useFakeTimers();
  let finishFirst!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  let finishRetry!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  vi.mocked(testProfileConnection).mockImplementationOnce(() => new Promise((resolve) => { finishFirst = resolve; }))
    .mockImplementationOnce(() => new Promise((resolve) => { finishRetry = resolve; }));
  await render();
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(diagnosticCopy().test);
  await act(async () => { await vi.advanceTimersByTimeAsync(29_999); });
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(true);
  await act(async () => { await vi.advanceTimersByTimeAsync(1); });
  expect(host.querySelector(".connection-check .settings-feedback")?.textContent).toBe(I18N.settings.profileCheckTimedOut);
  expect(host.querySelector(".connection-check")?.textContent).not.toContain(diagnosticCopy().unavailable);
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(false);
  await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
  expect(testProfileConnection).toHaveBeenCalledOnce();
  await click(diagnosticCopy().test);
  await act(async () => { finishFirst({ credential: "present", service: "available", reason: null }); });
  expect(host.querySelector(".connection-check .settings-feedback")).toBeNull();
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(true);
  await act(async () => { finishRetry({ credential: "present", service: "available", reason: null }); });
  expect(host.querySelector(".connection-check .settings-feedback")?.textContent).toBe(diagnosticCopy().available);
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(false);
  expect(testProfileConnection).toHaveBeenCalledTimes(2);
});

it("guards repeated clicks synchronously and clears the deadline when the check finishes", async () => {
  vi.useFakeTimers();
  let finish!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  vi.mocked(testProfileConnection).mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  await render(); await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const button = host.querySelector<HTMLButtonElement>(".connection-check button")!;
  await act(async () => { button.click(); button.click(); });
  expect(testProfileConnection).toHaveBeenCalledOnce();
  await act(async () => { finish({ credential: "present", service: "available", reason: null }); });
  await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
  expect(host.querySelector(".connection-check .settings-feedback")?.textContent).toBe(diagnosticCopy().available);
  expect(testProfileConnection).toHaveBeenCalledOnce();
});

it("clears an old-route connection result without replacing an unsaved credential or name draft", async () => {
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  const snapshot = { ...settings, profiles: [{ ...profile, credentialState: "present" as const }] };
  await render(snapshot); await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(diagnosticCopy().test);
  expect(host.querySelector(".connection-check .settings-feedback")?.textContent).toBe(diagnosticCopy().available);
  await click(I18N.settings.replaceCredentials);
  await change('.credential-panel input[type="password"]', "synthetic-unsaved-key");
  await change(".service-detail__name input", "Unsaved name");
  await render({ ...snapshot, networkProxy: { mode: "direct", url: null } });
  expect(host.querySelector(".connection-check .settings-feedback")).toBeNull();
  expect(host.querySelector<HTMLInputElement>('.credential-panel input[type="password"]')!.value).toBe("synthetic-unsaved-key");
  expect(host.querySelector<HTMLInputElement>(".service-detail__name input")!.value).toBe("Unsaved name");
  expect(testProfileConnection).toHaveBeenCalledOnce();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it("rejects a late check from the old proxy and only checks the new route after a manual retry", async () => {
  let finish!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  vi.mocked(testProfileConnection).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }))
    .mockResolvedValueOnce({ credential: "present", service: "available", reason: null });
  await render(); await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(diagnosticCopy().test);
  await render({ ...settings, networkProxy: { mode: "direct", url: null } });
  await act(async () => { finish({ credential: "present", service: "unavailable", reason: "unreachable" }); });
  expect(host.querySelector(".connection-check .settings-feedback")).toBeNull();
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(false);
  expect(testProfileConnection).toHaveBeenCalledOnce();
  await click(diagnosticCopy().test);
  expect(host.querySelector(".connection-check .settings-feedback")?.textContent).toBe(diagnosticCopy().available);
  expect(testProfileConnection).toHaveBeenCalledTimes(2);
});

it("releases a timed-out old-route check without claiming the new proxy timed out", async () => {
  vi.useFakeTimers(); vi.mocked(testProfileConnection).mockImplementation(() => new Promise(() => {}));
  await render(); await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(diagnosticCopy().test); await render({ ...settings, networkProxy: { mode: "direct", url: null } });
  await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
  expect(host.querySelector(".connection-check .settings-feedback")).toBeNull();
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(false);
  expect(testProfileConnection).toHaveBeenCalledOnce();
});

it.each(["zh", "en", "ja"] as const)("groups the service identity, credential state and existing actions without hiding them in %s", async (language) => {
  setStoredUiLanguage(language);
  await render({ ...settings, profiles: [{ ...profile, provider: "openAIRealtime", credentialState: "present" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const identity = host.querySelector(".service-detail__identity")!;
  expect(identity.querySelector('.provider-icon[data-provider="openAIRealtime"]')).not.toBeNull();
  expect(identity.querySelector("h2")?.textContent).toBe(profile.name);
  expect(identity.querySelector(".service-detail__title .credential-badge")?.getAttribute("aria-label")).toBe(I18N.settings.credentialPresent);
  expect(identity.querySelector(".service-detail__title .profile-active-badge")?.textContent).toBe(I18N.settings.activeProfile);
  expect(identity.querySelector(".service-detail__name-help .settings-help-control__description")?.textContent).toContain(I18N.settings.providerOpenAIDescription);
  expect(host.querySelector(".credential-storage-help > span")?.textContent).toBe(I18N.settings.credentials);
  expect(identity.querySelector(".service-detail__copy > p")).toBeNull();
  expect(identity.querySelector(".service-language-support")).toBeNull();
  expect(host.querySelector(".service-detail__configuration #translation-languages")).not.toBeNull();
  const connection = host.querySelector(".service-detail__connection")!;
  expect(connection.querySelector(".connection-check")).not.toBeNull();
  expect(connection.querySelector(".credential-panel__saved-actions")?.textContent).toContain(I18N.settings.replaceCredentials);
  await click(I18N.settings.replaceCredentials);
  expect(connection.querySelector('input[type="password"]')).not.toBeNull();
  expect(connection.querySelector(".settings-advanced, details")).toBeNull();
  const integratedStage = connection.querySelector(".service-stage--integrated")!;
  expect(integratedStage.querySelector("h3")?.textContent).toBe(I18N.settings.voiceTranslation);
  expect(integratedStage.querySelector('.provider-icon[data-provider="openAIRealtime"]')).not.toBeNull();
  expect(integratedStage.querySelector(".settings-help-control__description")?.textContent).toBe(I18N.settings.textTranslationUnsupported);
  expect(integratedStage.querySelector('[role="combobox"], p')).toBeNull();
  expect(host.querySelector(".service-detail__name label")?.textContent).toBe(I18N.settings.profileName);
  expect(host.querySelector(".service-detail__name")?.closest("details")).toBeNull();
  expect(host.querySelector(".service-detail__save-name")).toBeNull();
  expect(host.querySelector(".service-detail__configuration .credential-panel")).not.toBeNull();
  expect(host.querySelector(".service-detail__actions")?.textContent).toContain(I18N.settings.deleteProfile);
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.deleteProfileAPIKey).not.toHaveBeenCalled();
  expect(testProfileConnection).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("shows a default provider name once and keeps custom names distinct in %s", async (language) => {
  setStoredUiLanguage(language);
  const providerName = providerDisplayName("alibabaCloud");
  await render({ ...settings, profiles: [{ ...profile, name: providerName }, { ...profile, id: "custom", name: "Custom configuration" }] });
  const rows = [...host.querySelectorAll(".service-row")];
  expect(rows[0].querySelector(".service-row__copy")?.textContent).toBe(providerName);
  expect(rows[0].querySelector(".service-row__provider")).toBeNull();
  expect(rows[1].querySelector(".service-row__copy strong")?.textContent).toBe("Custom configuration");
  expect(rows[1].querySelector(".service-row__provider")?.textContent).toBe(providerName);
  expect(rows.some(row => !!row.querySelector(".service-row__copy small"))).toBe(false);
  expect(rows.every((row) => !!row.querySelector(".service-row__main") && !!row.querySelector(".service-row__edit"))).toBe(true);
});

it("autosaves a name without a save button, stealing focus or touching a credential draft", async () => {
  vi.useFakeTimers();
  const snapshot = { ...settings, profiles: [{ ...profile, credentialState: "present" as const }] };
  actions.updateProfile.mockResolvedValue(snapshot);
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  await change('input[type="password"]', "synthetic-replacement");
  const name = host.querySelector<HTMLInputElement>(".service-detail__name input")!;
  name.focus();
  await change(".service-detail__name input", "Other name · 空格 & (2)");
  expect(host.querySelector(".service-detail__save-name")).toBeNull();
  await act(async () => vi.advanceTimersByTimeAsync(400));
  expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith(profile.id, "Other name · 空格 & (2)");
  expect(document.activeElement).toBe(name);
  expect(name.disabled).toBe(false);
  expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("synthetic-replacement");
  expect(host.querySelector('.settings-toast[role="status"]')).toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it.each(["openAIRealtime", "volcanoEngine", "tencentCloud", "baiduTranslate"] as const)("makes supported %s languages directly selectable in the active service", async (provider) => {
  const snapshot = { ...settings, profiles: [{ ...profile, provider }] };
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  for (const [label, names] of [
    [I18N.settings.sourceLanguage, sourceLanguagesForSettings(snapshot).map(speechLanguageGuidance(snapshot).optionLabel)],
    [I18N.settings.translateTo, targetLanguagesForSettings(snapshot).map(language => targetLanguageOptionLabel(snapshot, language))],
  ] as const) {
    const picker = host.querySelector<HTMLButtonElement>(`#translation-languages [role="combobox"][aria-label="${label}"]`)!;
    if (names.length === 1) {
      expect(picker.textContent).toBe(names[0]);
      expect(picker.disabled).toBe(true);
    } else {
      await act(async () => picker.click());
      expect([...document.querySelectorAll('[role="option"]')].map(option => option.textContent)).toEqual(names);
      await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    }
  }
  expect(host.querySelector(".service-language-more")).toBeNull();
  expect(testProfileConnection).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it.each(["deepL", "deepLX", "openAICompatible", "chatMock"] as const)("keeps %s choices scoped to the actual translation route", async (textTranslation) => {
  const snapshot = { ...settings, profiles: [{ ...profile, textTranslation }] };
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const targets = targetLanguagesForSettings(snapshot).map(language => targetLanguageOptionLabel(snapshot, language));
  expect(host.querySelectorAll("#translation-languages [role=combobox]")).toHaveLength(2);
  await act(async () => host.querySelector<HTMLButtonElement>(`#translation-languages [role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!.click());
  expect([...document.querySelectorAll('[role="option"]')].map(option => option.textContent)).toEqual(targets);
  expect(targets).toContain(TARGET_LANGUAGE_DISPLAY_NAMES.fr);
  expect(testProfileConnection).not.toHaveBeenCalled();
});

it("does not edit the active service's global languages from another profile's detail", async () => {
  await render({ ...settings, profiles: [profile, { ...profile, id: "other", name: "Other", credentialState: "present" }] });
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  expect(host.querySelector("#translation-languages")).toBeNull();
  expect(host.querySelector(".service-detail__language-note")).toBeNull();
  expect(host.querySelector(".service-detail__actions .settings-help-control__description")?.textContent).toBe(I18N.settings.profileSwitchHelp);
  expect(host.querySelector(".service-detail__actions")?.textContent).toContain(I18N.settings.useProfile);
  expect(actions.saveSettings).not.toHaveBeenCalled();
});

it("keeps profile rename and delete actions reachable without opening another panel", async () => {
  const other = { ...profile, id: "other", name: "Other", credentialState: "present" as const };
  const snapshot = { ...settings, profiles: [{ ...profile, credentialState: "present" as const }, other] };
  actions.updateProfile.mockResolvedValue(snapshot);
  await render(snapshot);
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  await change(".service-detail__name input", "Renamed");
  await act(async () => { host.querySelector<HTMLInputElement>(".service-detail__name input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); });
  expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith("other", "Renamed");
  const management = host.querySelector(".service-detail__actions")!;
  expect(management.textContent).toContain(I18N.settings.useProfile);
  expect(management.textContent).toContain(I18N.settings.deleteProfile);
  const deleteButton = [...management.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.deleteProfile)!;
  expect(deleteButton.classList.contains("settings-button--danger")).toBe(true);
  await click(I18N.settings.deleteProfile);
  expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain(I18N.settings.deleteProfileConfirm("Other"));
  expect(document.querySelector('[role="alertdialog"] small')).toBeNull();
  expect(actions.deleteProfile).not.toHaveBeenCalled();
  await click(I18N.settings.cancel);
  expect(document.querySelector('[role="alertdialog"]')).toBeNull();
  expect(actions.deleteProfile).not.toHaveBeenCalled();
  actions.deleteProfile.mockResolvedValue({ ...snapshot, profiles: [snapshot.profiles[0]] });
  await click(I18N.settings.deleteProfile);
  await click(I18N.settings.confirmDelete);
  expect(actions.deleteProfile).toHaveBeenCalledExactlyOnceWith("other");
});

it("reveals an existing key in its original editable input without treating viewing as a replacement", async () => {
  const configured = { ...settings, profiles: [{ ...profile, provider: "openAIRealtime" as const, credentialState: "present" as const }] };
  await render(configured);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-stored-key");
  await click(I18N.settings.revealSavedCredential);
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: profile.id, field: "apiKey" });
  expect(host.querySelector<HTMLInputElement>('.credential-form input')!.value).toBe("synthetic-stored-key");
  expect(host.querySelector<HTMLInputElement>('.credential-form input')!.readOnly).toBe(false);
  expect(host.querySelectorAll(".credential-form input")).toHaveLength(1);
  expect(host.querySelector('.credential-form input[type="password"]')).toBeNull();
  expect(host.querySelector<HTMLButtonElement>('.credential-form button[type="submit"]')!.disabled).toBe(true);
  await click(I18N.settings.cancel);
  expect(host.querySelector('.credential-form input[type="text"]')).toBeNull();
  await click(I18N.settings.replaceCredentials);
  expect(host.querySelector('.credential-form input[type="text"]')).toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it.each(["credential_service_unavailable", "credential_store_access_denied", "credential_store_unavailable"])("retains unsaved Alibaba edits after %s and subsequent connection checks", async (error) => {
  actions.saveProfileCredentials.mockRejectedValue(error);
  await render();
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change('input[type="password"]', "synthetic-asr");
  await chooseCustomTranslation();
  await change('.service-stage--translation input[id$="-endpoint"]', "https://example.com/translate");
  await change('input[id$="-token"]', "synthetic-token");
  await submit();
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(host.querySelector('.settings-feedback[data-tone="error"]')?.textContent).toBe(profileErrorMessage(error));

  for (const [credential, reason] of [["serviceUnavailable", "credentialsServiceUnavailable"], ["accessDenied", "credentialsAccessDenied"], ["missing", "credentialsMissing"]] as const) {
    vi.mocked(testProfileConnection).mockResolvedValue({ credential, service: "unavailable", reason });
    await click(diagnosticCopy("linux").test);
    expect(testProfileConnection).toHaveBeenLastCalledWith(profile.id, "speech", {
      kind: "alibabaTranslation", apiKey: "synthetic-asr", textTranslation: "followService", endpoint: "", token: "", model: "",
    });
    expect(host.querySelector(".connection-check .settings-feedback")?.textContent).toBe(`${diagnosticCopy("linux").unavailable}: ${diagnosticCopy("linux").reasons[reason]}`);
    expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("synthetic-asr");
    expect(host.querySelector<HTMLInputElement>('input[id$="-token"]')!.value).toBe("synthetic-token");
    expect(host.querySelector<HTMLInputElement>('.service-stage--translation input[id$="-endpoint"]')!.value).toBe("https://example.com/translate");
    expect(host.querySelector('[role="combobox"]')!.textContent).toBe(I18N.settings.textTranslationCustom);
    expect(host.querySelector("select")).toBeNull();
    expect(host.querySelector<HTMLButtonElement>('.credential-form button[type="submit"]')!.disabled).toBe(false);
  }

  // A recoverable status refresh must also keep the existing write-only draft.
  await render({ ...settings, profiles: [{ ...profile, credentialState: "missing" }] });
  await submit();
  expect(actions.saveProfileCredentials).toHaveBeenLastCalledWith(profile.id, {
    kind: "alibabaTranslation", model: "", apiKey: "synthetic-asr", textTranslation: "deepLX",
    endpoint: "https://example.com/translate", token: "synthetic-token",
  });
  expect(actions.saveProfileCredentials).toHaveBeenCalledTimes(2);
  expect(host.textContent).not.toContain("synthetic-asr");
  expect(host.textContent).not.toContain("synthetic-token");
});

it("shows the same platform-aware storage guidance for other service profiles", async () => {
  await render({ ...settings, profiles: [{ ...profile, provider: "openAIRealtime" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector('.credential-unavailable[role="status"]')?.textContent).toBe(diagnosticCopy("linux").storage);
  expect(host.querySelector(".credential-badge")?.getAttribute("aria-label")).toBe(I18N.settings.credentialUnavailable);
});

it("replaces generic storage guidance with the failed save error", async () => {
  await render({ ...settings, profiles: [{ ...profile, provider: "openAIRealtime" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change('input[type="password"]', "synthetic-key");
  actions.saveProfileCredentials.mockRejectedValueOnce("credential_service_unavailable");
  await act(async () => host.querySelector<HTMLFormElement>(".credential-form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(host.textContent).toContain(profileErrorMessage("credential_service_unavailable"));
  expect(host.querySelector('.credential-unavailable[role="status"]')).toBeNull();
});

it.each(["alibabaCloud", "openAIRealtime"] as const)("keeps an unsaved %s key visible when a connection check recovers stored credentials", async (provider) => {
  await render({ ...settings, profiles: [{ ...profile, provider }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change('input[type="password"]', "synthetic-unsaved-replacement");
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();

  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await click(diagnosticCopy("linux").test);
  // The native connection check also emits this recovered settings snapshot.
  const recovered = { ...settings, profiles: [{ ...profile, provider, credentialState: "present" as const }] };
  await render(recovered);
  expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("synthetic-unsaved-replacement");
  expect(host.querySelector<HTMLButtonElement>('.credential-form button[type="submit"]')!.disabled).toBe(false);

  actions.saveProfileCredentials.mockResolvedValue(recovered);
  actions.selectProfile.mockResolvedValue(recovered);
  await submit();
  expect(actions.saveProfileCredentials).toHaveBeenCalledExactlyOnceWith(profile.id, provider === "alibabaCloud" ? {
    kind: "alibabaTranslation", model: "", apiKey: "synthetic-unsaved-replacement", textTranslation: "followService", endpoint: "", token: "",
  } : { kind: "apiKey", apiKey: "synthetic-unsaved-replacement" });
  expect(host.querySelector('input[type="password"]')).toBeNull();
  await click(I18N.settings.replaceCredentials);
  expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");
});

it("clears a generic replacement draft before confirmed credential deletion", async () => {
  await render({ ...settings, profiles: [{ ...profile, provider: "openAIRealtime", credentialState: "present" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  await change('input[type="password"]', "synthetic-unsaved-replacement");
  await click(I18N.settings.deleteCredentials);
  actions.deleteProfileAPIKey.mockRejectedValue("credential_store_access_denied");
  await click(I18N.settings.confirmDelete);
  expect(actions.deleteProfileAPIKey).toHaveBeenCalledExactlyOnceWith(profile.id);
  expect(host.querySelector('input[type="password"]')).toBeNull();
  await click(I18N.settings.replaceCredentials);
  expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");
  expect(host.querySelector('.settings-toast[data-tone="error"]')?.textContent).toBe(profileErrorMessage("credential_store_access_denied"));
});

it("clears an active profile's previous check on session failure and discards its in-flight result", async () => {
  const configured = { ...settings, profiles: [{ ...profile, credentialState: "present" as const }] };
  await render(configured);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await click(diagnosticCopy().test);
  expect(host.querySelector('.connection-check [data-tone="success"]')?.textContent).toBe(diagnosticCopy().available);
  await render(configured, "error");
  expect(host.querySelector(".connection-check .settings-feedback")).toBeNull();

  await render(configured);
  let complete!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  vi.mocked(testProfileConnection).mockImplementation(() => new Promise((resolve) => { complete = resolve; }));
  await click(diagnosticCopy().test);
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(true);
  await render(configured, "error");
  await act(async () => complete({ credential: "present", service: "available", reason: null }));
  expect(host.querySelector(".connection-check .settings-feedback")).toBeNull();
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(false);
  expect(testProfileConnection).toHaveBeenCalledTimes(2);
});

it("keeps an independent check for another profile when the active session fails", async () => {
  const configured = { ...settings, profiles: [
    { ...profile, credentialState: "present" as const },
    { ...profile, id: "other-synthetic", name: "Other service", credentialState: "present" as const },
  ] };
  await render(configured);
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await click(diagnosticCopy().test);
  await render(configured, "error");
  expect(host.querySelector('.connection-check [data-tone="success"]')?.textContent).toBe(diagnosticCopy().available);
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith("other-synthetic", "speech");
});

async function chooseStageProxy(index: number, label: string) {
  await act(async () => host.querySelectorAll<HTMLButtonElement>('.service-proxies [role="combobox"]')[index]!.click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === label)!;
  await act(async () => option.click());
}

it("saves only the chosen stage with its profile and restores saved choices when switching editors", async () => {
  const other = { ...profile, id: "other", name: "Other", speechNetworkProxy: { mode: "direct" as const, url: null } };
  const snapshot = { ...settings, profiles: [profile, other] };
  actions.updateProfile.mockResolvedValue(snapshot);
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const selectors = host.querySelectorAll('.service-proxies [role="combobox"]');
  expect(selectors.length).toBe(2);
  expect(selectors[0]!.getAttribute("aria-label")).toBe(I18N.settings.speechRecognition);
  expect(selectors[1]!.getAttribute("aria-label")).toBe(I18N.settings.textTranslationLabel);
  await change(".service-detail__name input", "Unsaved name");
  await chooseStageProxy(1, I18N.settings.networkProxyDirect);
  expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith(profile.id, undefined, { textNetworkProxy: { mode: "direct", url: null } });
  expect(host.querySelector<HTMLInputElement>(".service-detail__name input")!.value).toBe("Unsaved name");
  await chooseStageProxy(0, I18N.settings.networkProxyCustom);
  await click(I18N.settings.backToServices);
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1]!.click());
  expect(host.querySelector('.service-proxies [role="combobox"]')?.textContent).toContain(I18N.settings.networkProxyDirect);
  expect(host.querySelector('.service-proxies input')).toBeNull();
  await click(I18N.settings.backToServices);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector('.service-proxies [role="combobox"]')?.textContent).toContain(I18N.settings.networkProxySystem);
  expect(host.querySelector('.service-proxies input')).toBeNull();
});

it("keeps recognition check results when only the text route changes, and rejects a stale text result", async () => {
  vi.mocked(testProfileConnection).mockResolvedValueOnce({ credential: "present", service: "available", reason: null });
  const snapshot = { ...settings, profiles: [{ ...profile, credentialState: "present" as const }] };
  await render(snapshot); await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.checkSpeechRecognition);
  let finish!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  vi.mocked(testProfileConnection).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await click(I18N.settings.checkTextTranslation);
  await render({ ...snapshot, profiles: [{ ...snapshot.profiles[0]!, textNetworkProxy: { mode: "direct", url: null } }] });
  await act(async () => { finish({ credential: "present", service: "available", reason: null }); });
  const results = host.querySelectorAll('.connection-check .settings-feedback');
  expect(results.length).toBe(1);
  expect(results[0]!.textContent).toBe(diagnosticCopy().available);
  expect(testProfileConnection).toHaveBeenCalledTimes(2);
});

it("shows only one effective proxy for an integrated realtime service and locks profile proxies during a paused session", async () => {
  await render({ ...settings, profiles: [{ ...profile, provider: "openAIRealtime" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelectorAll('.service-proxies [role="combobox"]').length).toBe(1);
  expect(host.querySelector('.service-proxies .settings-help-control__description')?.textContent).toContain(I18N.settings.networkProxyIntegratedScope);
  await act(async () => root.render(<><ServiceProfiles settings={settings} sessionIsActive={false} sessionIsPaused /><SettingsToastRegion /></>));
  for (const selector of host.querySelectorAll<HTMLButtonElement>('.service-proxies [role="combobox"]')) expect(selector.disabled).toBe(true);
  expect(actions.updateProfile).not.toHaveBeenCalled();
});


it("prefills Azure nonsecret fields without reading its key and preserves an edited field against late metadata", async () => {
  let finish!: (value: Awaited<ReturnType<typeof profileCredentialEditorState>>) => void;
  vi.mocked(profileCredentialEditorState).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const azure: ServiceProfile = { ...profile, provider: "azureOpenAIRealtime", credentialState: "present" };
  const snapshot = { ...settings, profiles: [azure] };
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  await change('input[id$="-deployment"]', "edited-deployment");
  await act(async () => finish({ savedFields: ["apiKey"], endpoint: "https://synthetic.openai.azure.com", deployment: "stored-deployment", transcriptionDeployment: "stored-transcription" }));
  expect(host.querySelector<HTMLInputElement>('input[id$="-endpoint"]')!.value).toBe("https://synthetic.openai.azure.com");
  expect(host.querySelector<HTMLInputElement>('input[id$="-deployment"]')!.value).toBe("edited-deployment");
  expect(host.querySelector<HTMLInputElement>('input[id$="-transcriptionDeployment"]')!.value).toBe("stored-transcription");
  expect(profileRevealCredential).not.toHaveBeenCalled();
  await change('input[id$="-apiKey"]', "synthetic-new-key");
  actions.saveProfileCredentials.mockResolvedValue(snapshot);
  actions.selectProfile.mockResolvedValue(snapshot);
  await submit();
  expect(actions.saveProfileCredentials).toHaveBeenCalledExactlyOnceWith(profile.id, { kind: "azureOpenAI", endpoint: "https://synthetic.openai.azure.com", deployment: "edited-deployment", transcriptionDeployment: "stored-transcription", apiKey: "synthetic-new-key" });
});

it("expands long Azure configuration values and checks the edited deployment without saving or revealing its key", async () => {
  const endpoint = `https://${"synthetic-resource-".repeat(3)}one.openai.azure.com`;
  const deployment = `synthetic-${"translation-".repeat(12)}deployment`;
  const transcriptionDeployment = `synthetic-${"recognition-".repeat(12)}deployment`;
  vi.mocked(profileCredentialEditorState).mockResolvedValue({ savedFields: ["apiKey"], endpoint, deployment, transcriptionDeployment });
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await render({ ...settings, profiles: [{ ...profile, provider: "azureOpenAIRealtime", credentialState: "present" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  for (const [field, value] of [["endpoint", endpoint], ["deployment", deployment], ["transcriptionDeployment", transcriptionDeployment]] as const) {
    const group = host.querySelector(`input[id$="-${field}"]`)!.closest(".config-input-group")!;
    await act(() => group.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
    expect(group.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe(value);
    expect(group.querySelector<HTMLTextAreaElement>("textarea")!.readOnly).toBe(false);
  }
  expect(testProfileConnection).not.toHaveBeenCalled();
  await change('textarea[id$="-deployment"]', `${deployment}-edited`);
  await click(diagnosticCopy().test);
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith(profile.id, undefined, {
    kind: "azureOpenAI", endpoint, deployment: `${deployment}-edited`, transcriptionDeployment, apiKey: "",
  });
  expect(host.querySelector<HTMLTextAreaElement>('textarea[id$="-deployment"]')!.value).toBe(`${deployment}-edited`);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(host.querySelector('input[id$="-apiKey"]')!.closest(".config-input-group")!.querySelector(".config-input__expand")).toBeNull();
});

it("keeps a new generic key while toggling visibility and discards a late saved read after leaving its profile", async () => {
  const configured: ServiceProfile = { ...profile, provider: "openAIRealtime", credentialState: "present" };
  await render({ ...settings, profiles: [configured] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  await change('.credential-form input', "synthetic-unsaved-key");
  await click(I18N.settings.showCredential);
  expect(host.querySelector<HTMLInputElement>('.credential-form input')!.value).toBe("synthetic-unsaved-key");
  expect(profileRevealCredential).not.toHaveBeenCalled();
  await click(I18N.settings.cancel);
  await click(I18N.settings.replaceCredentials);
  let finish!: (value: string | null) => void;
  vi.mocked(profileRevealCredential).mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await click(I18N.settings.revealSavedCredential);
  await click(I18N.settings.backToServices);
  await act(async () => finish("synthetic-late-key"));
  expect(host.textContent).not.toContain("synthetic-late-key");
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  expect(host.querySelector<HTMLInputElement>('.credential-form input')!.value).toBe("");
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});


it("waits for saved configuration before submitting a replacement key and keeps that draft through retry", async () => {
  let reject!: (reason: unknown) => void;
  vi.mocked(profileCredentialEditorState).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  const configured: ServiceProfile = { ...profile, provider: "openAIRealtime", credentialState: "present" };
  const snapshot = { ...settings, profiles: [configured] };
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  await change('.credential-form input', "synthetic-unsaved-key");
  expect(host.querySelector<HTMLButtonElement>('.credential-form button[type="submit"]')!.disabled).toBe(true);
  await submit();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  await act(async () => reject("synthetic-private-read-error"));
  expect(host.querySelector('.credential-form [role="alert"]')?.textContent).toContain(I18N.settings.profileActionFailed);
  expect(host.textContent).not.toContain("synthetic-private-read-error");
  await submit();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  await click(I18N.settings.retryLoadingSettings);
  expect(host.querySelector<HTMLInputElement>('.credential-form input')!.value).toBe("synthetic-unsaved-key");
  actions.saveProfileCredentials.mockResolvedValue(snapshot);
  actions.selectProfile.mockResolvedValue(snapshot);
  await submit();
  expect(actions.saveProfileCredentials).toHaveBeenCalledExactlyOnceWith(profile.id, { kind: "apiKey", apiKey: "synthetic-unsaved-key" });
});

it.each([
  { provider: "openAIRealtime", fields: { apiKey: "synthetic-new-key" }, expected: { kind: "apiKey", apiKey: "synthetic-new-key" } },
  { provider: "azureOpenAIRealtime", fields: { endpoint: "https://synthetic.openai.azure.com", deployment: "translate", transcriptionDeployment: "transcribe", apiKey: "synthetic-new-key" },
    expected: { kind: "azureOpenAI", endpoint: "https://synthetic.openai.azure.com", deployment: "translate", transcriptionDeployment: "transcribe", apiKey: "synthetic-new-key" } },
  { provider: "tencentCloud", fields: { appId: "123", secretId: "synthetic-id", secretKey: "synthetic-key" },
    expected: { kind: "tencentCloud", appId: "123", secretId: "synthetic-id", secretKey: "synthetic-key" } },
  { provider: "baiduTranslate", fields: { appId: "123", appKey: "synthetic-key" }, expected: { kind: "baiduTranslate", appId: "123", appKey: "synthetic-key" } },
] as const)("checks an explicit complete $provider replacement while saved metadata is unavailable", async ({ provider, fields, expected }) => {
  let reject!: (reason: unknown) => void;
  vi.mocked(profileCredentialEditorState).mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await render({ ...settings, profiles: [{ ...profile, provider, credentialState: "present" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  const entries = Object.entries(fields);
  for (const [index, [field, value]] of entries.entries()) {
    await change(`.credential-form input[id$="-${field}"]`, value);
    expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(index < entries.length - 1);
  }
  await act(async () => reject("credential_store_unavailable"));
  expect(host.querySelector('.credential-form [role="alert"]')).not.toBeNull();
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(false);
  await click(diagnosticCopy().test);
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith(profile.id, undefined, expected);
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("shows independent translation names in the profile list and route picker without changing protocol icons", async () => {
  const named: ServiceProfile = { ...profile, credentialState: "present", textTranslation: "openAICompatible",
    textTranslationNames: { openAICompatible: "Office translator", deepLX: "Local translator" } };
  await render({ ...settings, profiles: [named] });
  expect(host.querySelector(".service-row__translation")?.textContent).toBe(`${I18N.settings.textTranslationLabel} · Office translator`);
  expect(host.querySelector(".service-row__translation .provider-icon")?.getAttribute("data-provider")).toBe("openAICompatible");
  expect(host.querySelector(".service-row__main")?.getAttribute("aria-label")).toContain("Office translator");
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const picker = host.querySelector<HTMLButtonElement>('.service-stage--translation [role="combobox"]')!;
  expect(picker.textContent).toBe("Office translator");
  await act(async () => picker.click());
  const local = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent === "Local translator")!;
  expect(local.querySelector(".provider-icon")?.getAttribute("data-provider")).toBe("deepLX");
  await act(async () => local.click());
  expect(host.querySelector<HTMLInputElement>(".translation-name-field input")!.value).toBe("Local translator");
  expect(host.querySelector<HTMLInputElement>(".translation-name-field input")!.placeholder).toBe("DeepLX");
  expect(actions.updateProfile).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
});

it.each(["alibabaCloud", "customDashScopeASR", "customOpenAIASR"] as const)(
  "saves and clears a %s translation name independently of pending credentials and profile-name edits", async provider => {
    const named: ServiceProfile = { ...profile, provider, credentialState: "present", speechCredentialState: "present", textCredentialState: "present",
      textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: "Old name" } };
    const snapshot = { ...settings, profiles: [named] };
    vi.mocked(profileCredentialEditorState).mockImplementation(async request => request.textTranslation
      ? { savedFields: ["token"], endpoint: "https://synthetic.example/v1", model: "saved-model" }
      : { savedFields: ["apiKey"] });
    const renamed: SettingsSnapshot = { ...snapshot, profiles: [{ ...named, textTranslationNames: { openAICompatible: "Office translator" } }] };
    actions.updateProfile.mockResolvedValueOnce(renamed);
    await render(snapshot);
    await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
    expect(host.querySelector<HTMLInputElement>('.service-stage--translation input[id$="-endpoint"]')!.value).toBe("https://synthetic.example/v1");
    expect(host.querySelector<HTMLInputElement>('.service-stage--translation input[id$="-model"]')!.value).toBe("saved-model");
    await change(".service-detail__name input", "Unsaved configuration name");
    await change('.service-stage--translation input[id$="-token"]', "synthetic-unsaved-token");
    await change(".translation-name-field input", "  Office translator  ");
    await act(async () => host.querySelector<HTMLInputElement>(".translation-name-field input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith(profile.id, undefined, {
      textTranslationName: { route: "openAICompatible", name: "Office translator" },
    });
    expect(host.querySelector('.settings-toast[role="status"]')).toBeNull();
    expect(host.querySelector(".translation-name-field .settings-button")).toBeNull();
    await render(renamed);
    expect(host.querySelector<HTMLInputElement>('.service-stage--translation input[id$="-token"]')!.value).toBe("synthetic-unsaved-token");
    expect(host.querySelector<HTMLInputElement>(".service-detail__name input")!.value).toBe("Unsaved configuration name");
    expect(host.querySelector('.service-stage--translation [role="combobox"]')?.textContent).toBe("Office translator");
    expect(profileRevealCredential).not.toHaveBeenCalled();

    const cleared: SettingsSnapshot = { ...snapshot, profiles: [{ ...named, textTranslationNames: {} }] };
    actions.updateProfile.mockResolvedValueOnce(cleared);
    await change(".translation-name-field input", "   ");
    await act(async () => host.querySelector<HTMLInputElement>(".translation-name-field input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(actions.updateProfile).toHaveBeenLastCalledWith(profile.id, undefined, {
      textTranslationName: { route: "openAICompatible", name: "" },
    });
    await render(cleared);
    expect(host.querySelector<HTMLInputElement>(".translation-name-field input")!.value.trim()).toBe("");
    expect(host.querySelector('.service-stage--translation [role="combobox"]')?.textContent).toBe(I18N.settings.textTranslationOpenAICompatible);
    expect(host.querySelector<HTMLInputElement>('.service-stage--translation input[id$="-token"]')!.value).toBe("synthetic-unsaved-token");
    expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
    expect(actions.selectProfile).not.toHaveBeenCalled();
  },
);

it.each(["customDashScopeASR", "customOpenAIASR"] as const)(
  "shows and saves a %s recognition name independently of configuration, translation, and credential drafts", async provider => {
    vi.useFakeTimers();
    const named: ServiceProfile = { ...profile, provider, name: "Whisper · Index", credentialState: "present", speechCredentialState: "present", textCredentialState: "present",
      speechRecognitionName: "Old recognizer", textTranslation: "openAICompatible", textTranslationNames: { openAICompatible: "Index · Local" } };
    const snapshot = { ...settings, profiles: [named] };
    vi.mocked(profileCredentialEditorState).mockImplementation(async request => request.textTranslation
      ? { savedFields: ["token"], endpoint: "https://synthetic.example/v1", model: "saved-translation-model" }
      : { savedFields: ["apiKey"], endpoint: "wss://synthetic.example/speech", model: "saved-speech-model" });
    const renamed = { ...snapshot, profiles: [{ ...named, speechRecognitionName: "Whisper" }] };
    actions.updateProfile.mockResolvedValueOnce(renamed);
    await render(snapshot);
    expect(host.querySelector(".service-row__copy strong")?.textContent).toBe(named.name);
    expect(host.querySelector(".service-row__provider")?.textContent).toBe(`${I18N.settings.speechRecognition} · Old recognizer`);
    expect(host.querySelector(".service-row__translation")?.textContent).toBe(`${I18N.settings.textTranslationLabel} · Index · Local`);
    expect(host.querySelector(".service-row .provider-icon")?.getAttribute("data-provider")).toBe(provider);
    expect(host.querySelector(".service-row__main")?.getAttribute("aria-label")).toContain("Old recognizer");
    await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
    const nameField = host.querySelector<HTMLInputElement>(".recognition-name-field input")!;
    expect(host.querySelector(".recognition-name-field label")?.textContent).toBe(I18N.settings.speechRecognitionName);
    expect(nameField.value).toBe("Old recognizer");
    expect(nameField.placeholder).toBe(providerDisplayName(provider));
    expect(host.querySelector('input[id$="-speech-key"]')).toBeNull();
    await click(I18N.settings.editSpeechConfiguration);
    await change(".service-detail__name input", "Unsaved configuration name");
    await change(".translation-name-field input", "Unsaved translator");
    await change('input[id$="-speech-key"]', "synthetic-unsaved-speech-key");
    await change('.service-stage--translation input[id$="-token"]', "synthetic-unsaved-translation-token");
    await change(".recognition-name-field input", "  Whisper  ");
    await act(async () => nameField.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith(profile.id, undefined, { speechRecognitionName: "Whisper" });
    await render(renamed);
    expect(host.querySelector<HTMLInputElement>(".service-detail__name input")!.value).toBe("Unsaved configuration name");
    expect(host.querySelector<HTMLInputElement>(".translation-name-field input")!.value).toBe("Unsaved translator");
    expect(host.querySelector<HTMLInputElement>('input[id$="-speech-key"]')!.value).toBe("synthetic-unsaved-speech-key");
    expect(host.querySelector<HTMLInputElement>('.service-stage--translation input[id$="-token"]')!.value).toBe("synthetic-unsaved-translation-token");
    expect(host.querySelector('.service-stage--translation [role="combobox"]')?.textContent).toBe("Index · Local");

    const cleared = { ...snapshot, profiles: [{ ...named, speechRecognitionName: undefined }] };
    actions.updateProfile.mockResolvedValueOnce(cleared);
    await change(".recognition-name-field input", "   ");
    await act(async () => nameField.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(actions.updateProfile).toHaveBeenCalledTimes(2);
    expect(actions.updateProfile).toHaveBeenLastCalledWith(profile.id, undefined, { speechRecognitionName: "" });
    await render(cleared);
    expect(nameField.value.trim()).toBe("");
    expect(nameField.placeholder).toBe(providerDisplayName(provider));
    expect(host.querySelector<HTMLInputElement>('input[id$="-speech-key"]')!.value).toBe("synthetic-unsaved-speech-key");
    expect(host.querySelector('.settings-toast[role="status"]')).toBeNull();
    expect(host.querySelector(".recognition-name-field .settings-button")).toBeNull();
    expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
    expect(actions.selectProfile).not.toHaveBeenCalled();
    expect(profileRevealCredential).not.toHaveBeenCalled();
  },
);

it("keeps a failed recognition-name draft available for retry without touching credentials", async () => {
  const named: ServiceProfile = { ...profile, provider: "customOpenAIASR", credentialState: "present", speechCredentialState: "present", speechRecognitionName: "Old recognizer" };
  const snapshot = { ...settings, profiles: [named] };
  actions.updateProfile.mockRejectedValueOnce(new Error("synthetic-private-save-error"));
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change(".recognition-name-field input", "Whisper");
  await act(async () => host.querySelector<HTMLInputElement>(".recognition-name-field input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
  expect(host.querySelector<HTMLInputElement>(".recognition-name-field input")!.value).toBe("Whisper");
  expect(host.querySelector('.recognition-name-field [role="alert"]')?.textContent).toContain(I18N.settings.profileActionFailed);
  expect(host.textContent).not.toContain("synthetic-private-save-error");
  expect(host.querySelector<HTMLInputElement>(".recognition-name-field input")!.disabled).toBe(false);
  actions.updateProfile.mockResolvedValueOnce({ ...snapshot, profiles: [{ ...named, speechRecognitionName: "Whisper" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".recognition-name-field .auto-save-name-field__error button")!.click());
  expect(actions.updateProfile).toHaveBeenCalledTimes(2);
  expect(actions.updateProfile).toHaveBeenLastCalledWith(profile.id, undefined, { speechRecognitionName: "Whisper" });
  expect(host.querySelector('.recognition-name-field [role="alert"]')).toBeNull();
  expect(host.querySelector('.settings-toast[role="status"]')).toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("disables recognition-name editing during an active session", async () => {
  const named: ServiceProfile = { ...profile, provider: "customDashScopeASR", credentialState: "present", speechCredentialState: "present", speechRecognitionName: "Whisper",
    credentialStorage: "localFile" as const };
  const snapshot = { ...settings, profiles: [named] };
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await act(async () => root.render(<><ServiceProfiles settings={snapshot} sessionIsActive /><SettingsToastRegion /></>));
  const nameField = host.querySelector<HTMLInputElement>(".recognition-name-field input")!;
  expect(nameField.value).toBe("Whisper");
  expect(nameField.readOnly).toBe(false);
  expect(nameField.disabled).toBe(true);
  expect(actions.updateProfile).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("keeps a failed translation-name edit beside its field and retries without credential writes", async () => {
  const named: ServiceProfile = { ...profile, credentialState: "present", textTranslation: "deepLX", textTranslationNames: { deepLX: "Old translator" } };
  const snapshot = { ...settings, profiles: [named] };
  actions.updateProfile.mockRejectedValueOnce(new Error("synthetic-private-save-error"));
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change(".translation-name-field input", "Unsaved translator");
  await act(async () => host.querySelector<HTMLInputElement>(".translation-name-field input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
  expect(host.querySelector<HTMLInputElement>(".translation-name-field input")!.value).toBe("Unsaved translator");
  expect(host.querySelector('.translation-name-field [role="alert"]')?.textContent).toContain(I18N.settings.profileActionFailed);
  expect(host.querySelector('.settings-toast[role="status"]')).toBeNull();
  expect(host.textContent).not.toContain("synthetic-private-save-error");
  expect(host.querySelector<HTMLInputElement>(".translation-name-field input")!.disabled).toBe(false);
  actions.updateProfile.mockResolvedValueOnce({ ...snapshot, profiles: [{ ...named, textTranslationNames: { deepLX: "Unsaved translator" } }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".auto-save-name-field__error button")!.click());
  expect(actions.updateProfile).toHaveBeenCalledTimes(2);
  expect(actions.updateProfile).toHaveBeenLastCalledWith(profile.id, undefined, {
    textTranslationName: { route: "deepLX", name: "Unsaved translator" },
  });
  expect(host.querySelector('.translation-name-field [role="alert"]')).toBeNull();
  expect(host.querySelector('.settings-toast[role="status"]')).toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
});

it("keeps both name fields editable while independent autosaves finish in either order", async () => {
  vi.useFakeTimers();
  const named: ServiceProfile = { ...profile, credentialState: "present", textTranslation: "deepLX", textTranslationNames: { deepLX: "Old translator" } };
  const snapshot = { ...settings, profiles: [named] };
  let finishProfile!: (value: SettingsSnapshot) => void;
  let finishTranslation!: (value: SettingsSnapshot) => void;
  actions.updateProfile.mockImplementation((_id, name) => new Promise(resolve => {
    if (name === undefined) finishTranslation = resolve;
    else finishProfile = resolve;
  }));
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change(".service-detail__name input", "Configuration B 站");
  await change(".translation-name-field input", "B 站 · @home / (测试) 😀");
  await act(async () => vi.advanceTimersByTimeAsync(400));
  expect(actions.updateProfile).toHaveBeenCalledWith(profile.id, "Configuration B 站");
  expect(actions.updateProfile).toHaveBeenCalledWith(profile.id, undefined, { textTranslationName: { route: "deepLX", name: "B 站 · @home / (测试) 😀" } });
  expect(host.querySelector<HTMLInputElement>(".service-detail__name input")!.disabled).toBe(false);
  expect(host.querySelector<HTMLInputElement>(".translation-name-field input")!.disabled).toBe(false);
  const translated = { ...snapshot, profiles: [{ ...named, textTranslationNames: { deepLX: "B 站 · @home / (测试) 😀" } }] };
  await act(async () => finishTranslation(translated));
  await render(translated);
  expect(host.querySelector<HTMLInputElement>(".service-detail__name input")!.value).toBe("Configuration B 站");
  const both = { ...translated, profiles: [{ ...translated.profiles[0], name: "Configuration B 站" }] };
  await act(async () => finishProfile(both));
  await render(both);
  expect(host.querySelector(".service-detail__title h2")?.textContent).toBe("Configuration B 站");
  expect(host.querySelector('.service-stage--translation [role="combobox"]')?.textContent).toBe("B 站 · @home / (测试) 😀");
  expect(host.querySelector('.settings-toast[role="status"]')).toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it("checks a new OpenAI key draft directly without saving or selecting the profile", async () => {
  const unconfigured: ServiceProfile = { ...profile, provider: "openAIRealtime", credentialState: "missing" };
  vi.mocked(profileCredentialEditorState).mockResolvedValue({ savedFields: [] });
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await render({ ...settings, profiles: [unconfigured] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change('.credential-form input[id$="-apiKey"]', "  synthetic-new-api-key  ");
  await click(diagnosticCopy().test);
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith(profile.id, undefined, {
    kind: "apiKey", apiKey: "synthetic-new-api-key",
  });
  expect(host.querySelector('.connection-check [data-tone="success"]')?.textContent).toBe(diagnosticCopy().available);
  expect(host.querySelector<HTMLInputElement>('.credential-form input[id$="-apiKey"]')!.value).toBe("  synthetic-new-api-key  ");
  expect(host.querySelector(".credential-badge")?.getAttribute("aria-label")).toBe(I18N.settings.credentialMissing);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.updateProfile).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
});

it("checks saved OpenAI credentials after reveal, then checks an actual secret edit as a separate draft", async () => {
  const configured: ServiceProfile = { ...profile, provider: "openAIRealtime", credentialState: "present" };
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-stored-api-key");
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await render({ ...settings, profiles: [configured] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  await click(I18N.settings.revealSavedCredential);
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: profile.id, field: "apiKey" });
  expect(host.querySelector<HTMLInputElement>('.credential-form input[id$="-apiKey"]')!.value).toBe("synthetic-stored-api-key");
  await click(diagnosticCopy().test);
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith(profile.id);
  expect(host.querySelector('.connection-check [data-tone="success"]')).not.toBeNull();

  await change('.credential-form input[id$="-apiKey"]', "synthetic-replacement-api-key");
  expect(host.querySelector(".connection-check .settings-feedback")).toBeNull();
  await click(diagnosticCopy().test);
  expect(testProfileConnection).toHaveBeenNthCalledWith(2, profile.id, undefined, {
    kind: "apiKey", apiKey: "synthetic-replacement-api-key",
  });
  expect(host.querySelector<HTMLInputElement>('.credential-form input[id$="-apiKey"]')!.value).toBe("synthetic-replacement-api-key");
  expect(host.querySelector('.connection-check [data-tone="success"]')).not.toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.updateProfile).not.toHaveBeenCalled();
  expect(actions.selectProfile).not.toHaveBeenCalled();
});


it.each([true, false])("saves a declaration and reports normalized recognition when event-first is %s", async eventFirst => {
  const custom: ServiceProfile = { ...profile, provider: "customDashScopeASR", textTranslation: "openAICompatible", customSpeechSourceLanguages: ["en", "fr"] };
  const before: SettingsSnapshot = { ...settings, profiles: [custom], sourceLanguage: "fr" };
  const after: SettingsSnapshot = { ...before, sourceLanguage: "auto", profiles: [{ ...custom, customSpeechSourceLanguages: ["en"] }] };
  let resolve!: (value: SettingsSnapshot) => void;
  actions.updateProfile.mockImplementationOnce(() => new Promise(value => { resolve = value; }));
  await render(before);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const button = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".custom-speech-languages button")].find(item => item.textContent === label)!;
  await act(async () => button(I18N.settings.customSpeechLanguagesEdit).click());
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>(".custom-speech-languages__choices button")].find(item => item.textContent === "Frenchfr")!.click());
  await act(async () => button(I18N.settings.customSpeechLanguagesSave).click());
  expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith(custom.id, undefined, { customSpeechSourceLanguages: ["en"] });
  if (eventFirst) await render(after);
  expect(host.textContent).not.toContain(I18N.settings.recognitionLanguageAdjusted("French", I18N.settings.recognitionServiceDefault));
  await act(async () => resolve(after));
  if (!eventFirst) await render(after);
  expect(host.textContent).toContain(I18N.settings.recognitionLanguageAdjusted("French", I18N.settings.recognitionServiceDefault));
  expect(host.querySelector(".custom-speech-languages__expanded")).toBeNull();
  expect(host.querySelector(".custom-speech-languages__summary")?.textContent).toBe(I18N.settings.customSpeechLanguagesCount(1));
});

it("keeps language declaration edits locked during an active session", async () => {
  const snapshot: SettingsSnapshot = { ...settings, profiles: [{ ...profile, provider: "customOpenAIASR" }] };
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await act(async () => root.render(<><ServiceProfiles settings={snapshot} sessionIsActive={true} sessionStatusKind="listening" /><SettingsToastRegion /></>));
  expect(host.querySelector<HTMLButtonElement>(".custom-speech-languages .settings-button")?.disabled).toBe(true);
  expect(actions.updateProfile).not.toHaveBeenCalled();
});

it.each([false, true])("switches saved services while live or paused without unlocking edits (paused=%s)", async paused => {
  const other = { ...profile, id: "other", name: "Other model", credentialState: "present" as const };
  const snapshot = { ...settings, profiles: [profile, other] };
  let finish!: (value: SettingsSnapshot) => void;
  actions.selectProfile.mockReturnValue(new Promise<SettingsSnapshot>(resolve => { finish = resolve; }));
  await act(async () => root.render(<><ServiceProfiles settings={snapshot} sessionIsActive={!paused} sessionIsPaused={paused} sessionStatusKind="listening" /><SettingsToastRegion /></>));
  const choices = host.querySelectorAll<HTMLButtonElement>(".service-row__main");
  expect(choices[1].disabled).toBe(false);
  const add = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.includes(I18N.settings.addProfile))!;
  expect(add.disabled).toBe(true);
  await act(async () => { choices[1].click(); choices[1].click(); });
  expect(actions.selectProfile).toHaveBeenCalledExactlyOnceWith(other.id);
  expect(choices[1].disabled).toBe(true);
  await act(async () => finish({ ...snapshot, activeProfileId: other.id }));
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector<HTMLInputElement>(`#profile-name-${profile.id}`)?.disabled).toBe(true);
});

it.each(["connecting", "stopping"] as const)("blocks service selection during %s", async status => {
  await act(async () => root.render(<ServiceProfiles settings={{ ...settings, profiles: [profile, { ...profile, id: "other" }] }} sessionIsActive sessionStatusKind={status} />));
  for (const button of host.querySelectorAll<HTMLButtonElement>(".service-row__main")) expect(button.disabled).toBe(true);
  expect(actions.selectProfile).not.toHaveBeenCalled();
});

it("keeps the current service and shows a sanitized recording restriction when switching fails", async () => {
  actions.selectProfile.mockRejectedValue("profile_switch_recording_requires_stop");
  await act(async () => root.render(<><ServiceProfiles settings={{ ...settings, profiles: [profile, { ...profile, id: "other", credentialState: "present" }] }} sessionIsActive sessionStatusKind="listening" /><SettingsToastRegion /></>));
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__main")[1].click());
  expect(host.querySelector('.service-row[data-active="true"]')).toBe(host.querySelector(".service-row"));
  expect(host.querySelector(".settings-toast")?.textContent).toContain(I18N.settings.profileSwitchRecordingRequiresStop);
  expect(host.querySelectorAll<HTMLButtonElement>(".service-row__main")[1].disabled).toBe(false);
});


it("reports reconnect failure after the selected profile is already saved without a success toast", async () => {
  const other: ServiceProfile = { ...profile, id: "other", credentialState: "present" };
  const snapshot = { ...settings, profiles: [profile, other] };
  const saved = { ...snapshot, activeProfileId: other.id };
  const error = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
  let reject!: (reason: string) => void;
  actions.selectProfile.mockReturnValue(new Promise<SettingsSnapshot>((_resolve, failure) => { reject = failure; }));
  await act(async () => root.render(<><ServiceProfiles settings={snapshot} sessionIsActive sessionStatusKind="listening" /><SettingsToastRegion /></>));
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__main")[1].click());
  await render(saved, "error");
  await act(async () => reject(error));
  expect(host.querySelector('.service-row[data-active="true"]')).toBe(host.querySelectorAll(".service-row")[1]);
  expect(host.querySelector('.settings-toast[data-tone="error"]')?.textContent).toContain(profileErrorMessage(error));
  expect(host.querySelector('.settings-toast[data-tone="success"]')).toBeNull();
  expect(host.textContent).not.toContain(error);
  expect(actions.selectProfile).toHaveBeenCalledExactlyOnceWith(other.id);
});

it("opens and focuses the active Apple resource editor from another profile without selecting or downloading anything", async () => {
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", () => {});
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  const snapshot = { ...appleSettings(), profiles: [profile, appleProfile] };
  const show = async (request: number, visible = true) => act(async () => root.render(
    <ServiceProfiles settings={snapshot} sessionIsActive={false} visible={visible} appleResourcesRequest={request} />,
  ));
  await show(0);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector(".service-detail__title")?.textContent).toContain(profile.name);
  await show(1);
  const resources = host.querySelector<HTMLElement>("#apple-speech-resources")!;
  expect(resources).not.toBeNull();
  expect(host.querySelector(".service-detail__title")?.textContent).toContain(appleProfile.name);
  expect(document.activeElement).toBe(resources);
  expect(resources.scrollIntoView).toHaveBeenCalledWith({ block: "start" });
  await click(I18N.settings.backToServices);
  await show(1);
  expect(host.querySelector(".service-detail")).toBeNull();
  await show(2, false);
  expect(host.querySelector(".service-detail")).toBeNull();
  await show(2);
  expect(document.activeElement).toBe(host.querySelector("#apple-speech-resources"));
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(actions.saveSettings).not.toHaveBeenCalled();
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

it.each([false, true])("waits for preparation without reviving a cancelled resource intent (cancelled=%s)", async cancelled => {
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", () => {});
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  const otherApple = { ...appleProfile, id: "other-apple", name: "Other Apple" };
  const snapshot = { ...appleSettings(), profiles: [appleProfile, otherApple] };
  const show = async (request: number) => act(async () => root.render(
    <ServiceProfiles settings={snapshot} sessionIsActive={false} appleResourcesRequest={request} />,
  ));
  let prepared!: (value: typeof appleSupport) => void;
  vi.mocked(prepareAppleSpeechLanguage).mockReturnValue(new Promise(resolve => { prepared = resolve; }));
  await show(0);
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  await click(I18N.settings.appleSpeechDownloadAndUse);
  await show(1);
  expect(host.querySelector(".service-detail__title")?.textContent).toContain(otherApple.name);
  if (cancelled) await show(0);
  await act(async () => prepared({ ...appleSupport, languages: appleSupport.languages.map(item => ({ ...item, installed: true, status: "installed" as const })) }));
  expect(host.querySelector(".service-detail__title")?.textContent).toContain(cancelled ? otherApple.name : appleProfile.name);
  if (!cancelled) expect(document.activeElement).toBe(host.querySelector("#apple-speech-resources"));
  expect(prepareAppleSpeechLanguage).toHaveBeenCalledOnce();
});

it("waits for initialized Apple settings and does not revive a resource intent received for a non-Apple profile", async () => {
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", () => {});
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(appleSupport);
  const show = async (snapshot: SettingsSnapshot, request: number) => act(async () => root.render(
    <ServiceProfiles settings={snapshot} sessionIsActive={false} appleResourcesRequest={request} />,
  ));
  boot.initializationStatus = "loading";
  await show(settings, 1);
  expect(host.querySelector("#apple-speech-resources")).toBeNull();
  boot.initializationStatus = "ready";
  await show(appleSettings(), 1);
  expect(document.activeElement).toBe(host.querySelector("#apple-speech-resources"));
  await show(settings, 2);
  await show(appleSettings(), 2);
  const scroller = vi.mocked(Element.prototype.scrollIntoView);
  scroller.mockClear();
  await act(async () => document.body.focus());
  await show(appleSettings(), 2);
  expect(scroller).not.toHaveBeenCalled();
});


it("checks the displayed Apple language on an inactive profile without saving it", async () => {
  const readySupport = { ...appleSupport, languages: appleSupport.languages.map(language => ({ ...language, installed: true, status: "installed" as const })) };
  vi.mocked(getAppleSpeechSupport).mockResolvedValue(readySupport);
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null, elapsedMs: 95 });
  await render({ ...settings, profiles: [profile, appleProfile], sourceLanguage: "auto" });
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  await click(I18N.settings.checkSpeechRecognition);
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith(appleProfile.id, "speech", undefined, "en");
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(actions.saveSettings).not.toHaveBeenCalled();
  expect(host.querySelector(".connection-check__elapsed")?.textContent).toContain("95 ms");
  await act(async () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!.click());
  await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === SOURCE_LANGUAGE_DISPLAY_NAMES.ja)!.click());
  expect(host.querySelector(".connection-check__elapsed")).toBeNull();
  await click(I18N.settings.checkSpeechRecognition);
  expect(testProfileConnection).toHaveBeenLastCalledWith(appleProfile.id, "speech", undefined, "ja");
});

it("applies an inactive Apple profile and selected language in one action and surfaces failure", async () => {
  vi.mocked(getAppleSpeechSupport).mockResolvedValue({ ...appleSupport, languages: appleSupport.languages.map(language => ({ ...language, installed: true, status: "installed" as const })) });
  actions.selectProfile.mockRejectedValue(new Error("apple_speech_assets_missing"));
  await render({ ...settings, profiles: [profile, appleProfile], sourceLanguage: "en" });
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  await click(I18N.settings.appleSpeechUseLanguage);
  expect(actions.selectProfile).toHaveBeenCalledExactlyOnceWith(appleProfile.id, "en");
  expect(actions.saveSettings).not.toHaveBeenCalled();
  expect(document.body.textContent).toContain(I18N.settings.appleSpeechAssetsMissing);
  expect(host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.sourceLanguage}"]`)!.disabled).toBe(false);
});

it("opens the current configuration instead of the previously viewed editor on every navigation request", async () => {
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", () => {});
  const current = { ...profile, id: "current", name: "Current configuration", credentialState: "present" as const };
  const other = { ...profile, id: "other", name: "Another configuration", credentialState: "present" as const };
  const snapshot = { ...settings, profiles: [current, other], activeProfileId: current.id };
  await render(snapshot);
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  expect(host.querySelector(".service-detail h2")?.textContent).toBe(other.name);
  const navigate = async (request: number) => act(async () => root.render(<><ServiceProfiles settings={snapshot} sessionIsActive={true} sessionStatusKind="listening" profileEditorRequest={request} /><SettingsToastRegion /></>));
  await navigate(1);
  expect(host.querySelector(".service-detail h2")?.textContent).toBe(current.name);
  expect(document.activeElement).toBe(host.querySelector(".service-detail h2"));
  expect(host.querySelector<HTMLInputElement>(`#profile-name-${current.id}`)?.disabled).toBe(true);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-back")!.click());
  expect(host.querySelector(".service-detail")).toBeNull();
  await navigate(2);
  expect(host.querySelector(".service-detail h2")?.textContent).toBe(current.name);
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it("waits for settings initialization before consuming a current-configuration navigation request", async () => {
  boot.initializationStatus = "loading";
  const draw = () => root.render(<ServiceProfiles settings={settings} sessionIsActive={false} profileEditorRequest={1} />);
  await act(async () => draw());
  expect(host.querySelector(".service-detail")).toBeNull();
  boot.initializationStatus = "ready";
  await act(async () => draw());
  expect(host.querySelector(".service-detail h2")?.textContent).toBe(profile.name);
});

it.each([true, false])("defers current-configuration navigation until a pending save ends, keeping a failed draft when success=%s", async success => {
  const current = { ...profile, id: "current", name: "Current", credentialState: "present" as const };
  const other: ServiceProfile = { ...profile, id: "other", name: "Other", provider: "customDashScopeASR", textTranslation: "openAICompatible", customSpeechSourceLanguages: ["en", "fr"] };
  const snapshot = { ...settings, profiles: [current, other], activeProfileId: current.id };
  let resolve!: (value: SettingsSnapshot) => void, reject!: (error: Error) => void;
  actions.updateProfile.mockImplementationOnce(() => new Promise((done, failed) => { resolve = done; reject = failed; }));
  await render(snapshot);
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  const declarationButton = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".custom-speech-languages button")].find(item => item.textContent === label)!;
  await act(async () => declarationButton(I18N.settings.customSpeechLanguagesEdit).click());
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>(".custom-speech-languages__choices button")].find(item => item.textContent === "Frenchfr")!.click());
  await act(async () => declarationButton(I18N.settings.customSpeechLanguagesSave).click());
  const navigate = (request: number) => root.render(<><ServiceProfiles settings={snapshot} sessionIsActive={false} profileEditorRequest={request} /><SettingsToastRegion /></>);
  await act(async () => navigate(1));
  expect(host.querySelector(".service-detail h2")?.textContent).toBe(other.name);
  await act(async () => success ? resolve(snapshot) : reject(new Error("synthetic save failure")));
  expect(host.querySelector(".service-detail h2")?.textContent).toBe(success ? current.name : other.name);
  if (!success) {
    expect(host.querySelector(".custom-speech-languages__expanded")).not.toBeNull();
    expect(host.querySelector('.custom-speech-languages [role="alert"]')).not.toBeNull();
    await act(async () => navigate(2));
    expect(host.querySelector(".service-detail h2")?.textContent).toBe(current.name);
  }
});
