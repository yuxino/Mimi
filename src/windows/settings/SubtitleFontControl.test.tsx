// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { installedFontFamilies } from "../../lib/ipc";
import { I18N } from "../../lib/i18n";
import { SubtitleFontControl } from "./SubtitleFontControl";
import { SettingsToastRegion } from "./SettingsToast";

vi.mock("../../lib/ipc", async original => ({ ...await original<typeof import("../../lib/ipc")>(), installedFontFamilies: vi.fn(), isTauri: false }));
let host: HTMLDivElement, root: Root;
const save = vi.fn();
const preview = vi.fn();
function Harness({ initial = "" }: { initial?: string }) {
  const [value, setValue] = useState(initial);
  return <><SubtitleFontControl value={value} onPreview={preview} onChange={async family => {
    await save(family);
    setValue(family);
  }} /><SettingsToastRegion /></>;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(installedFontFamilies).mockReset().mockResolvedValue(["Arial", "Noto Sans CJK SC", "Times New Roman"]);
  save.mockReset().mockResolvedValue(undefined);
  preview.mockReset();
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
async function arrow(key: "ArrowDown" | "ArrowUp") {
  await act(async () => trigger().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
}

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

it("cycles all installed fonts directly after a filtered selection while keeping the focused menu closed", async () => {
  await mount("Arial"); await open();
  const search = document.querySelector<HTMLInputElement>(".mimi-select__search")!;
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "noto");
    search.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await choose("Noto Sans CJK SC");
  const button = trigger();
  expect(document.activeElement).toBe(button);
  for (const [key, value] of [
    ["ArrowDown", "Times New Roman"], ["ArrowUp", "Noto Sans CJK SC"], ["ArrowUp", "Arial"],
    ["ArrowUp", ""], ["ArrowUp", "Times New Roman"], ["ArrowDown", ""],
  ] as const) {
    await arrow(key);
    expect(save).toHaveBeenLastCalledWith(value);
    expect(trigger().textContent).toBe(value || I18N.settings.subtitleFontDefault);
    expect(document.activeElement).toBe(button);
    expect(button.disabled).toBe(false);
    expect(document.querySelector('[role="listbox"]')).toBeNull();
  }
  expect(installedFontFamilies).toHaveBeenCalledOnce();
});

it("previews rapid arrows immediately and serializes only the latest queued choice without losing focus", async () => {
  let finishFirst!: () => void, finishLast!: () => void;
  save.mockImplementationOnce(() => new Promise<void>(resolve => { finishFirst = resolve; }))
    .mockImplementationOnce(() => new Promise<void>(resolve => { finishLast = resolve; }));
  await mount("Arial"); await open(); await choose("Noto Sans CJK SC");
  const button = trigger();
  expect(button.disabled).toBe(false);
  expect(preview).toHaveBeenLastCalledWith("Noto Sans CJK SC");
  await act(() => {
    for (const key of ["ArrowDown", "ArrowDown", "ArrowDown"]) button.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
  });
  expect(preview.mock.calls).toEqual([["Noto Sans CJK SC"], ["Times New Roman"], [""], ["Arial"]]);
  expect(button.textContent).toBe("Arial");
  expect(save).toHaveBeenCalledExactlyOnceWith("Noto Sans CJK SC");
  expect(document.activeElement).toBe(button);
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  await act(async () => finishFirst());
  expect(save.mock.calls).toEqual([["Noto Sans CJK SC"], ["Arial"]]);
  expect(button.textContent).toBe("Arial");
  expect(preview).toHaveBeenLastCalledWith("Arial");
  await act(async () => finishLast());
  expect(preview).toHaveBeenLastCalledWith(undefined);
  expect(button.textContent).toBe("Arial");
  expect(document.activeElement).toBe(button);
  expect(host.querySelector('.settings-select-wrap[aria-busy="true"]')).toBeNull();
});

it("does not save again when rapid arrows return to the choice already being persisted", async () => {
  let finish!: () => void;
  save.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }));
  await mount("Arial"); await open(); await choose("Noto Sans CJK SC");
  await arrow("ArrowDown"); await arrow("ArrowUp");
  await act(async () => finish());
  expect(save).toHaveBeenCalledExactlyOnceWith("Noto Sans CJK SC");
  expect(trigger().textContent).toBe("Noto Sans CJK SC");
  expect(preview).toHaveBeenLastCalledWith(undefined);
});

it("rolls a failed final choice back to the last saved font and permits another arrow", async () => {
  let finishFirst!: () => void, rejectLast!: (error: Error) => void;
  save.mockImplementationOnce(() => new Promise<void>(resolve => { finishFirst = resolve; }))
    .mockImplementationOnce(() => new Promise<void>((_resolve, reject) => { rejectLast = reject; }));
  await mount("Arial"); await open(); await choose("Noto Sans CJK SC");
  await arrow("ArrowDown");
  await act(async () => finishFirst());
  expect(trigger().textContent).toBe("Times New Roman");
  await act(async () => rejectLast(new Error("private-save-error")));
  expect(trigger().textContent).toBe("Noto Sans CJK SC");
  expect(document.activeElement).toBe(trigger());
  expect(preview).toHaveBeenLastCalledWith(undefined);
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.settings.settingSaveFailed(I18N.settings.subtitleFont));
  expect(document.body.textContent).not.toContain("private-save-error");
  await arrow("ArrowDown");
  expect(save).toHaveBeenLastCalledWith("Times New Roman");
  expect(trigger().textContent).toBe("Times New Roman");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it.each([
  ["ArrowDown", ""], ["ArrowUp", "Times New Roman"],
] as const)("replaces a removed font from a defined boundary on %s", async (key, expected) => {
  await mount("Removed Font"); await open();
  await act(() => document.querySelector<HTMLInputElement>(".mimi-select__search")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  await arrow(key);
  expect(save).toHaveBeenCalledExactlyOnceWith(expected);
  expect(trigger().textContent).toBe(expected || I18N.settings.subtitleFontDefault);
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});

it("keeps an empty installed list on system default without creating saves", async () => {
  vi.mocked(installedFontFamilies).mockResolvedValueOnce([]);
  await mount(); await open(); await choose(I18N.settings.subtitleFontDefault);
  await arrow("ArrowDown"); await arrow("ArrowUp");
  expect(save).not.toHaveBeenCalled();
  expect(document.querySelector('[role="listbox"]')).toBeNull();
  expect(trigger().textContent).toBe(I18N.settings.subtitleFontDefault);
});

it("finishes the latest queued choice after navigation without reviving the old preview or failed-save notice", async () => {
  let finish!: () => void;
  save.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }))
    .mockRejectedValueOnce(new Error("late-save-error"));
  await mount("Arial"); await open(); await choose("Noto Sans CJK SC"); await arrow("ArrowDown");
  await act(() => root.render(<SettingsToastRegion scopeKey="other-page" />));
  expect(preview).toHaveBeenLastCalledWith(undefined);
  const callsAtNavigation = preview.mock.calls.length;
  await act(async () => finish());
  expect(save.mock.calls).toEqual([["Noto Sans CJK SC"], ["Times New Roman"]]);
  expect(preview).toHaveBeenCalledTimes(callsAtNavigation);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});
