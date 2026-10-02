// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { diagnosticCopy, profileErrorMessage } from "../../lib/connectionDiagnostics";
import { I18N, providerDisplayName, setStoredUiLanguage } from "../../lib/i18n";
import { profileRevealCredential, testProfileConnection } from "../../lib/ipc";
import { sourceLanguagesForSettings, targetLanguagesForSettings } from "../../lib/providerCapabilities";
import { SOURCE_LANGUAGE_DISPLAY_NAMES, TARGET_LANGUAGE_DISPLAY_NAMES } from "../../lib/types";
import type { ServiceProfile, SettingsSnapshot } from "../../lib/types";
import { ServiceProfiles } from "./ServiceProfiles";

const actions = vi.hoisted(() => ({
  createProfile: vi.fn(), updateProfile: vi.fn(), selectProfile: vi.fn(),
  saveSettings: vi.fn(), deleteProfile: vi.fn(), saveProfileCredentials: vi.fn(), deleteProfileAPIKey: vi.fn(),
}));
const boot = vi.hoisted(() => ({ initializationStatus: "ready" as "ready" | "loading" | "error", initializationError: null as "timeout" | "unavailable" | null, init: vi.fn() }));
vi.mock("../../lib/store", () => ({ useStore: (select: (state: typeof actions & typeof boot & { settings: { windowsAudioSource: string }; session: { isActive: boolean; isPaused: boolean } }) => unknown) => select({ ...actions, ...boot, settings: { windowsAudioSource: "" }, session: { isActive: false, isPaused: false } }) }));
vi.mock("../../lib/ipc", () => ({ isTauri: false, testProfileConnection: vi.fn(), profileRevealCredential: vi.fn() }));

