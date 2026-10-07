// @vitest-environment jsdom
import { act, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { getAppleSpeechSupport } from "../../lib/ipc";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import type { AppleSpeechSupport, SettingsSnapshot, SourceLanguage } from "../../lib/types";
import { useAppleSpeechSupport } from "./useAppleSpeechSupport";
import { SettingsToastRegion } from "./SettingsToast";
import { useSettingsToast } from "./useSettingsToast";

vi.mock("../../lib/ipc", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  getAppleSpeechSupport: vi.fn(),
}));

const initial = useStore.getState();
const ready: AppleSpeechSupport = { available: true, languages: [
  { sourceLanguage: "en", locale: "en-US", installed: true },
  { sourceLanguage: "ja", locale: "ja-JP", installed: true },
] };
const missing: AppleSpeechSupport = { ...ready, languages: ready.languages.map(language => ({ ...language, installed: false, status: "supported" as const })) };
let host: HTMLDivElement, root: Root;
let current: ReturnType<typeof useAppleSpeechSupport>;
let beginNotice: ReturnType<typeof useSettingsToast>["beginToast"];
function settings(sources: readonly SourceLanguage[] = ["en", "ja"], revision?: number): SettingsSnapshot {
  return { ...initial.settings, activeProfileId: "apple", sourceLanguage: "en", targetLanguage: "original",
    profiles: [{ id: "apple", name: "Apple Speech", provider: "appleSpeech", credentialState: "present" }],
    languageCapabilities: { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original",
      sourceLanguages: sources, targetLanguages: ["original"], appleSpeechSupportRevision: revision } };
}
function deferred() {
  let resolve!: (support: AppleSpeechSupport) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<AppleSpeechSupport>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function Harness({ visible }: { visible: boolean }) {
  const support = useAppleSpeechSupport(visible);
  const { beginToast } = useSettingsToast();
  useEffect(() => { current = support; }, [support]);
  useEffect(() => { beginNotice = beginToast; }, [beginToast]);
  return <SettingsToastRegion />;
}
const render = async (visible = true) => { await act(async () => root.render(<Harness visible={visible} />)); };
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  vi.mocked(getAppleSpeechSupport).mockReset().mockResolvedValue(ready);
  useStore.setState({ ...initial, settings: settings() }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});

it("requeries resources when a visible native ready list changes and hides stale readiness while loading", async () => {
  await render();
  expect(current.support).toEqual(ready);
  const refreshed = deferred();
  vi.mocked(getAppleSpeechSupport).mockReturnValueOnce(refreshed.promise);
  await act(async () => useStore.setState({ settings: settings([]) }));
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(2);
  expect(current.loading).toBe(true);
  await act(async () => refreshed.resolve(missing));
  expect(current.support).toEqual(missing);
  expect(current.loading).toBe(false);
  expect(useStore.getState().settings.sourceLanguage).toBe("en");
});

it("does not query again for its own equivalent settings broadcast or reordered duplicate source codes", async () => {
  vi.mocked(getAppleSpeechSupport).mockImplementation(async () => {
    useStore.setState({ settings: settings() });
    return ready;
  });
  await render();
  await act(async () => useStore.setState({ settings: { ...settings(["ja", "en", "ja"]), fontSize: 19 } }));
  expect(getAppleSpeechSupport).toHaveBeenCalledOnce();
});

it("converges after the first query publishes a newly available native signature", async () => {
  useStore.setState({ settings: { ...settings(), languageCapabilities: undefined } });
  vi.mocked(getAppleSpeechSupport).mockImplementation(async () => {
    useStore.setState({ settings: settings([]) });
    return missing;
  });
  await render();
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(2);
  expect(current.support).toEqual(missing);
  expect(current.loading).toBe(false);
});

it("refreshes a filtered-out language's resource state when the complete revision changes", async () => {
  const before = settings(["en"], 1);
  before.profiles = [{ ...before.profiles[0], textTranslation: "deepL" }];
  before.targetLanguage = "zh";
  before.languageCapabilities = { ...before.languageCapabilities!, textTranslation: "deepL", targetLanguage: "zh" };
  const frenchReady: AppleSpeechSupport = { available: true, languages: [ready.languages[0], { sourceLanguage: "fr", locale: "fr-FR", installed: true }] };
  useStore.setState({ settings: before });
  vi.mocked(getAppleSpeechSupport).mockResolvedValueOnce(frenchReady);
  await render();
  const after = { ...frenchReady, languages: frenchReady.languages.map(language => ({ ...language, installed: language.sourceLanguage === "en", status: language.sourceLanguage === "en" ? "installed" as const : "supported" as const })) };
  vi.mocked(getAppleSpeechSupport).mockResolvedValueOnce(after);
  await act(async () => useStore.setState({ settings: { ...before, languageCapabilities: { ...before.languageCapabilities!, appleSpeechSupportRevision: 2 } } }));
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(2);
  expect(current.support).toEqual(after);
  expect(useStore.getState().settings.languageCapabilities?.sourceLanguages).toEqual(["en"]);
});

