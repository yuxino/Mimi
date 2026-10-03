// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { ConfigInput } from "./ConfigInput";
import { I18N } from "../../lib/i18n";
let host: HTMLDivElement, root: Root;
const read = vi.fn<() => Promise<string>>();
const changed = vi.fn();
function Draft({ disabled = false }: { disabled?: boolean }) {
  const [value, setValue] = useState("");
  return <ConfigInput id="synthetic-key" type="password" value={value} disabled={disabled} onValueChange={next => { changed(next); setValue(next); }} />;
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { readText: read } });
  read.mockReset(); changed.mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(() => root.unmount()); host.remove(); Reflect.deleteProperty(navigator, "clipboard"); vi.unstubAllGlobals(); });
it("reads only on a paste click, updates the draft, and retains normal password input", async () => {
  read.mockResolvedValue("synthetic-pasted-key");
  await act(() => root.render(<Draft />));
  expect(read).not.toHaveBeenCalled();
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(read).toHaveBeenCalledOnce();
  expect(changed).toHaveBeenCalledExactlyOnceWith("synthetic-pasted-key");
  expect(host.querySelector<HTMLInputElement>("input")!.value).toBe("synthetic-pasted-key");
  expect(host.querySelector("input")!.type).toBe("password");
});
it("does not read when disabled, rejects a late read after switching fields, and does not echo clipboard errors", async () => {
  await act(() => root.render(<Draft disabled />));
  await act(() => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(read).not.toHaveBeenCalled();
  let finish!: (value: string) => void;
  read.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await act(() => root.render(<Draft key="first" />));
  await act(() => host.querySelector<HTMLButtonElement>("button")!.click());
  await act(() => root.render(<Draft key="second" />));
  await act(async () => finish("synthetic-stale-key"));
  expect(changed).not.toHaveBeenCalled();
  read.mockRejectedValueOnce(new Error("synthetic-private-clipboard-error"));
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(host.querySelector('[role="status"]')?.textContent).toBe(I18N.settings.pasteFailed);
  expect(host.textContent).not.toContain("synthetic-private");
});