const profile: ServiceProfile = { id: "synthetic", name: "Alibaba", provider: "alibabaCloud", credentialState: "unavailable" };
const settings: SettingsSnapshot = {
  profiles: [profile], activeProfileId: profile.id, sourceLanguage: "auto", targetLanguage: "zh",
  translationMode: "turbo", fontSize: 18, subtitleColor: "white", subtitleAlignment: "center",
  subtitleDisplayMode: "translation", pulseAnimation: null, pulseStyle: "ribbon", subtitleAnimation: null,
  showSubtitleDividers: false,
  subtitleBlendsWithBackground: false, isOverlayLocked: false, uiLanguage: "en",
  retainSessionHistory: false, recordSessionAudio: false, windowsAudioSource: "", showInDock: false,
  networkProxy: { mode: "system", url: null },
};
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
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
  await act(() => root.unmount()); host.remove();
  setStoredUiLanguage("en"); vi.restoreAllMocks(); vi.unstubAllGlobals();
  vi.useRealTimers();
});
async function render(snapshot = settings, sessionStatusKind: "idle" | "error" = "idle") { await act(() => root.render(<ServiceProfiles settings={snapshot} sessionIsActive={false} sessionStatusKind={sessionStatusKind} />)); }
it.each(["en", "zh", "ja"] as const)("keeps local dev file credentials out of editors and reveal in %s", async (language) => {
  setStoredUiLanguage(language);
  const snapshot: SettingsSnapshot = { ...settings, credentialStorage: "localDevFile", profiles: [{ ...profile, credentialState: "present" }] };
  await render(snapshot);
  expect(host.querySelector(".services-hint")?.textContent).toContain(diagnosticCopy().localDevReadOnly);
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.textContent).toContain(diagnosticCopy().localDevReadOnly);
  expect(host.querySelector('input[type="password"]')).toBeNull();
  expect(host.querySelector(".credential-form")).toBeNull();
  expect(host.querySelector<HTMLInputElement>(`#profile-name-${profile.id}`)?.disabled).toBe(false);
  expect(vi.mocked(profileRevealCredential)).not.toHaveBeenCalled();
  expect(actions.saveProfileCredentials).not.toHaveBeenCalled();
  expect(actions.deleteProfileAPIKey).not.toHaveBeenCalled();
  await render({ ...snapshot, profiles: [profile] });
  expect(host.textContent).toContain(diagnosticCopy().localDevUnavailable);
  expect(host.textContent).not.toContain(I18N.settings.credentialUnavailableHelp);
});
async function click(label: string) {
  const button = [...host.querySelectorAll("button")].find(node => node.textContent === label)!;
  expect(button).toBeTruthy();
  await act(async () => button.click());
}
async function change(selector: string, value: string) {
  const node = host.querySelector<HTMLInputElement>(selector)!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(node, value);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function chooseCustomTranslation() {
  await act(() => {
    const advanced = host.querySelector<HTMLDetailsElement>(".settings-advanced")!;
    advanced.open = true;
    advanced.dispatchEvent(new Event("toggle"));
  });
  await act(() => host.querySelector<HTMLButtonElement>('.settings-advanced [role="combobox"]')!.click());
  const custom = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === I18N.settings.textTranslationCustom)!;
  await act(() => custom.click());
}
async function submit() {
  await act(async () => host.querySelector(".credential-form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
}

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

it("releases a hung connection check after 30 seconds and ignores its late result during a manual retry", async () => {
  vi.useFakeTimers();
  let finishFirst!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  let finishRetry!: (result: Awaited<ReturnType<typeof testProfileConnection>>) => void;
  vi.mocked(testProfileConnection).mockImplementationOnce(() => new Promise((resolve) => { finishFirst = resolve; }))
    .mockImplementationOnce(() => new Promise((resolve) => { finishRetry = resolve; }));
  await render();
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
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
  await render(); await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const button = host.querySelector<HTMLButtonElement>(".connection-check button")!;
  await act(() => { button.click(); button.click(); });
  expect(testProfileConnection).toHaveBeenCalledOnce();
  await act(async () => { finish({ credential: "present", service: "available", reason: null }); });
  await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
  expect(host.querySelector(".connection-check .settings-feedback")?.textContent).toBe(diagnosticCopy().available);
  expect(testProfileConnection).toHaveBeenCalledOnce();
});

it("clears an old-route connection result without replacing an unsaved credential or name draft", async () => {
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  const snapshot = { ...settings, profiles: [{ ...profile, credentialState: "present" as const }] };
  await render(snapshot); await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
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
  await render(); await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
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
  await render(); await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(diagnosticCopy().test); await render({ ...settings, networkProxy: { mode: "direct", url: null } });
  await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
  expect(host.querySelector(".connection-check .settings-feedback")).toBeNull();
  expect(host.querySelector<HTMLButtonElement>(".connection-check button")!.disabled).toBe(false);
  expect(testProfileConnection).toHaveBeenCalledOnce();
});

it.each(["zh", "en", "ja"] as const)("groups the service identity, credential state and existing actions without hiding them in %s", async (language) => {
  setStoredUiLanguage(language);
  await render({ ...settings, profiles: [{ ...profile, provider: "openAIRealtime", credentialState: "present" }] });
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const identity = host.querySelector(".service-detail__identity")!;
  expect(identity.querySelector('.provider-icon[data-provider="openAIRealtime"]')).not.toBeNull();
  expect(identity.querySelector("h2")?.textContent).toBe(profile.name);
  expect(identity.querySelector(".service-detail__title .credential-badge")?.textContent).toBe(I18N.settings.credentialPresent);
  expect(identity.querySelector(".service-detail__title .profile-active-badge")?.textContent).toBe(I18N.settings.activeProfile);
  expect(identity.querySelector(".service-detail__copy > p")?.textContent).toContain(I18N.settings.providerOpenAIDescription);
  expect(identity.querySelector(".service-language-support")).toBeNull();
  expect(host.querySelector(".service-detail__configuration #translation-languages")).not.toBeNull();
  const connection = host.querySelector(".service-detail__connection")!;
  expect(connection.querySelector(".connection-check")).not.toBeNull();
  expect(connection.querySelector(".credential-panel__saved-actions")?.textContent).toContain(I18N.settings.replaceCredentials);
  await click(I18N.settings.replaceCredentials);
  expect(connection.querySelector('input[type="password"]')).not.toBeNull();
  expect(connection.querySelector(".credential-form .settings-advanced")).toBeNull();
  expect(connection.querySelector(".credential-form + .settings-advanced")).not.toBeNull();
  expect(host.querySelector(".service-detail__name label")?.textContent).toBe(I18N.settings.profileName);
  expect(host.querySelector(".service-detail__name")?.closest("details")).toBeNull();
  expect(host.querySelector(".service-detail__name button")).toBeNull();
  expect(host.querySelector(".service-detail__configuration .credential-panel")).not.toBeNull();
  expect(host.querySelector(".service-detail__actions")?.textContent).toContain(I18N.settings.deleteProfile);
  expect(host.querySelector(".settings-advanced summary")?.textContent).toBe(I18N.settings.advancedTranslation);
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
  expect(rows[0].querySelector(".service-row__copy small")).toBeNull();
  expect(rows[1].querySelector(".service-row__copy strong")?.textContent).toBe("Custom configuration");
  expect(rows[1].querySelector(".service-row__copy small")?.textContent).toBe(providerName);
  expect(rows.every((row) => !!row.querySelector(".service-row__main") && !!row.querySelector(".service-row__edit"))).toBe(true);
});

it("offers the small name-save action only for a draft change, without touching a credential draft", async () => {
  await render({ ...settings, profiles: [{ ...profile, credentialState: "present" }] });
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
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
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  const groups = [...host.querySelectorAll("#translation-languages [role=group]")];
  expect(groups.map(group => [...group.querySelectorAll("button span")].map(node => node.textContent))).toEqual([
    sourceLanguagesForSettings(snapshot).map(language => SOURCE_LANGUAGE_DISPLAY_NAMES[language]),
    targetLanguagesForSettings(snapshot).map(language => TARGET_LANGUAGE_DISPLAY_NAMES[language]),
  ]);
  expect(host.querySelector(".service-language-more")).toBeNull();
  expect(testProfileConnection).not.toHaveBeenCalled();
  expect(profileRevealCredential).not.toHaveBeenCalled();
});

it.each(["deepL", "deepLX", "openAICompatible"] as const)("keeps %s choices scoped to the actual translation route", async (textTranslation) => {
  const snapshot = { ...settings, profiles: [{ ...profile, textTranslation }] };
  await render(snapshot);
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelectorAll("#translation-languages [role=combobox]")).toHaveLength(0);
  const groups = [...host.querySelectorAll("#translation-languages [role=group]")];
  expect(groups[1].textContent).toBe(targetLanguagesForSettings(snapshot).map(language => TARGET_LANGUAGE_DISPLAY_NAMES[language]).join(""));
  expect(testProfileConnection).not.toHaveBeenCalled();
});

it("does not edit the active service's global languages from another profile's detail", async () => {
  await render({ ...settings, profiles: [profile, { ...profile, id: "other", name: "Other", credentialState: "present" }] });
  await act(() => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  expect(host.querySelector("#translation-languages")).toBeNull();
  expect(host.textContent).toContain(I18N.settings.useProfileForLanguages);
  expect(host.querySelector(".service-detail__actions")?.textContent).toContain(I18N.settings.useProfile);
  expect(actions.saveSettings).not.toHaveBeenCalled();
});

it("keeps profile rename and delete actions reachable without opening another panel", async () => {
  const other = { ...profile, id: "other", name: "Other", credentialState: "present" as const };
  const snapshot = { ...settings, profiles: [{ ...profile, credentialState: "present" as const }, other] };
  actions.updateProfile.mockResolvedValue(snapshot);
  await render(snapshot);
  await act(() => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  await change(".service-detail__name input", "Renamed");
  await act(async () => { host.querySelector(".service-detail__name")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
  expect(actions.updateProfile).toHaveBeenCalledExactlyOnceWith("other", "Renamed");
  const management = host.querySelector(".service-detail__actions")!;
  expect(management.textContent).toContain(I18N.settings.useProfile);
  expect(management.textContent).toContain(I18N.settings.deleteProfile);
  await click(I18N.settings.deleteProfile);
  expect(host.querySelector(".destructive-confirmation")?.textContent).toContain(I18N.settings.deleteProfileConfirm("Other"));
  expect(actions.deleteProfile).not.toHaveBeenCalled();
});

it("reveals an existing key only on demand and never puts it in the replacement draft", async () => {
  const configured = { ...settings, profiles: [{ ...profile, provider: "openAIRealtime" as const, credentialState: "present" as const }] };
  await render(configured);
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
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
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
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
    expect(testProfileConnection).toHaveBeenLastCalledWith(profile.id);
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
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  expect(host.querySelector('.credential-unavailable[role="status"]')?.textContent).toBe(diagnosticCopy("linux").storage);
  expect(host.textContent).toContain(I18N.settings.credentialUnavailable);
});

it("replaces generic storage guidance with the failed save error", async () => {
  await render({ ...settings, profiles: [{ ...profile, provider: "openAIRealtime" }] });
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await change('input[type="password"]', "synthetic-key");
  actions.saveProfileCredentials.mockRejectedValueOnce("credential_service_unavailable");
  await act(async () => host.querySelector<HTMLFormElement>(".credential-form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(host.textContent).toContain(profileErrorMessage("credential_service_unavailable"));
  expect(host.querySelector('.credential-unavailable[role="status"]')).toBeNull();
});

it.each(["alibabaCloud", "openAIRealtime"] as const)("keeps an unsaved %s key visible when a connection check recovers stored credentials", async (provider) => {
  await render({ ...settings, profiles: [{ ...profile, provider }] });
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
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
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
  await click(I18N.settings.replaceCredentials);
  await change('input[type="password"]', "synthetic-unsaved-replacement");
  await click(I18N.settings.deleteCredentials);
  actions.deleteProfileAPIKey.mockRejectedValue("credential_store_access_denied");
  await click(I18N.settings.confirmDelete);
  expect(actions.deleteProfileAPIKey).toHaveBeenCalledExactlyOnceWith(profile.id);
  expect(host.querySelector('input[type="password"]')).toBeNull();
  await click(I18N.settings.replaceCredentials);
  expect(host.querySelector<HTMLInputElement>('input[type="password"]')!.value).toBe("");
  expect(host.querySelector('.settings-feedback[data-tone="error"]')?.textContent).toBe(profileErrorMessage("credential_store_access_denied"));
});

it("clears an active profile's previous check on session failure and discards its in-flight result", async () => {
  const configured = { ...settings, profiles: [{ ...profile, credentialState: "present" as const }] };
  await render(configured);
  await act(() => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
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
  await act(() => host.querySelectorAll<HTMLButtonElement>(".service-row__edit")[1].click());
  vi.mocked(testProfileConnection).mockResolvedValue({ credential: "present", service: "available", reason: null });
  await click(diagnosticCopy().test);
  await render(configured, "error");
  expect(host.querySelector('.connection-check [data-tone="success"]')?.textContent).toBe(diagnosticCopy().available);
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith("other-synthetic");
});