it("refreshes device availability when the ready list stays empty", async () => {
  useStore.setState({ settings: settings([], 1) });
  vi.mocked(getAppleSpeechSupport).mockResolvedValueOnce(missing);
  await render();
  expect(current.support?.available).toBe(true);
  vi.mocked(getAppleSpeechSupport).mockResolvedValueOnce({ available: false, languages: [] });
  await act(async () => useStore.setState({ settings: settings([], 2) }));
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(2);
  expect(current.support).toEqual({ available: false, languages: [] });
});

it("does not query again for new snapshot objects with the same complete resource revision", async () => {
  useStore.setState({ settings: settings(["en"], 3) });
  vi.mocked(getAppleSpeechSupport).mockImplementation(async () => {
    useStore.setState({ settings: settings(["en"], 3) });
    return ready;
  });
  await render();
  await act(async () => useStore.setState({ settings: settings(["en"], 3) }));
  expect(getAppleSpeechSupport).toHaveBeenCalledOnce();
});

it.each([false, true])("preserves another operation's toast through an automatic resource refresh when it rejects=%s", async fails => {
  await render();
  const notify = beginNotice();
  const query = deferred();
  vi.mocked(getAppleSpeechSupport).mockReturnValueOnce(query.promise);
  await act(async () => useStore.setState({ settings: settings([]) }));
  await act(async () => notify("Synthetic operation feedback", true));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe("Synthetic operation feedback");
  await act(async () => { if (fails) query.reject(new Error("private-native-error")); else query.resolve(missing); });
  expect(host.querySelector('[role="alert"]')?.textContent).toBe("Synthetic operation feedback");
  expect(current.failed).toBe(fails);
  expect(host.textContent).not.toContain("private-native-error");
});

it("shows a failure toast only for an explicit resource retry", async () => {
  vi.mocked(getAppleSpeechSupport).mockRejectedValue(new Error("private-native-error"));
  await render();
  expect(current.failed).toBe(true);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await act(async () => current.refresh());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.appleSpeechLoadFailed);
  expect(host.textContent).not.toContain("private-native-error");
});

it("does not query while hidden and queries the latest native readiness when shown again", async () => {
  await render(false);
  await act(async () => useStore.setState({ settings: settings([]) }));
  expect(getAppleSpeechSupport).not.toHaveBeenCalled();
  vi.mocked(getAppleSpeechSupport).mockResolvedValueOnce(missing);
  await render();
  expect(getAppleSpeechSupport).toHaveBeenCalledOnce();
  expect(current.support).toEqual(missing);
  await render(false);
  await act(async () => useStore.setState({ settings: settings(["en"]) }));
  expect(getAppleSpeechSupport).toHaveBeenCalledOnce();
  await render();
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(2);
  expect(current.support).toEqual(ready);
});

it("queries after a matching route change without inferring resource installation from the filtered list", async () => {
  await render();
  const next = settings([]);
  next.profiles = [{ ...next.profiles[0], textTranslation: "deepL" }];
  next.targetLanguage = "zh";
  next.languageCapabilities = { ...next.languageCapabilities!, textTranslation: "deepL", targetLanguage: "zh" };
  await act(async () => useStore.setState({ settings: next }));
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(2);
  expect(current.support).toEqual(ready);
});

it("waits for a matching Apple capability stamp instead of reacting to changes in an expired one", async () => {
  useStore.setState({ settings: { ...settings(), languageCapabilities: { ...settings().languageCapabilities!, profileId: "old" } } });
  await render();
  await act(async () => useStore.setState({ settings: { ...settings([]), languageCapabilities: { ...settings([]).languageCapabilities!, profileId: "old" } } }));
  expect(getAppleSpeechSupport).toHaveBeenCalledOnce();
  await act(async () => useStore.setState({ settings: settings([]) }));
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(2);
});

it.each([false, true])("keeps explicit preparation results when an older refresh later rejects=%s", async fails => {
  await render();
  const prepared = current.update;
  const old = deferred();
  vi.mocked(getAppleSpeechSupport).mockReturnValueOnce(old.promise);
  await act(async () => useStore.setState({ settings: settings([]) }));
  await act(async () => prepared(ready));
  await act(async () => { if (fails) old.reject(new Error("private-native-error")); else old.resolve(missing); });
  expect(current.support).toEqual(ready);
  expect(current.loading).toBe(false);
  expect(current.failed).toBe(false);
});

it("discards hidden query results and refreshes after a preparation completes while hidden", async () => {
  const old = deferred();
  vi.mocked(getAppleSpeechSupport).mockReturnValueOnce(old.promise);
  await render();
  await render(false);
  await act(async () => current.update(ready));
  await act(async () => old.resolve(missing));
  expect(current.support).toEqual(ready);
  vi.mocked(getAppleSpeechSupport).mockResolvedValueOnce(missing);
  await render();
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(2);
  expect(current.support).toEqual(missing);
});
