// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { getAppleTranslationStatus, prepareAppleTranslationLanguages } from "../../lib/ipc";
import type { AppleTranslationStatus } from "../../lib/types";
import { AppleTranslationSettings } from "./AppleTranslationSettings";
import { SettingsToastRegion } from "./SettingsToast";

vi.mock("../../lib/ipc", async original => ({ ...await original<typeof import("../../lib/ipc")>(), getAppleTranslationStatus: vi.fn(), prepareAppleTranslationLanguages: vi.fn() }));
let root: Root, host: HTMLDivElement;
let props: ComponentProps<typeof AppleTranslationSettings>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  vi.mocked(getAppleTranslationStatus).mockReset().mockResolvedValue("supported");
  vi.mocked(prepareAppleTranslationLanguages).mockReset().mockResolvedValue("installed");
  props = { profileId: "apple-test", sourceLanguage: "en", targetLanguage: "ja", support: { available: true, sourceLanguages: ["en", "ja"], targetLanguages: ["en", "ja"] },
    loading: false, failed: false, disabled: false, onRetry: vi.fn().mockResolvedValue(undefined), onBusyChange: vi.fn(), onPrepared: vi.fn() };
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
const render = async (next: Partial<typeof props> = {}) => { props = { ...props, ...next }; await act(async () => root.render(<><AppleTranslationSettings {...props} /><SettingsToastRegion /></>)); };
const button = (text: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === text);
function deferred() { let resolve!: (value: AppleTranslationStatus) => void; let reject!: (reason: Error) => void; const promise = new Promise<AppleTranslationStatus>((done, fail) => { resolve = done; reject = fail; }); return { promise, resolve, reject }; }

it.each(["zh", "en", "ja"] as const)("shows the real pair and explicit preparation action in %s", async language => {
  setStoredUiLanguage(language);
  await render();
  expect(getAppleTranslationStatus).toHaveBeenCalledExactlyOnceWith("en", "ja");
  expect(button(I18N.settings.appleTranslationPrepare)).toBeDefined();
  expect(host.querySelector(".apple-speech-resource-status")?.textContent).toContain(I18N.settings.appleTranslationNeedsSetup);
  expect(host.querySelector(".apple-speech-resource-status")?.textContent).not.toContain(I18N.settings.appleSpeechNotInstalled);
  expect(prepareAppleTranslationLanguages).not.toHaveBeenCalled();
});

it("requires one explicit download action and invalidates checks only after its result", async () => {
  const request = deferred();
  vi.mocked(prepareAppleTranslationLanguages).mockReturnValue(request.promise);
  await render();
  const download = button(I18N.settings.appleTranslationPrepare)!;
  await act(async () => { download.click(); download.click(); });
  expect(prepareAppleTranslationLanguages).toHaveBeenCalledExactlyOnceWith("en", "ja");
  expect(download.disabled).toBe(true);
  expect(props.onBusyChange).toHaveBeenCalledWith(true);
  expect(props.onPrepared).not.toHaveBeenCalled();
  await act(async () => request.resolve("installed"));
  expect(button(I18N.settings.appleTranslationPrepare)).toBeUndefined();
  expect(host.querySelector(".apple-speech-resource-status")?.textContent).toContain(I18N.settings.appleTranslationReady);
  expect(props.onPrepared).toHaveBeenCalledOnce();
  expect(props.onBusyChange).toHaveBeenLastCalledWith(false);
});

it.each(["apple_translation_cancelled", "private-native-error"])("keeps %s retryable without automatic retries or raw errors", async error => {
  vi.mocked(prepareAppleTranslationLanguages).mockRejectedValueOnce(new Error(error));
  await render();
  await act(async () => button(I18N.settings.appleTranslationPrepare)!.click());
  expect(host.textContent).toContain(error === "apple_translation_cancelled" ? I18N.settings.appleTranslationCancelled : I18N.settings.appleTranslationPrepareFailed);
  expect(host.textContent).not.toContain(error);
  expect(button(I18N.settings.appleTranslationPrepare)?.disabled).toBe(false);
  expect(prepareAppleTranslationLanguages).toHaveBeenCalledOnce();
  expect(props.onPrepared).not.toHaveBeenCalled();
});

it.each([
  { sourceLanguage: "auto" as const, targetLanguage: "ja" as const, message: "appleTranslationChooseSource" as const },
  { sourceLanguage: "en" as const, targetLanguage: "original" as const, message: "appleTranslationChooseTarget" as const },
  { sourceLanguage: "en" as const, targetLanguage: "en" as const, message: "appleTranslationSameLanguage" as const },
  { sourceLanguage: "fr" as const, targetLanguage: "ja" as const, message: "appleTranslationUnsupported" as const },
])("does not query or download for $sourceLanguage → $targetLanguage", async test => {
  await render(test);
  expect(host.textContent).toContain(I18N.settings[test.message]);
  expect(getAppleTranslationStatus).not.toHaveBeenCalled();
  expect(prepareAppleTranslationLanguages).not.toHaveBeenCalled();
  expect(button(I18N.settings.appleTranslationPrepare)).toBeUndefined();
});

