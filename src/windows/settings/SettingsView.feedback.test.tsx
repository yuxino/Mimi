// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SettingsView } from "./SettingsView";

vi.mock("./ServiceProfiles", () => ({ ServiceProfiles: () => null }));
vi.mock("./SoftwareUpdate", () => ({ SoftwareUpdate: () => null }));
vi.mock("./SessionExport", () => ({ SessionExport: () => null }));
vi.mock("./SupportDiagnostics", () => ({ SupportDiagnostics: () => null }));
const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
const save = vi.fn(initial.saveSettings);
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  Element.prototype.scrollTo = vi.fn();
  save.mockReset().mockImplementation(initial.saveSettings);
  setStoredUiLanguage("en"); window.history.replaceState(null, "", "#subtitle-settings");
  useStore.setState({ ...initial, initializationStatus: "ready", saveSettings: save }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); useStore.setState(initial, true);
  setStoredUiLanguage("system"); window.history.replaceState(null, "", window.location.pathname);
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});
async function render() { await act(async () => root.render(<SettingsView />)); }
async function toggle() { await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${I18N.settings.subtitleDividers}"][role="switch"]`)!.click()); }

it("saves instant preferences quietly, reports a sanitized failure as a toast and allows retry", async () => {
  save.mockRejectedValueOnce(new Error("synthetic-private-preference-error"));
  await render(); const before = useStore.getState().settings.showSubtitleDividers;
  await toggle();
  expect(host.querySelector('.settings-toast[role="alert"]')?.textContent).toBe(I18N.settings.settingSaveFailed(I18N.settings.subtitleDividers));
  expect(host.querySelector(".settings-category-panel .settings-feedback")).toBeNull();
  expect(host.textContent).not.toContain("synthetic-private-preference-error");
  expect(useStore.getState().settings.showSubtitleDividers).toBe(before);
  await toggle();
  expect(useStore.getState().settings.showSubtitleDividers).toBe(!before);
  expect(host.querySelector(".settings-toast")).toBeNull();
});

it("does not revive an autosave failure after category navigation", async () => {
  let fail!: (error: Error) => void;
  save.mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  await render(); await toggle();
  await act(async () => host.querySelector<HTMLButtonElement>("#settings-category-general")!.click());
  await act(async () => fail(new Error("late synthetic error")));
  expect(host.querySelector(".settings-toast")).toBeNull();
});

it("reports overlay-lock failure without hiding it in an empty catch", async () => {
  useStore.setState({ setOverlayLocked: vi.fn().mockRejectedValue(new Error("synthetic")) });
  await render();
  await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${I18N.settings.lockPosition}"][role="switch"]`)!.click());
  expect(host.querySelector(".settings-toast")?.textContent).toBe(I18N.settings.settingSaveFailed(I18N.settings.lockPosition));
});

it("preserves the session appearance choice and explains that storage failed", async () => {
  await render();
  await act(async () => host.querySelector<HTMLButtonElement>("#settings-category-general")!.click());
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("private storage error"); });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>(".appearance-option")].find(button => button.textContent === I18N.settings.themeDark)!.click());
  expect(host.querySelector(".settings-console--dark")).not.toBeNull();
  expect(host.querySelector(".settings-toast")?.textContent).toBe(I18N.settings.settingSaveFailed(I18N.settings.appearance));
  expect(host.textContent).not.toContain("private storage error");
  vi.restoreAllMocks();
});
