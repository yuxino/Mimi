// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { diagnosticCopy, profileErrorMessage } from "../../lib/connectionDiagnostics";
import { I18N, providerDisplayName, setStoredUiLanguage } from "../../lib/i18n";
import { profileRevealCredential, testProfileConnection } from "../../lib/ipc";
import { SERVICE_PROVIDERS, sourceLanguagesForSettings, targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES } from "../../lib/types";
import type { ServiceProfile, SettingsSnapshot } from "../../lib/types";
import { ServiceProfiles } from "./ServiceProfiles";

const actions = vi.hoisted(() => ({
  createProfile: vi.fn(), updateProfile: vi.fn(), selectProfile: vi.fn(),
  saveSettings: vi.fn(), deleteProfile: vi.fn(), saveProfileCredentials: vi.fn(), deleteProfileAPIKey: vi.fn(),
}));
const boot = vi.hoisted(() => ({ initializationStatus: "ready" as "ready" | "loading" | "error", initializationError: null as "timeout" | "unavailable" | null, init: vi.fn() }));
vi.mock("../../lib/store", () => ({ useStore: (select: (state: typeof actions & typeof boot & { settings: { windowsAudioSource: string }; session: { isActive: boolean; isPaused: boolean } }) => unknown) => select({ ...actions, ...boot, settings: { windowsAudioSource: "" }, session: { isActive: false, isPaused: false } }) }));
vi.mock("../../lib/ipc", () => ({ isTauri: false, testProfileConnection: vi.fn(), profileRevealCredential: vi.fn(), setOverlayPointerCursor: vi.fn() }));

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
  boot.initializationStatus = "ready"; boot.initializationError = null; boot.init.mockReset().mockResolvedValue(undefined);
  setStoredUiLanguage("en");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals();
  vi.useRealTimers();
});
async function render(snapshot = settings, sessionStatusKind: "idle" | "error" = "idle") { await act(async () => root.render(<><ServiceProfiles settings={snapshot} sessionIsActive={false} sessionStatusKind={sessionStatusKind} /><SettingsToastRegion /></>)); }
it("shows a custom speech profile as ready for Original even when its independent translation key is missing", async () => {
  const custom: ServiceProfile = { ...profile, provider: "customDashScopeASR", credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing", textTranslation: "deepL" };
  await render({ ...settings, targetLanguage: "original", profiles: [custom] });
  expect(host.querySelector(".credential-badge")?.getAttribute("aria-label")).toBe(I18N.settings.credentialPresent);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector(".service-detail__title .credential-badge")?.getAttribute("aria-label")).toBe(I18N.settings.credentialPresent);
  expect(host.querySelector(".service-stage--translation")?.textContent).toContain("DeepL");
  expect(host.querySelector('input[id$="-speech-key"]')).toBeNull();
});
it.each(["en", "zh", "ja"].flatMap(language => ["alibabaCloud", "googleGeminiLive"].map(provider => ({language, provider}))) as {language: "en" | "zh" | "ja"; provider: "alibabaCloud" | "googleGeminiLive"}[])("keeps $provider local dev credentials out of editors and reveal in $language", async ({language, provider}) => {
  setStoredUiLanguage(language);
  const snapshot: SettingsSnapshot = { ...settings, credentialStorage: "localDevFile", profiles: [{ ...profile, provider, credentialStorage: "localDevFile", credentialState: "present" }] };
  await render(snapshot);
  expect(host.querySelector(".services-hint .settings-help-control__description")?.textContent).toBe(diagnosticCopy().localDevReadOnly);
  expect(host.querySelector("p.services-hint")).toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.textContent).toContain(diagnosticCopy().localDevReadOnly);
  expect(host.querySelector('input[type="password"]')).toBeNull();
  expect(host.querySelector(".credential-form")).toBeNull();
  expect([...host.querySelectorAll(".service-stage h3")].map(node => node.textContent)).toEqual(provider === "alibabaCloud" ? [I18N.settings.speechRecognition, I18N.settings.textTranslationLabel] : [I18N.settings.voiceTranslation]);
  expect(host.querySelector('.service-stage--translation [role="combobox"]')).toBeNull();
  if (provider === "alibabaCloud") {
    expect(host.querySelector('.service-stage__restriction [role="status"]')?.textContent).toBe(diagnosticCopy().localDevTranslationLocked);
    expect(host.querySelector('.service-stage__restriction .settings-help-control__description')?.textContent).toBe(diagnosticCopy().localDevTranslationHelp);
  }
  expect(host.querySelector(".stored-credential-reveal, .credential-panel__saved-actions, .credential-form__actions")).toBeNull();
  expect(host.querySelector('button[type="submit"]:not(.service-detail__save-name)')).toBeNull();
  expect(host.querySelector<HTMLInputElement>(`#profile-name-${profile.id}`)?.disabled).toBe(false);
  expect(host.querySelector<HTMLInputElement>(`#profile-name-${profile.id}`)?.readOnly).toBe(true);
  expect(vi.mocked(profileRevealCredential)).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.deleteProfileAPIKey).not.toHaveBeenCalled();
  await render({ ...snapshot, profiles: [{ ...profile, provider, credentialStorage: "localDevFile" }] });
  expect(host.textContent).toContain(diagnosticCopy().localDevUnavailable);
  expect(host.textContent).not.toContain(I18N.settings.credentialUnavailableHelp);
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
it("keeps ordinary translation configuration editable even when the default dev file is unavailable", async () => {
  const preset: ServiceProfile = { ...profile, id: "alibaba-local-dev", name: "Alibaba Cloud · dev", credentialStorage: "localDevFile" };
  const regular: ServiceProfile = { ...profile, id: "regular", credentialStorage: "keychain", credentialState: "present" };
  const snapshot: SettingsSnapshot = { ...settings, credentialStorage: "localDevFile", activeProfileId: preset.id, profiles: [preset, regular] };
  const saved = { ...snapshot, activeProfileId: regular.id, profiles: [preset, { ...regular, textTranslation: "chatMock" as const }] };
  actions.saveProfileCredentials.mockResolvedValue(saved);
  actions.selectProfile.mockResolvedValue(saved);
  await render(snapshot);
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1]!.click());
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
  const node = host.querySelector<HTMLInputElement>(selector)!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(node, value);
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
  boot.initializationStatus = "loading";
  await render();
  expect(host.textContent).toContain(I18N.settings.settingsSnapshotLoading);
  expect(host.textContent).not.toContain(I18N.settings.credentialUnavailable);
  expect(host.querySelector(".service-rows")).toBeNull();
  boot.initializationStatus = "error"; boot.initializationError = "timeout";
  await render();
  expect(host.textContent).toContain(I18N.settings.settingsSnapshotTimeout);
  await click(I18N.settings.retryLoadingSettings);
  expect(boot.init).toHaveBeenCalledOnce();
  expect(testProfileConnection).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("previews a provider and leaves settings unchanged when cancelled in %s", async language => {
  setStoredUiLanguage(language);
  await render();
  await click(I18N.settings.addProfile);
  const options = [...host.querySelectorAll<HTMLButtonElement>(".provider-option")];
  expect(options.map(option => option.dataset.provider)).toEqual(SERVICE_PROVIDERS);
  expect(options.slice(-2).map(option => option.dataset.provider)).toEqual(["customDashScopeASR", "customOpenAIASR"]);
  expect(host.querySelector(".provider-picker small, .provider-picker p")).toBeNull();
  expect(host.querySelector(".provider-picker__heading .settings-help-control__description")?.textContent).toBe(I18N.settings.chooseProviderDescription);
  await previewProvider("customOpenAIASR");
  expect(document.querySelector(".provider-picker__preview h3")?.textContent).toBe(providerDisplayName("customOpenAIASR"));
  expect(document.querySelector(".provider-picker__preview .settings-help-control__description")?.textContent).toBe(`${I18N.settings.customSpeechRequirementsOpenAI}\n${I18N.settings.customSpeechLanguages}`);
  expect(actions.createProfile).not.toHaveBeenCalled();
  await click(I18N.settings.cancel);
  expect(document.querySelector(".provider-picker__preview")).toBeNull();
  expect(host.querySelectorAll(".provider-option")).toHaveLength(SERVICE_PROVIDERS.length);
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

it("requires saving an unsaved translation route before checking it while leaving recognition available", async () => {
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
  expect(textCheck.querySelector(".settings-help-control__description")?.textContent).toBe(I18N.settings.saveTranslationBeforeCheck);
  expect(textCheck.querySelector(".settings-feedback")).toBeNull();
  await act(async () => textCheck.querySelector<HTMLButtonElement>("button.settings-button")!.click());
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith(profile.id, "text");
  await click(I18N.settings.checkSpeechRecognition);
  expect(testProfileConnection).toHaveBeenLastCalledWith(profile.id, "speech");
  expect(testProfileConnection).toHaveBeenCalledTimes(2);
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
  expect(identity.querySelector(".service-detail__description .settings-help-control__description")?.textContent).toContain(I18N.settings.providerOpenAIDescription);
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
  expect(host.querySelector(".service-detail__name button")).toBeNull();
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

it("offers the small name-save action only for a draft change, without touching a credential draft", async () => {
  await render({ ...settings, profiles: [{ ...profile, credentialState: "present" }] });
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector(".service-detail__name button")).toBeNull();
  await click(I18N.settings.replaceCredentials);
  await change('input[type="password"]', "synthetic-replacement");
  await change(".service-detail__name input", "Other name");
  expect(host.querySelector(".service-detail__name button")?.textContent).toBe(I18N.settings.saveName);
  expect(host.querySelector(".service-detail__name button")?.classList.contains("settings-link")).toBe(true);
  expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("synthetic-replacement");
  await change(".service-detail__name input", profile.name);
  expect(host.querySelector(".service-detail__name button")).toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.updateProfile).not.toHaveBeenCalled();
});

it.each(["openAIRealtime", "volcanoEngine", "tencentCloud", "baiduTranslate"] as const)("makes supported %s languages directly selectable in the active service", async (provider) => {
  const snapshot = { ...settings, profiles: [{ ...profile, provider }] };
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const groups = [...host.querySelectorAll("#translation-languages [role=group]")];
  expect(groups.map(group => [...group.querySelectorAll("button span")].map(node => node.textContent))).toEqual([
    sourceLanguagesForSettings(snapshot).map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]),
    targetLanguagesForSettings(snapshot).map(language => TARGET_LANGUAGE_DISPLAY_NAMES[language]),
  ]);
  expect(host.querySelector(".service-language-more")).toBeNull();
  expect(testProfileConnection).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it.each(["deepL", "deepLX", "openAICompatible", "chatMock"] as const)("keeps %s choices scoped to the actual translation route", async (textTranslation) => {
  const snapshot = { ...settings, profiles: [{ ...profile, textTranslation }] };
  await render(snapshot);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelectorAll("#translation-languages [role=combobox]")).toHaveLength(0);
  const groups = [...host.querySelectorAll("#translation-languages [role=group]")];
  expect(groups[1].textContent).toBe(targetLanguagesForSettings(snapshot).map(language => TARGET_LANGUAGE_DISPLAY_NAMES[language]).join(""));
  expect(testProfileConnection).not.toHaveBeenCalled();
});

it("does not edit the active service's global languages from another profile's detail", async () => {
  await render({ ...settings, profiles: [profile, { ...profile, id: "other", name: "Other", credentialState: "present" }] });
  await act(async () => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  expect(host.querySelector("#translation-languages")).toBeNull();
  expect(host.querySelector(".service-detail__language-note")).toBeNull();
  expect(host.querySelector(".service-detail__actions .settings-help-control__description")?.textContent).toBe(I18N.settings.useProfileForLanguages);
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
  await act(async () => { host.querySelector(".service-detail__name")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
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

it("reveals an existing key only on demand and never puts it in the replacement draft", async () => {
  const configured = { ...settings, profiles: [{ ...profile, provider: "openAIRealtime" as const, credentialState: "present" as const }] };
  await render(configured);
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  expect(profileRevealCredential).not.toHaveBeenCalled();
  vi.mocked(profileRevealCredential).mockResolvedValue("synthetic-stored-key");
  await click(I18N.settings.revealSavedCredential);
  expect(profileRevealCredential).toHaveBeenCalledExactlyOnceWith({ profileId: profile.id, field: "apiKey" });
  expect(host.querySelector<HTMLInputElement>(".stored-credential-reveal input")!.value).toBe("synthetic-stored-key");
  expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");
  expect(host.querySelector<HTMLButtonElement>('.credential-form button[type="submit"]')!.disabled).toBe(true);
  await click(I18N.settings.cancel);
  expect(host.querySelector(".stored-credential-reveal input")).toBeNull();
  await click(I18N.settings.replaceCredentials);
  expect(host.querySelector(".stored-credential-reveal input")).toBeNull();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
});

it.each(["credential_service_unavailable", "credential_store_access_denied", "credential_store_unavailable"])("retains unsaved Alibaba edits after %s and subsequent connection checks", async (error) => {
  actions.saveProfileCredentials.mockRejectedValue(error);
  await render();
  await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change('input[type="password"]', "synthetic-asr");
  await chooseCustomTranslation();
  await change('input[type="text"][placeholder="https://example.com/translate"]', "https://example.com/translate");
  await change('input[id$="-token"]', "synthetic-token");
  await submit();
  expect(actions.selectProfile).not.toHaveBeenCalled();
  expect(host.querySelector('.settings-feedback[data-tone="error"]')?.textContent).toBe(profileErrorMessage(error));

  for (const [credential, reason] of [["serviceUnavailable", "credentialsServiceUnavailable"], ["accessDenied", "credentialsAccessDenied"], ["missing", "credentialsMissing"]] as const) {
    vi.mocked(testProfileConnection).mockResolvedValue({ credential, service: "unavailable", reason });
    await click(diagnosticCopy("linux").test);
    expect(testProfileConnection).toHaveBeenLastCalledWith(profile.id, "speech");
    expect(host.querySelector(".connection-check .settings-feedback")?.textContent).toBe(`${diagnosticCopy("linux").unavailable}: ${diagnosticCopy("linux").reasons[reason]}`);
    expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("synthetic-asr");
    expect(host.querySelector<HTMLInputElement>('input[id$="-token"]')!.value).toBe("synthetic-token");
    expect(host.querySelector<HTMLInputElement>('input[placeholder="https://example.com/translate"]')!.value).toBe("https://example.com/translate");
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
  expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith(profile.id, profile.name, { textNetworkProxy: { mode: "direct", url: null } });
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
