// @vitest-environment jsdom
import { act } from "react";
import { SettingsToastRegion } from "./SettingsToast";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DockPreference } from "./DockPreference";
import { useStore } from "../../lib/store";

let host: HTMLDivElement;
let root: Root;
const initial = useStore.getState();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (Macintosh; Intel Mac OS X)");
  vi.spyOn(navigator, "platform", "get").mockReturnValue("MacIntel");
  useStore.setState(initial, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); useStore.setState(initial, true); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
async function mount() { await act(async () => root.render(<><DockPreference /><SettingsToastRegion /></>)); }

it("hides the control on Windows and Linux", async () => {
  for (const platform of ["Win32", "Linux x86_64"]) {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 " + platform);
    vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
    await mount(); expect(host.querySelector('[role="switch"]')).toBeNull();
  }
});
it("shows in the Dock by default and saves either toggle direction through the shared store", async () => {
  await mount(); expect(host.querySelector('[role="switch"]')?.getAttribute("aria-checked")).toBe("true");
  await act(async () => host.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
  expect(useStore.getState().settings.showInDock).toBe(false);
  await act(async () => host.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
  expect(useStore.getState().settings.showInDock).toBe(true);
});
it("blocks another click while saving and reports failure without changing the choice", async () => {
  let rejectSave!: (reason: Error) => void;
  const save = vi.fn(() => new Promise<void>((_resolve, reject) => { rejectSave = reject; }));
  useStore.setState({ saveSettings: save }); await mount();
  await act(async () => host.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
  expect(host.querySelector<HTMLButtonElement>('[role="switch"]')?.disabled).toBe(true);
  expect(save).toHaveBeenCalledExactlyOnceWith({ showInDock: false });
  await act(async () => rejectSave(new Error("synthetic-write-failure")));
  expect(host.querySelector('[role="alert"]')).toBeTruthy();
  expect(host.querySelector<HTMLButtonElement>('[role="switch"]')?.disabled).toBe(false);
  expect(useStore.getState().settings.showInDock).toBe(true);
});
