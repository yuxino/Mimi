// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { createPointerCursorUpdater, isClickablePointerTarget, OVERLAY_POINTER_TARGET_EVENT, publishOverlayPointerMotion } from "./overlayPointer";

afterEach(() => {
  publishOverlayPointerMotion(null);
  vi.restoreAllMocks();
});

it("points at a clickable icon's button, but not disabled, busy or plain content", () => {
  const button = document.createElement("button");
  const icon = document.createElement("span");
  button.append(icon);
  expect(isClickablePointerTarget(icon)).toBe(true);
  button.disabled = true;
  expect(isClickablePointerTarget(icon)).toBe(false);
  button.disabled = false;
  button.setAttribute("aria-busy", "true");
  expect(isClickablePointerTarget(icon)).toBe(false);
  expect(isClickablePointerTarget(document.createElement("div"))).toBe(false);
});

it("reasserts the hand on movement after success, coalesces pending samples and never retries while stationary", async () => {
  const resolutions: Array<(accepted: boolean) => void> = [];
  const send = vi.fn(() => new Promise<boolean>(resolve => resolutions.push(resolve)));
  const update = createPointerCursorUpdater(send);
  update({ x: 1, y: 1 }, true);
  for (let x = 2; x <= 50; x++) update({ x, y: 1 }, true);
  expect(send).toHaveBeenCalledOnce();
  resolutions[0](false);
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(2);
  expect(send).toHaveBeenLastCalledWith({ x: 50, y: 1 }, true);
  resolutions[1](true);
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  update({ x: 51, y: 1 }, true);
  // WebKit can replace the accepted hand during a later native move.
  expect(send).toHaveBeenCalledTimes(3);
  expect(send).toHaveBeenLastCalledWith({ x: 51, y: 1 }, true);
  update({ x: 52, y: 1 }, false);
  expect(send).toHaveBeenCalledTimes(3);
  resolutions[2](true);
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(4);
  expect(send).toHaveBeenLastCalledWith({ x: 52, y: 1 }, false);
  resolutions[3](false);
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(4); // No stationary failure loop.
  update(null);
  update({ x: 53, y: 1 }, true);
  expect(send).toHaveBeenCalledTimes(5);
  resolutions[4](true);
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(5); // No stationary success loop either.
});

it("uses the hand for selectable controls and disclosure, while preserving text and resize targets", () => {
  const host = document.createElement("div");
  host.innerHTML = '<details><summary><span>Details</span></summary></details><div role="option"><span>Choice</span></div><input type="color"><input type="search" role="combobox"><input type="range"><textarea></textarea><div style="cursor: ew-resize"></div><fieldset disabled><button>Unavailable</button></fieldset>';
  for (const selector of ['summary span', '[role="option"] span', 'input[type="color"]']) {
    expect(isClickablePointerTarget(host.querySelector(selector))).toBe(true);
  }
  const option = host.querySelector('[role="option"]')!;
  option.setAttribute("aria-disabled", "true");
  expect(isClickablePointerTarget(option.firstElementChild)).toBe(false);
  for (const selector of ['input[type="search"]', 'input[type="range"]', 'textarea', '[style]', 'fieldset button']) {
    expect(isClickablePointerTarget(host.querySelector(selector))).toBe(false);
  }
});

it("does not let a late hand response override native exit and a fresh entry", async () => {
  const resolutions: Array<(accepted: boolean) => void> = [];
  const send = vi.fn(() => new Promise<boolean>(resolve => resolutions.push(resolve)));
  const update = createPointerCursorUpdater(send);
  update({ x: 1, y: 1 }, true);
  update(null);
  update({ x: 9, y: 9 }, true);
  resolutions[0](true); // It succeeded before native exit, but the reply was late.
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(send).toHaveBeenCalledTimes(2);
  expect(send).toHaveBeenLastCalledWith({ x: 9, y: 9 }, true);
  resolutions[1](true);
});

it("hit tests once per motion and notifies only changed targets, including native leave", () => {
  const button = document.createElement("button");
  const previous = Object.getOwnPropertyDescriptor(document, "elementFromPoint");
  const hitTest = vi.fn(() => button);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: hitTest });
  const changed = vi.fn();
  document.addEventListener(OVERLAY_POINTER_TARGET_EVENT, changed);
  try {
    publishOverlayPointerMotion({ x: 10, y: 20 });
    publishOverlayPointerMotion({ x: 11, y: 21 });
    expect(hitTest).toHaveBeenCalledTimes(2);
    expect(changed).toHaveBeenCalledOnce();
    expect((changed.mock.calls[0][0] as CustomEvent).detail).toBe(button);
    publishOverlayPointerMotion(null);
    publishOverlayPointerMotion(null);
    expect(hitTest).toHaveBeenCalledTimes(2);
    expect(changed).toHaveBeenCalledTimes(2);
    expect((changed.mock.calls[1][0] as CustomEvent).detail).toBeNull();
    publishOverlayPointerMotion({ x: Number.NaN, y: 20 });
    publishOverlayPointerMotion({ x: window.innerWidth, y: 20 });
    expect(hitTest).toHaveBeenCalledTimes(2);
    expect(changed).toHaveBeenCalledTimes(2);
  } finally {
    document.removeEventListener(OVERLAY_POINTER_TARGET_EVENT, changed);
    if (previous) Object.defineProperty(document, "elementFromPoint", previous);
    else Reflect.deleteProperty(document, "elementFromPoint");
  }
});
