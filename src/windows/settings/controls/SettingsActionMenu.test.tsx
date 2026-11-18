// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SettingsActionMenu } from "./SettingsActionMenu";

let host: HTMLDivElement, root: Root;
const copy = vi.fn(), remove = vi.fn();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  copy.mockReset(); remove.mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
async function render(disabled = false, copyDisabled = false, deleteDisabled = false) {
  await act(() => root.render(<SettingsActionMenu label="More" name="Configuration" disabled={disabled} actions={[
    { id: "duplicate", label: "Copy", icon: "copy", disabled: copyDisabled, onSelect: copy },
    { id: "delete", label: "Delete", icon: "trash", destructive: true, disabled: deleteDisabled, onSelect: remove },
  ]} />));
}
const trigger = () => host.querySelector("button")!;
const menu = () => document.querySelector<HTMLElement>('[role="menu"]');
async function key(target: Element, key: string) {
  await act(() => target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
}
async function open() { await act(() => trigger().click()); }

it("supports keyboard entry, wrapping, Home/End and Escape with restored focus", async () => {
  await render();
  await key(trigger(), "ArrowUp");
  expect(document.activeElement?.textContent).toBe("Delete");
  await key(menu()!, "ArrowDown");
  expect(document.activeElement?.textContent).toBe("Copy");
  await key(menu()!, "End");
  expect(document.activeElement?.textContent).toBe("Delete");
  await key(menu()!, "Home");
  expect(document.activeElement?.textContent).toBe("Copy");
  await key(menu()!, "Escape");
  expect(menu()).toBeNull(); expect(document.activeElement).toBe(trigger());
  expect(copy).not.toHaveBeenCalled(); expect(remove).not.toHaveBeenCalled();
});

it("skips disabled actions and keeps an all-disabled menu dismissible", async () => {
  await render(false, true); await open();
  expect(document.activeElement?.textContent).toBe("Delete");
  await key(menu()!, "ArrowDown");
  expect(document.activeElement?.textContent).toBe("Delete");
  await act(() => menu()!.querySelector<HTMLButtonElement>(".service-row__duplicate")!.click());
  expect(copy).not.toHaveBeenCalled();
  await key(menu()!, "Escape");
  await render(false, true, true); await open();
  expect(document.activeElement).toBe(menu());
  await key(menu()!, "Escape"); expect(menu()).toBeNull();
});

it("dismisses before invoking an action and restores its trigger", async () => {
  await render(); await open();
  copy.mockImplementation(() => {
    // React flushes removal after the handler, but focus moves synchronously.
    expect(document.activeElement).toBe(trigger());
  });
  await act(() => menu()!.querySelector<HTMLButtonElement>(".service-row__duplicate")!.click());
  expect(copy).toHaveBeenCalledOnce(); expect(menu()).toBeNull();
});

it("closes on outside input, viewport changes and blur without stealing outside focus", async () => {
  await render();
  for (const event of ["pointerdown", "scroll", "resize", "blur"]) {
    await open();
    await act(() => (event === "resize" || event === "blur" ? window : document).dispatchEvent(new Event(event)));
    expect(menu()).toBeNull();
  }
  await open();
  const outside = document.createElement("button"); host.append(outside);
  await act(() => outside.focus());
  expect(menu()).toBeNull(); expect(document.activeElement).toBe(outside);
  outside.remove();
});

it("closes for Tab or a busy/hidden owner and does not reopen when enabled", async () => {
  await render(); await open(); await key(menu()!, "Tab");
  expect(menu()).toBeNull(); expect(document.activeElement).toBe(trigger());
  await open(); await render(true);
  expect(menu()).toBeNull(); expect(trigger().disabled).toBe(true);
  await render(); expect(menu()).toBeNull();
});
