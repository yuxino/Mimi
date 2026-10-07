// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, UI_LANGUAGES, setStoredUiLanguage } from "../../lib/i18n";
import { isClickablePointerTarget, OVERLAY_POINTER_TARGET_EVENT } from "../../lib/overlayPointer";
import { DragHandle } from "./DragHandle";

const native = vi.hoisted(() => ({ enabled: true, move: vi.fn() }));
vi.mock("../../lib/ipc", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  get isTauri() { return native.enabled; },
  overlayMoveStart: native.move,
}));

let host: HTMLDivElement;
let root: Root;
const toggle = vi.fn();

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  setStoredUiLanguage("en");
  native.enabled = true;
  native.move.mockReset().mockResolvedValue(undefined);
  toggle.mockReset();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(() => root.unmount());
  host.remove();
  setStoredUiLanguage("system");
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function render(props: Partial<React.ComponentProps<typeof DragHandle>> = {}) {
  await act(() => root.render(<DragHandle onToggleCollapsed={toggle} {...props} />));
  return host.querySelector<HTMLButtonElement>('[data-testid="drag-handle"]')!;
}

async function press(button: HTMLButtonElement, detail = 1, mouseButton = 0) {
  const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: mouseButton, detail });
  await act(() => button.dispatchEvent(event));
  return event;
}

it.each([
  { compact: false, width: 40, expectedWidth: 40, expectedHeight: 18 },
  { compact: false, width: 120, expectedWidth: 120, expectedHeight: 18 },
  { compact: true, width: 120, expectedWidth: 42, expectedHeight: 30 },
])("keeps the narrow/compact hit area and an accessible action: %j", async ({ compact, width, expectedWidth, expectedHeight }) => {
  const button = await render({ compact, width });
  expect(button.tagName).toBe("BUTTON");
  expect(button.getAttribute("aria-label")).toBe(compact ? I18N.overlay.expandSubtitle : I18N.overlay.collapseSubtitle);
  expect(button.style.width).toBe(`${expectedWidth}px`);
  expect(button.style.height).toBe(`${expectedHeight}px`);
  expect(button.style.cursor).toBe("pointer");
  expect(isClickablePointerTarget(button.firstElementChild)).toBe(true);
});

it("keeps a primary press for dragging and the second press for collapse without focusing", async () => {
  const button = await render();
  expect((await press(button)).defaultPrevented).toBe(true);
  expect(native.move).toHaveBeenCalledOnce();
  expect(toggle).not.toHaveBeenCalled();
  expect(document.activeElement).not.toBe(button);
  await act(() => button.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 })));
  await press(button, 2);
  await act(() => button.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 2 })));
  expect(toggle).toHaveBeenCalledOnce();
  expect(native.move).toHaveBeenCalledOnce();
  await press(button, 1, 1);
  await press(button, 1, 2);
  expect(toggle).toHaveBeenCalledOnce();
  expect(native.move).toHaveBeenCalledOnce();
});

it("keeps double-click collapse in browser previews without starting native movement", async () => {
  native.enabled = false;
  const button = await render();
  await press(button);
  await press(button, 2);
  expect(native.move).not.toHaveBeenCalled();
  expect(toggle).toHaveBeenCalledOnce();
});

it.each(["Enter", " "])("toggles once with %j and suppresses default activation, repeats and IME shortcuts", async key => {
  const button = await render();
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  await act(() => button.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(true);
  expect(toggle).toHaveBeenCalledOnce();
  for (const modifiers of [{ repeat: true }, { isComposing: true }, { ctrlKey: true }, { metaKey: true }, { altKey: true }]) {
    await act(() => button.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers })));
  }
  expect(toggle).toHaveBeenCalledOnce();
  expect(native.move).not.toHaveBeenCalled();
});

it("accepts assistive click activation without treating it as a drag", async () => {
  const button = await render();
  await act(() => button.click());
  expect(toggle).toHaveBeenCalledOnce();
  expect(native.move).not.toHaveBeenCalled();
});

