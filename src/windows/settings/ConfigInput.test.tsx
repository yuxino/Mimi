// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act, createRef, useState, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { ConfigInput, type ConfigInputElement } from "./ConfigInput";
import { I18N } from "../../lib/i18n";
let host: HTMLDivElement, root: Root;
const read = vi.fn<() => Promise<string>>();
const changed = vi.fn();
function Draft({ disabled = false }: { disabled?: boolean }) {
  const [value, setValue] = useState("");
  return <ConfigInput id="synthetic-key" type="password" value={value} disabled={disabled} onValueChange={next => { changed(next); setValue(next); }} />;
}
function ExpandableDraft({ initial = "", ...props }: Omit<ComponentProps<typeof ConfigInput>, "value" | "onValueChange"> & { initial?: string }) {
  const [value, setValue] = useState(initial);
  return <ConfigInput id="synthetic-config" type="text" expandable value={value} {...props} onValueChange={next => { changed(next); setValue(next); }} />;
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
  await act(() => root.render(<><Draft /><SettingsToastRegion /></>));
  expect(read).not.toHaveBeenCalled();
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(read).toHaveBeenCalledOnce();
  expect(changed).toHaveBeenCalledExactlyOnceWith("synthetic-pasted-key");
  expect(host.querySelector<HTMLInputElement>("input")!.value).toBe("synthetic-pasted-key");
  expect(host.querySelector("input")!.type).toBe("password");
});
it("does not read when disabled, rejects a late read after switching fields, and does not echo clipboard errors", async () => {
  await act(() => root.render(<><Draft disabled /><SettingsToastRegion /></>));
  await act(() => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(read).not.toHaveBeenCalled();
  let finish!: (value: string) => void;
  read.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await act(() => root.render(<><Draft key="first" /><SettingsToastRegion /></>));
  await act(() => host.querySelector<HTMLButtonElement>("button")!.click());
  await act(() => root.render(<><Draft key="second" /><SettingsToastRegion /></>));
  await act(async () => finish("synthetic-stale-key"));
  expect(changed).not.toHaveBeenCalled();
  read.mockRejectedValueOnce(new Error("synthetic-private-clipboard-error"));
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.pasteFailed);
  expect(host.textContent).not.toContain("synthetic-private");
});

it("expands a long value into one wrapping editable textbox and preserves its selection and field semantics", async () => {
  const value = `https://synthetic.example/${"long-path/".repeat(30)}`;
  const field = createRef<ConfigInputElement>();
  await act(() => root.render(<ExpandableDraft initial={value} ref={field} maxLength={512} required aria-invalid aria-describedby="synthetic-error" />));
  const original = host.querySelector<HTMLInputElement>("input")!;
  await act(() => { original.focus(); original.setSelectionRange(8, 25); });
  const expand = host.querySelector<HTMLButtonElement>(".config-input__expand")!;
  expect(expand.getAttribute("aria-label")).toBe(I18N.settings.expandField);
  expect(expand.getAttribute("aria-expanded")).toBe("false");
  await act(() => expand.click());
  expect(host.querySelector("input")).toBeNull();
  const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;
  expect(host.querySelectorAll("input, textarea")).toHaveLength(1);
  expect(textarea.id).toBe("synthetic-config");
  expect(textarea.value).toBe(value);
  expect(textarea.wrap).toBe("soft");
  expect(textarea.maxLength).toBe(512);
  expect(textarea.required).toBe(true);
  expect(textarea.getAttribute("aria-invalid")).toBe("true");
  expect(textarea.getAttribute("aria-describedby")).toBe("synthetic-error");
  expect(document.activeElement).toBe(textarea);
  expect(field.current).toBe(textarea);
  expect([textarea.selectionStart, textarea.selectionEnd]).toEqual([8, 25]);
  expect(changed).not.toHaveBeenCalled();
  expect(read).not.toHaveBeenCalled();
  await act(() => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "https://edited.example/\r\npath");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(textarea.value).toBe("https://edited.example/path");
  expect(changed).toHaveBeenLastCalledWith("https://edited.example/path");
  await act(() => textarea.setSelectionRange(4, 8));
  const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
  await act(() => textarea.dispatchEvent(escape));
  expect(escape.defaultPrevented).toBe(true);
  const collapsed = host.querySelector<HTMLInputElement>("input")!;
  expect(collapsed.value).toBe("https://edited.example/path");
  expect(document.activeElement).toBe(collapsed);
  expect(field.current).toBe(collapsed);
  expect([collapsed.selectionStart, collapsed.selectionEnd]).toEqual([4, 8]);
});

it("keeps placeholders empty and blocks line breaks without intercepting IME confirmation", async () => {
  const placeholder = `https://synthetic.example/${"example/".repeat(30)}`;
  const onKeyDown = vi.fn();
  await act(() => root.render(<ExpandableDraft placeholder={placeholder} onKeyDown={onKeyDown} />));
  await act(() => host.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;
  expect(textarea.placeholder).toBe(placeholder);
  expect(textarea.value).toBe("");
  expect(changed).not.toHaveBeenCalled();
  const enter = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
  await act(() => textarea.dispatchEvent(enter));
  expect(enter.defaultPrevented).toBe(true);
  const composing = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, isComposing: true });
  await act(() => textarea.dispatchEvent(composing));
  expect(composing.defaultPrevented).toBe(false);
  expect(onKeyDown).toHaveBeenCalledTimes(2);
});

