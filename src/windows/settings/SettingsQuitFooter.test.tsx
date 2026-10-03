// @vitest-environment jsdom
import { act } from "react";
import { SettingsToastRegion } from "./SettingsToast";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SettingsQuitFooter } from "./SettingsQuitFooter";
import { SettingsView } from "./SettingsView";
import permissions from "../../../src-tauri/permissions/app.toml?raw";

let host: HTMLDivElement;
let root: Root;
const initial = useStore.getState();

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  Element.prototype.scrollTo = vi.fn();
  window.history.replaceState(null, "", "#subtitle-settings");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  useStore.setState(initial, true);
  setStoredUiLanguage("system");
  window.history.replaceState(null, "", window.location.pathname);
  vi.unstubAllGlobals();
});

it.each(["zh", "en", "ja"] as const)("keeps the %s normal quit action outside the sidebar scroll area in every settings category", async (language) => {
  setStoredUiLanguage(language);
  const quit = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, quit }, true);
  await act(async () => root.render(<SettingsView />));
  const button = host.querySelector<HTMLButtonElement>(".settings-quit-button")!;
  expect(button.textContent).toBe(I18N.tray.quit);
  expect(button.classList.contains("settings-sidebar-action")).toBe(true);
  expect(button.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
  expect(host.querySelector(".settings-sidebar-scroll")?.contains(button)).toBe(false);
  for (const category of ["service", "general", "export", "diagnostics"]) {
    await act(async () => host.querySelector<HTMLButtonElement>(`#settings-category-${category}`)!.click());
    expect(host.querySelector(".settings-quit-button")).toBe(button);
  }
  await act(async () => host.querySelector<HTMLButtonElement>(".settings-guide-entry")!.click());
  expect(host.querySelector(".settings-quit-button")).toBe(button);
  await act(async () => button.focus());
  expect(document.activeElement).toBe(button);
  await act(async () => button.click());
  expect(quit).toHaveBeenCalledOnce();
  expect(host.querySelector('[role="dialog"]')).toBeNull();
});

it("shows exit progress immediately, blocks rapid duplicate requests and allows a retry after a sanitized failure", async () => {
  let reject!: (error: Error) => void;
  const onQuit = vi.fn(() => new Promise<void>((_resolve, failure) => { reject = failure; }));
  await act(async () => root.render(<><SettingsQuitFooter onQuit={onQuit} /><SettingsToastRegion /></>));
  const button = host.querySelector<HTMLButtonElement>("button")!;
  await act(async () => { button.click(); button.click(); });
  expect(onQuit).toHaveBeenCalledOnce();
  expect(button.disabled).toBe(true);
  expect(button.getAttribute("aria-busy")).toBe("true");
  expect(button.textContent).toBe(I18N.tray.quitting);
  await act(async () => reject(new Error("private raw failure")));
  expect(button.disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.tray.quitFailed);
  expect(host.textContent).not.toContain("private raw failure");
  await act(async () => button.click());
  expect(onQuit).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await act(async () => reject(new Error("synthetic retry failure")));
});

it("authorizes normal quitting from settings and tray without granting the subtitle overlay an exit action", () => {
  const granted = permissions.split("[[permission]]")
    .filter((entry) => entry.includes('"app_quit"'));
  expect(granted).toHaveLength(2);
  expect(granted.every((entry) => entry.includes('identifier = "app-settings"') || entry.includes('identifier = "app-tray-panel"'))).toBe(true);
});