it.each([{ ctrlKey: true }, { altKey: true }, { metaKey: true }])("ignores modified Space's browser click after keyup: %j", async modifiers => {
  const button = await render();
  const keydown = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true, ...modifiers });
  await act(() => {
    button.dispatchEvent(keydown);
    button.dispatchEvent(new KeyboardEvent("keyup", { key: " ", bubbles: true, cancelable: true, ...modifiers }));
    button.dispatchEvent(new MouseEvent("click", { detail: 0, bubbles: true, cancelable: true, ...modifiers }));
  });
  // This also prevents a click if the modifier is released before Space.
  expect(keydown.defaultPrevented).toBe(true);
  expect(toggle).not.toHaveBeenCalled();
  expect(native.move).not.toHaveBeenCalled();
});

it.each([{ disabled: true }, { busy: true }])("blocks movement and toggling while unavailable: %j", async props => {
  const button = await render(props);
  expect(button.disabled).toBe(true);
  expect(button.style.cursor).toBe("busy" in props ? "progress" : "default");
  expect(isClickablePointerTarget(button.firstElementChild)).toBe(false);
  await press(button);
  await press(button, 2);
  await act(() => {
    button.click();
    button.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  });
  expect(native.move).not.toHaveBeenCalled();
  expect(toggle).not.toHaveBeenCalled();
});

it("uses shared tooltip feedback for inactive native hover and keyboard focus", async () => {
  const button = await render();
  await act(() => document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: button.firstElementChild })));
  const tooltip = () => document.querySelector<HTMLElement>('[role="tooltip"]');
  expect(tooltip()?.textContent).toBe(I18N.overlay.dragTooltip);
  expect(button.getAttribute("aria-describedby")).toBe(tooltip()?.id);
  expect((button.firstElementChild as HTMLElement).style.width).toBe("40px");
  expect(document.activeElement).not.toBe(button);
  await act(() => document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: null })));
  expect(tooltip()).toBeNull();
  expect((button.firstElementChild as HTMLElement).style.width).toBe("32px");
  const matches = button.matches.bind(button);
  vi.spyOn(button, "matches").mockImplementation(selector => selector === ":focus-visible" || matches(selector));
  await act(() => button.focus());
  expect(tooltip()?.textContent).toBe(I18N.overlay.dragTooltip);
  expect(button.className).toContain("focus-visible:outline-white");
  await act(() => button.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(tooltip()).toBeNull();
  expect(toggle).not.toHaveBeenCalled();
  expect(native.move).not.toHaveBeenCalled();
});

it("allows native movement while error presentation prevents pointer, keyboard and assistive collapse", async () => {
  const button = await render({ collapseDisabled: true });
  expect(button.disabled).toBe(false);
  expect(button.style.cursor).toBe("pointer");
  expect(isClickablePointerTarget(button.firstElementChild)).toBe(true);
  expect(button.getAttribute("aria-label")).toBe(I18N.overlay.moveSubtitle);
  await press(button);
  expect(native.move).toHaveBeenCalledOnce();
  await press(button, 2);
  await act(() => {
    button.click();
    for (const key of ["Enter", " "]) {
      const keydown = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      button.dispatchEvent(keydown);
      expect(keydown.defaultPrevented).toBe(true);
    }
  });
  expect(toggle).not.toHaveBeenCalled();
  await act(() => document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: null })));
  await act(() => document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: button.firstElementChild })));
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.overlay.moveSubtitle);
  await render();
  await press(button, 2);
  expect(toggle).toHaveBeenCalledOnce();
});

it.each(UI_LANGUAGES)("uses a state-specific expansion hint in the 54px compact surface: %s", async language => {
  setStoredUiLanguage(language);
  const button = await render({ compact: true });
  await act(() => document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: button.firstElementChild })));
  const tooltip = document.querySelector<HTMLElement>('[role="tooltip"]');
  expect(tooltip?.textContent).toBe(I18N.overlay.expandDragTooltip);
  expect(tooltip?.textContent).not.toBe(I18N.overlay.dragTooltip);
  expect(button.getAttribute("aria-describedby")).toBe(tooltip?.id);
  await act(() => button.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })));
  await press(button, 2);
  expect(toggle).toHaveBeenCalledOnce();
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
});