it("keeps expansion focus inside the field group and passes a single-line pasted draft to its owner", async () => {
  const leftGroup = vi.fn(), onPasteValue = vi.fn();
  await act(() => root.render(<ExpandableDraft maxLength={40} onPasteValue={onPasteValue} onGroupBlur={event => {
    if (!event.currentTarget.contains(event.relatedTarget)) leftGroup();
  }} />));
  await act(() => host.querySelector<HTMLInputElement>("input")!.focus());
  const expand = host.querySelector<HTMLButtonElement>(".config-input__expand")!;
  await act(() => { expand.focus(); expand.click(); });
  expect(leftGroup).not.toHaveBeenCalled();
  read.mockResolvedValue("https://synthetic.example/\npath");
  const paste = host.querySelector<HTMLButtonElement>(".config-input__paste")!;
  await act(async () => { paste.focus(); paste.click(); });
  expect(leftGroup).not.toHaveBeenCalled();
  expect(host.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe("https://synthetic.example/path");
  expect(onPasteValue).toHaveBeenCalledExactlyOnceWith("https://synthetic.example/path");
  await act(() => expand.click());
  expect(leftGroup).not.toHaveBeenCalled();
  const outside = document.createElement("button"); document.body.append(outside);
  await act(() => outside.focus());
  expect(leftGroup).toHaveBeenCalledOnce();
  outside.remove();
});

it("does not report a proxy-style draft as blurred when a mouse click replaces the native textbox", async () => {
  const leftGroup = vi.fn();
  let nativeField: ConfigInputElement | null = null;
  const nativeRef = (element: ConfigInputElement | null) => {
    // WebKit reports no next focus target when a focused textbox is removed.
    if (!element && nativeField) nativeField.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null }));
    nativeField = element;
  };
  await act(() => root.render(<ExpandableDraft initial="invalid-proxy-draft" ref={nativeRef} onGroupBlur={event => {
    if (!event.currentTarget.contains(event.relatedTarget)) leftGroup();
  }} />));
  await act(() => host.querySelector<HTMLInputElement>("input")!.focus());
  for (const tag of ["textarea", "input"] as const) {
    const expand = host.querySelector<HTMLButtonElement>(".config-input__expand")!;
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    await act(() => {
      expand.dispatchEvent(down);
      expand.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      expand.click();
    });
    expect(down.defaultPrevented).toBe(true);
    expect(host.querySelector<ConfigInputElement>(tag)!.value).toBe("invalid-proxy-draft");
    expect(document.activeElement).toBe(host.querySelector(tag));
    expect(leftGroup).not.toHaveBeenCalled();
    expect(changed).not.toHaveBeenCalled();
  }
  await act(() => host.querySelector<HTMLInputElement>("input")!.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null })));
  expect(leftGroup).toHaveBeenCalledOnce();
});

it("preserves clipboard limits, disabled and read-only behavior in expanded fields", async () => {
  await act(() => root.render(<><ExpandableDraft maxLength={5} /><SettingsToastRegion /></>));
  await act(() => host.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  read.mockResolvedValue("too-long-value");
  await act(async () => host.querySelector<HTMLButtonElement>(".config-input__paste")!.click());
  expect(changed).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.pasteFailed);
  await act(() => root.render(<><ExpandableDraft disabled maxLength={5} /><SettingsToastRegion /></>));
  expect(host.querySelector<HTMLTextAreaElement>("textarea")!.disabled).toBe(true);
  expect(host.querySelector<HTMLButtonElement>(".config-input__expand")!.disabled).toBe(true);
  expect(host.querySelector<HTMLButtonElement>(".config-input__paste")!.disabled).toBe(true);
  await act(() => root.render(<ExpandableDraft key="readonly" readOnly initial="saved" />));
  await act(() => host.querySelector<HTMLButtonElement>(".config-input__expand")!.click());
  expect(host.querySelector<HTMLTextAreaElement>("textarea")!.readOnly).toBe(true);
  expect(host.querySelector<HTMLButtonElement>(".config-input__paste")!.disabled).toBe(true);
});

it("keeps expansion opt-in and never expands a password input", async () => {
  await act(() => root.render(<ExpandableDraft expandable={false} initial="ordinary" />));
  expect(host.querySelector(".config-input__expand, textarea")).toBeNull();
  await act(() => root.render(<ExpandableDraft expandable type="password" initial="synthetic-key" />));
  expect(host.querySelector(".config-input__expand, textarea")).toBeNull();
  expect(host.querySelector<HTMLInputElement>("input")!.type).toBe("password");
  expect(changed).not.toHaveBeenCalled();
});