it.each(["ja", "en"] as const)("never advertises an unsupported device as ready for en → %s", async targetLanguage => {
  await render({ targetLanguage, support: { available: false, sourceLanguages: [], targetLanguages: [] } });
  expect(host.textContent).toContain(I18N.settings.appleTranslationUnavailable);
  expect(host.textContent).not.toContain(I18N.settings.appleTranslationSameLanguage);
  expect(getAppleTranslationStatus).not.toHaveBeenCalled();
  expect(button(I18N.settings.appleTranslationPrepare)).toBeUndefined();
});

it("waits for the pair check and keeps failed status checks retryable", async () => {
  const query = deferred();
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(query.promise);
  await render();
  expect(host.textContent).toContain(I18N.settings.appleTranslationChecking);
  expect(button(I18N.settings.appleTranslationPrepare)).toBeUndefined();
  await act(async () => query.reject(new Error("private-status-failure")));
  expect(host.textContent).toContain(I18N.settings.appleTranslationStatusFailed);
  await act(async () => button(I18N.settings.retryLoadingSettings)!.click());
  expect(getAppleTranslationStatus).toHaveBeenCalledTimes(2);
  expect(button(I18N.settings.appleTranslationPrepare)).toBeDefined();
});

it("discards old-pair status and preparation results after a source change", async () => {
  const download = deferred(), nextStatus = deferred();
  vi.mocked(prepareAppleTranslationLanguages).mockReturnValueOnce(download.promise);
  await render();
  await act(async () => button(I18N.settings.appleTranslationPrepare)!.click());
  vi.mocked(getAppleTranslationStatus).mockReturnValueOnce(nextStatus.promise);
  await render({ sourceLanguage: "ja", targetLanguage: "en" });
  await act(async () => download.resolve("installed"));
  expect(props.onPrepared).not.toHaveBeenCalled();
  expect(host.textContent).not.toContain(I18N.settings.appleTranslationPrepared);
  expect(host.textContent).toContain(I18N.settings.appleTranslationChecking);
  await act(async () => nextStatus.resolve("supported"));
  expect(button(I18N.settings.appleTranslationPrepare)?.disabled).toBe(false);
});

it("clears old-pair download errors when the pair changes", async () => {
  vi.mocked(prepareAppleTranslationLanguages).mockRejectedValueOnce(new Error("private-failure"));
  await render();
  await act(async () => button(I18N.settings.appleTranslationPrepare)!.click());
  expect(host.querySelector(".apple-translation-settings")?.textContent).toContain(I18N.settings.appleTranslationPrepareFailed);
  await render({ sourceLanguage: "ja", targetLanguage: "en" });
  expect(host.querySelector(".apple-translation-settings")?.textContent).not.toContain(I18N.settings.appleTranslationPrepareFailed);
});

it.each(["disabled", "requiresStop"] as const)("honors the %s preparation guard", async guard => {
  await render({ [guard]: true });
  expect(button(I18N.settings.appleTranslationPrepare)?.disabled).toBe(true);
  expect(prepareAppleTranslationLanguages).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("does not ask to stop an active session for ready translation packs in %s", async language => {
  setStoredUiLanguage(language);
  vi.mocked(getAppleTranslationStatus).mockResolvedValue("installed");
  await render({ requiresStop: true });
  expect(host.querySelector(".apple-speech-resource-status")?.textContent).toContain(I18N.settings.appleTranslationReady);
  expect(host.textContent).not.toContain(I18N.settings.languageChangeRequiresStop);
  expect(host.textContent).not.toContain(I18N.settings.appleLanguagePreparationRequiresStop);
  expect(button(I18N.settings.appleTranslationPrepare)).toBeUndefined();
  expect(prepareAppleTranslationLanguages).not.toHaveBeenCalled();
});

it.each(["zh", "en", "ja"] as const)("explains only the preparation stop requirement for missing packs in an active session in %s", async language => {
  setStoredUiLanguage(language);
  await render({ requiresStop: true });
  const download = button(I18N.settings.appleTranslationPrepare)!;
  expect(download.disabled).toBe(true);
  expect(host.querySelector(".settings-feedback")?.textContent).toBe(I18N.settings.appleLanguagePreparationRequiresStop);
  expect(host.textContent).not.toContain(I18N.settings.languageChangeRequiresStop);
  await act(async () => download.click());
  expect(prepareAppleTranslationLanguages).not.toHaveBeenCalled();
  await render({ requiresStop: false });
  expect(button(I18N.settings.appleTranslationPrepare)?.disabled).toBe(false);
  expect(host.textContent).not.toContain(I18N.settings.appleLanguagePreparationRequiresStop);
});

it("releases the parent busy guard after unmount without showing late success", async () => {
  const request = deferred();
  vi.mocked(prepareAppleTranslationLanguages).mockReturnValueOnce(request.promise);
  await render();
  await act(async () => button(I18N.settings.appleTranslationPrepare)!.click());
  await act(async () => root.render(<SettingsToastRegion />));
  expect(props.onBusyChange).toHaveBeenLastCalledWith(false);
  await act(async () => request.resolve("installed"));
  expect(props.onPrepared).not.toHaveBeenCalled();
  expect(host.textContent).not.toContain(I18N.settings.appleTranslationPrepared);
});
