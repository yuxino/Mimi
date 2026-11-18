// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import type { IconName } from "../../components/Icon";
import { ControlButton } from "./ControlButton";
import { OVERLAY_POINTER_TARGET_EVENT } from "../../lib/overlay/overlayPointer";

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    disconnect() {}
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const popup = this.classList.contains("mimi-tooltip");
    return { left: 600, top: 16, right: 624, bottom: 40, width: popup ? 130 : 24, height: 24 } as DOMRect;
  });
  host = document.createElement("div");
  host.style.overflow = "hidden";
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  setStoredUiLanguage("system");
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function hover() {
  await act(async () => host.querySelector(".mimi-tooltip-trigger")!.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })));
}

it.each(["zh", "en", "ja"] as const)("shows the %s label for every icon action in a portal outside clipped parents", async (language) => {
  setStoredUiLanguage(language);
  const actions: [IconName, string][] = [
    ["play", I18N.overlay.retry],
    ["pause", I18N.overlay.pause],
    ["play", I18N.overlay.resume],
    ["chevron-down", I18N.overlay.expandSubtitle],
    ["chevron-up", I18N.overlay.collapseSubtitle],
    ["eraser", I18N.overlay.clearSubtitles],
    ["blend", I18N.overlay.enterImmersiveMode],
    ["lock", I18N.overlay.lockPosition],
    ["gear", I18N.overlay.openSettings],
  ];
  for (const [icon, label] of actions) {
    await act(async () => root.render(<ControlButton key={label} icon={icon} label={label} onClick={() => {}} />));
    await hover();
    const tooltip = document.querySelector<HTMLElement>('[role="tooltip"]')!;
    expect(tooltip.textContent).toBe(label);
    expect(tooltip.parentElement).toBe(document.body);
    expect(host.contains(tooltip)).toBe(false);
    expect(tooltip.style.visibility).toBe("visible");
    expect(host.querySelector("button")?.getAttribute("aria-describedby")).toBe(tooltip.id);
  }
});

it("shows on keyboard focus, dismisses with Escape, and removes the tooltip on blur", async () => {
  await act(async () => root.render(<ControlButton icon="gear" label="Open settings" onClick={() => {}} />));
  const button = host.querySelector<HTMLButtonElement>("button")!;
  await act(async () => button.focus());
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe("Open settings");
  await act(async () => button.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  await act(async () => button.blur());
  await act(async () => button.focus());
  expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
  await act(async () => button.blur());
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
});

it("shows an available shortcut with the hover label without changing the action's accessible name", async () => {
  await act(() => root.render(<ControlButton icon="lock" label="Lock Subtitle Position" shortcutHint="⌘⇧L" onClick={() => {}} />));
  await hover();
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe("Lock Subtitle Position ⌘⇧L");
  expect(document.querySelector('[role="tooltip"] kbd')?.textContent).toBe("⌘⇧L");
  expect(host.querySelector("button")?.getAttribute("aria-label")).toBe("Lock Subtitle Position");
  await act(() => root.render(<ControlButton icon="lock" label="Lock Subtitle Position" onClick={() => {}} />));
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe("Lock Subtitle Position");
});

it("follows pending labels and blocks duplicate actions while showing a busy indicator", async () => {
  const onClick = vi.fn();
  await act(async () => root.render(<ControlButton icon="play" label="Retry connection" onClick={onClick} />));
  await hover();
  await act(async () => root.render(<ControlButton icon="play" label="Connecting" busy onClick={onClick} />));
  const button = host.querySelector<HTMLButtonElement>("button")!;
  expect(button.disabled).toBe(true);
  expect(button.getAttribute("aria-busy")).toBe("true");
  expect(button.querySelector(".overlay-control-button__busy")).not.toBeNull();
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe("Connecting");
  await act(async () => button.click());
  expect(onClick).not.toHaveBeenCalled();
  await act(async () => root.render(<ControlButton icon="pause" label="Pause" disabled onClick={onClick} />));
  expect(button.disabled).toBe(true);
  expect(button.querySelector(".overlay-control-button__busy")).toBeNull();
});

it("opens on movement without an enter event, closes after a click and does not persist with pointer focus", async () => {
  const onClick = vi.fn();
  await act(async () => root.render(<ControlButton icon="gear" label="Open settings" onClick={onClick} />));
  const button = host.querySelector<HTMLButtonElement>("button")!;
  await act(async () => button.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
  expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
  expect(button.dataset.hovered).toBe("true");
  await act(async () => {
    button.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    button.focus();
    button.click();
  });
  expect(onClick).toHaveBeenCalledOnce();
  expect(document.activeElement).toBe(button);
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  await act(async () => button.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  await act(async () => button.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
  expect(button.dataset.hovered).toBeUndefined();
  await hover();
  expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
  await act(async () => button.dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: document.body })));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
});

it("uses native nonactivating hover targets without focusing or clicking the action", async () => {
  const onClick = vi.fn();
  await act(async () => root.render(<ControlButton icon="gear" label="Open settings" onClick={onClick} />));
  const button = host.querySelector<HTMLButtonElement>("button")!;
  const nativeHover = async (target: Element | null) => act(async () => document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: target })));
  await nativeHover(button.querySelector("svg"));
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe("Open settings");
  expect(button.dataset.hovered).toBe("true");
  expect(document.activeElement).not.toBe(button);
  expect(onClick).not.toHaveBeenCalled();
  await act(async () => button.click());
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  await nativeHover(button);
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  await nativeHover(null);
  expect(button.dataset.hovered).toBeUndefined();
  await nativeHover(button);
  expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
  await act(async () => window.dispatchEvent(new Event("blur")));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  expect(button.dataset.hovered).toBeUndefined();
});
