// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage, UI_LANGUAGES } from "../../lib/i18n";
import { appOpenAudioPrivacySettings, getAppleSpeechSupport, prepareAppleSpeechLanguage } from "../../lib/ipc";
import { QuickStartGuide } from "./QuickStartGuide";

vi.mock("../../lib/ipc", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  appOpenAudioPrivacySettings: vi.fn(),
  getAppleSpeechSupport: vi.fn().mockResolvedValue({ available: false, languages: [] }),
  prepareAppleSpeechLanguage: vi.fn(),
}));

let host: HTMLDivElement;
let root: Root;
const onConfigureService = vi.fn();
const onOpenSubtitles = vi.fn();

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(getAppleSpeechSupport).mockReset().mockResolvedValue({ available: false, languages: [] });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  setStoredUiLanguage("system");
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it.each([
  ["Macintosh", true, true],
  ["Macintosh", false, false],
  ["Windows NT 10.0", true, false],
  ["Linux", true, false],
] as const)("offers free Apple recognition only with confirmed Mac support (%s, available=%s)", async (userAgent, available, shown) => {
  setStoredUiLanguage("en");
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(userAgent);
  vi.mocked(getAppleSpeechSupport).mockResolvedValue({ available, languages: [] });
  await mount();
  expect(panel("setup").textContent?.includes(I18N.settings.quickStartAppleBody)).toBe(shown);
  expect(getAppleSpeechSupport).toHaveBeenCalledTimes(userAgent === "Macintosh" ? 1 : 0);
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
  expect(onConfigureService).not.toHaveBeenCalled();
});

it("keeps Apple guidance hidden while support is unconfirmed or the query fails", async () => {
  setStoredUiLanguage("en");
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Macintosh");
  let reject!: (reason: Error) => void;
  vi.mocked(getAppleSpeechSupport).mockImplementationOnce(() => new Promise((_resolve, fail) => { reject = fail; }));
  await mount();
  expect(panel("setup").textContent).not.toContain(I18N.settings.quickStartAppleBody);
  await act(async () => reject(new Error("private-support-query-failure")));
  expect(panel("setup").textContent).not.toContain(I18N.settings.quickStartAppleBody);
  expect(host.textContent).not.toContain("private-support-query-failure");
  expect(prepareAppleSpeechLanguage).not.toHaveBeenCalled();
});

async function mount() {
  await act(async () => root.render(<QuickStartGuide onConfigureService={onConfigureService} onOpenSubtitles={onOpenSubtitles} />));
}

function tab(id: string) {
  return host.querySelector<HTMLButtonElement>(`#quick-start-tab-${id}`)!;
}

function panel(id: string) {
  return host.querySelector<HTMLDivElement>(`#quick-start-panel-${id}`)!;
}

it.each(UI_LANGUAGES)("separates setup and common actions without invoking native actions in %s", async language => {
  setStoredUiLanguage(language);
  await mount();
  expect(tab("setup").textContent).toBe(I18N.settings.quickStartSetupTab);
  expect(tab("operations").textContent).toBe(I18N.settings.quickStartOperationsTitle);
  expect(tab("setup").getAttribute("aria-selected")).toBe("true");
  expect(panel("setup").hidden).toBe(false);
  expect(panel("operations").hidden).toBe(true);
  await act(async () => tab("operations").click());
  expect(panel("setup").hidden).toBe(true);
  expect(panel("operations").hidden).toBe(false);
  expect(tab("operations").getAttribute("aria-controls")).toBe(panel("operations").id);
  expect(panel("operations").getAttribute("aria-labelledby")).toBe(tab("operations").id);
  await act(async () => tab("setup").click());
  expect(panel("setup").hidden).toBe(false);
  expect(panel("operations").hidden).toBe(true);
  expect(onConfigureService).not.toHaveBeenCalled();
  expect(onOpenSubtitles).not.toHaveBeenCalled();
  expect(appOpenAudioPrivacySettings).not.toHaveBeenCalled();
});

it("moves focus and selection with arrows, Home and End, preserving modified shortcuts", async () => {
  setStoredUiLanguage("en");
  await mount();
  tab("setup").focus();
  for (const [key, next] of [["ArrowRight", "operations"], ["ArrowRight", "setup"], ["ArrowLeft", "operations"], ["Home", "setup"], ["End", "operations"]]) {
    await act(async () => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true })));
    expect(document.activeElement).toBe(tab(next));
    expect(tab(next).tabIndex).toBe(0);
    expect(tab(next === "setup" ? "operations" : "setup").tabIndex).toBe(-1);
    expect(panel(next).hidden).toBe(false);
  }
  await act(async () => tab("operations").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true, bubbles: true })));
  expect(document.activeElement).toBe(tab("operations"));
  expect(panel("operations").hidden).toBe(false);
});
