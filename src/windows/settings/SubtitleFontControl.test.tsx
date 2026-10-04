// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { installedFontFamilies } from "../../lib/ipc";
import { I18N } from "../../lib/i18n";
import { SubtitleFontControl } from "./SubtitleFontControl";
import { SettingsToastRegion } from "./SettingsToast";
import { useSettingsToast } from "./useSettingsToast";

vi.mock("../../lib/ipc", async original => ({ ...await original<typeof import("../../lib/ipc")>(), installedFontFamilies: vi.fn(), isTauri: false }));
let host: HTMLDivElement, root: Root;
const save = vi.fn();
function Harness({ initial = "" }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  const { runWithToast } = useSettingsToast();
  return <><SubtitleFontControl value={value} onChange={family => runWithToast(async () => {
    await save(family);
    setValue(family);
  }, I18N.settings.settingSaveFailed(I18N.settings.subtitleFont))} /><SettingsToastRegion /></>;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(installedFontFamilies).mockReset().mockResolvedValue(["Arial", "Noto Sans CJK SC", "Times New Roman"]);
  save.mockReset().mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(() => root.unmount()); host.remove(); vi.unstubAllGlobals();
});
async function mount(initial = "") { await act(() => root.render(<Harness initial={initial} />)); }
function trigger() { return host.querySelector<HTMLButtonElement>('button[role="combobox"]')!; }
async function open() { await act(async () => trigger().click()); }
function options() { return [...document.querySelectorAll<HTMLElement>('[role="option"]')]; }
async function choose(label: string) { await act(async () => options().find(option => option.textContent === label)!.click()); }

it("loads on opening, searches installed families, saves a choice and restores the system default", async () => {
  await mount();
  expect(trigger().textContent).toBe(I18N.settings.subtitleFontDefault);
  expect(installedFontFamilies).not.toHaveBeenCalled();
  await open();
  expect(installedFontFamilies).toHaveBeenCalledTimes(1);
  const input = document.querySelector<HTMLInputElement>(".mimi-select__search")!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "noto");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(options().map(option => option.textContent)).toEqual(["Noto Sans CJK SC"]);
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  expect(save).toHaveBeenCalledExactlyOnceWith("Noto Sans CJK SC");
  expect(trigger().textContent).toBe("Noto Sans CJK SC");
  expect(host.querySelector(".settings-toast")).toBeNull();
  await open();
  expect(installedFontFamilies).toHaveBeenCalledTimes(2);
  await choose(I18N.settings.subtitleFontDefault);
  expect(save).toHaveBeenLastCalledWith("");
  expect(trigger().textContent).toBe(I18N.settings.subtitleFontDefault);
});

it("preserves a removed font until the user chooses a replacement and refreshes on reopen", async () => {
  await mount("Removed Font"); await open();
  expect(trigger().textContent).toBe("Removed Font");
  expect(save).not.toHaveBeenCalled();
  await open(); // close
  vi.mocked(installedFontFamilies).mockResolvedValueOnce(["New Font"]);
  await open();
  expect(options().map(option => option.textContent)).toEqual([I18N.settings.subtitleFontDefault, "New Font"]);
  await choose("New Font");
  expect(save).toHaveBeenCalledExactlyOnceWith("New Font");
});

it("keeps the saved font after a failed save and shows a sanitized transient error", async () => {
  save.mockRejectedValueOnce(new Error("private-native-error"));
  await mount("Arial"); await open(); await choose("Times New Roman");
  expect(trigger().textContent).toBe("Arial");
  expect(trigger().disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.settings.settingSaveFailed(I18N.settings.subtitleFont));
  expect(document.body.textContent).not.toContain("private-native-error");
});

it("allows reset after an enumeration failure and retries when reopened", async () => {
  vi.mocked(installedFontFamilies).mockRejectedValueOnce(new Error("private-font-error"));
  await mount("Removed Font"); await open();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.settings.subtitleFontsLoadFailed);
  expect(document.body.textContent).not.toContain("private-font-error");
  await choose(I18N.settings.subtitleFontDefault);
  await open();
  expect(options()).toHaveLength(4);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("shares an in-flight enumeration and ignores failure after navigation", async () => {
  let reject!: (reason: Error) => void;
  vi.mocked(installedFontFamilies).mockReturnValue(new Promise((_, fail) => { reject = fail; }));
  await mount(); await open(); await open(); await open();
  expect(installedFontFamilies).toHaveBeenCalledTimes(1);
  expect(document.querySelector('.mimi-select__menu [role="status"]')?.textContent).toBe(I18N.settings.subtitleFontsLoading);
  await act(() => root.render(<SettingsToastRegion scopeKey="other-page" />));
  await act(async () => reject(new Error("late-error")));
  expect(host.querySelector('[role="alert"]')).toBeNull();
});


it("keeps the saved font as the keyboard choice after lazy enumeration completes", async () => {
  await mount("Times New Roman"); await open();
  expect(document.querySelector('[role="option"][data-active="true"]')?.textContent).toBe("Times New Roman");
  const input = document.querySelector<HTMLInputElement>(".mimi-select__search")!;
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  expect(save).not.toHaveBeenCalled();
  expect(trigger().textContent).toBe("Times New Roman");
});

it("opens with an arrow key, browses loaded fonts in both directions and only saves on Enter", async () => {
  await mount("Arial");
  await act(async () => trigger().dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  const input = document.querySelector<HTMLInputElement>(".mimi-select__search")!;
  expect(document.activeElement).toBe(input);
  const active = () => document.getElementById(input.getAttribute("aria-activedescendant")!)?.textContent;
  expect(active()).toBe("Arial");
  await act(() => input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  expect(active()).toBe("Noto Sans CJK SC");
  await act(() => input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })));
  expect(active()).toBe("Arial");
  await act(() => input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  expect(save).not.toHaveBeenCalled();
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  expect(save).toHaveBeenCalledExactlyOnceWith("Noto Sans CJK SC");
  expect(trigger().textContent).toBe("Noto Sans CJK SC");
  expect(document.activeElement).toBe(trigger());
  await open();
  const reopened = document.querySelector<HTMLInputElement>(".mimi-select__search")!;
  await act(() => reopened.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })));
  await act(() => reopened.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(trigger().textContent).toBe("Noto Sans CJK SC");
  expect(save).toHaveBeenCalledTimes(1);
});
