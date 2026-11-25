// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { OVERLAY_POINTER_TARGET_EVENT } from "../../../lib/overlay/overlayPointer";
import { Tooltip } from "./Tooltip";

let host: HTMLDivElement;
let root: Root;
let anchorTop: number;
let resized: (() => void)[];
const disconnected = vi.fn();
const geometry = vi.fn();

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  anchorTop = 100;
  resized = [];
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { resized.push(callback); }
    observe() {}
    disconnect = disconnected;
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    geometry();
    return this.classList.contains("mimi-tooltip")
      ? { left: 0, top: 0, right: 80, bottom: 20, width: 80, height: 20 } as DOMRect
      : { left: 100, top: anchorTop, right: 124, bottom: anchorTop + 24, width: 24, height: 24 } as DOMRect;
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

afterEach(async () => {
  await act(() => root.unmount()); host.remove();
  vi.unstubAllGlobals(); vi.restoreAllMocks();
  geometry.mockClear(); disconnected.mockClear();
});

async function show() {
  await act(() => root.render(<><div id="ancestor"><Tooltip label="Pause captions">
    {id => <button aria-describedby={id}>Pause</button>}
  </Tooltip></div><div id="timeline" /><button id="other">Other action</button></>));
  await act(() => host.querySelector("button")!.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
  return document.querySelector<HTMLElement>('[role="tooltip"]')!;
}

const move = (from: HTMLElement, to: HTMLElement) => act(() => {
  from.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: to }));
  to.dispatchEvent(new MouseEvent("mouseover", { bubbles: true, relatedTarget: from }));
});

it("keeps pointer access across the gap and through the portaled help", async () => {
  const popup = await show();
  const trigger = host.querySelector("button")!;
  expect(popup.style.getPropertyValue("--tooltip-bridge-width")).toBe("24px");
  expect(popup.style.getPropertyValue("--tooltip-bridge-height")).toBe("4px");
  await move(trigger, popup);
  expect(popup.isConnected).toBe(true);
  await move(popup, trigger);
  expect(popup.isConnected).toBe(true);
  await move(trigger, popup);
  await move(popup, document.body);
  expect(popup.isConnected).toBe(false);
});

it.each(["DOM", "native relay"])("opens on the first return after Escape removes the hovered popup (%s)", async input => {
  const popup = await show();
  const trigger = host.querySelector("button")!;
  await act(() => trigger.focus());
  if (input === "DOM") await move(trigger, popup);
  else await act(() => document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: popup })));
  expect(popup.isConnected).toBe(true);
  await act(() => trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(popup.isConnected).toBe(false);
  expect(document.activeElement).toBe(trigger);
  // The pointer leaves from the removed popup. Its leave handler no longer
  // exists by the time the browser reports movement outside.
  await act(() => document.body.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
  await act(() => trigger.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
  expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
  expect(document.activeElement).toBe(trigger);
});

it("dismisses hover help before its containing panel without moving unrelated focus", async () => {
  const popup = await show();
  const trigger = host.querySelector("button")!;
  const other = host.querySelector<HTMLButtonElement>("#other")!;
  await act(() => other.focus());
  const raisedEscape = vi.fn();
  window.addEventListener("keydown", raisedEscape);
  const escape = (isComposing = false) => other.dispatchEvent(new KeyboardEvent("keydown", {
    key: "Escape", bubbles: true, cancelable: true, isComposing,
  }));
  try {
    other.addEventListener("keydown", (event) => event.preventDefault(), { once: true });
    await act(() => { escape(); });
    expect(popup.isConnected).toBe(true);
    await act(() => { escape(true); });
    expect(popup.isConnected).toBe(true);
    expect(raisedEscape).toHaveBeenCalledTimes(2);
    raisedEscape.mockClear();
    await act(() => { escape(); });
    expect(popup.isConnected).toBe(false);
    expect(document.activeElement).toBe(other);
    expect(raisedEscape).not.toHaveBeenCalled();
    await act(() => trigger.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    await act(() => { escape(); });
    expect(raisedEscape).toHaveBeenCalledOnce();
    await act(() => trigger.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: other })));
    await act(() => trigger.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
    expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
  } finally {
    window.removeEventListener("keydown", raisedEscape);
  }
});

it("keeps keyboard-open help available after the pointer moves away", async () => {
  const popup = await show();
  const trigger = host.querySelector("button")!;
  // jsdom 29 no longer reports script-driven focus as :focus-visible; browsers do for keyboard use.
  const matches = Element.prototype.matches;
  vi.spyOn(Element.prototype, "matches").mockImplementation(function (this: Element, selector: string) {
    return selector === ":focus-visible" ? this === document.activeElement : matches.call(this, selector);
  });
  await act(() => trigger.focus());
  await act(() => trigger.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
  expect(document.activeElement).toBe(trigger);
  expect(popup.isConnected).toBe(true);
  await act(() => trigger.blur());
  expect(popup.isConnected).toBe(false);
});

it("skips unrelated subtitle scrolling and follows a scrolling ancestor", async () => {
  const popup = await show();
  const initialTop = popup.style.top;
  geometry.mockClear();
  anchorTop = 70;
  for (let index = 0; index < 100; index++) host.querySelector("#timeline")!.dispatchEvent(new Event("scroll"));
  expect(geometry).not.toHaveBeenCalled();
  expect(popup.style.top).toBe(initialTop);

  host.querySelector("#ancestor")!.dispatchEvent(new Event("scroll"));
  expect(geometry).toHaveBeenCalledTimes(2);
  expect(popup.style.top).not.toBe(initialTop);
  expect(host.querySelector("button")!.getAttribute("aria-describedby")).toBe(popup.id);
});

it.each(["document scroll", "window scroll", "window resize", "observer resize"])("keeps tooltip positioning current on %s", async event => {
  const popup = await show();
  const initialTop = popup.style.top;
  geometry.mockClear();
  anchorTop = 60;
  if (event === "document scroll") document.dispatchEvent(new Event("scroll"));
  else if (event === "window scroll") window.dispatchEvent(new Event("scroll"));
  else if (event === "window resize") window.dispatchEvent(new Event("resize"));
  else resized.forEach(callback => callback());
  expect(geometry).toHaveBeenCalledTimes(2);
  expect(popup.style.top).not.toBe(initialTop);
});

it("removes geometry observers and scroll listeners when dismissed", async () => {
  await show();
  await act(() => window.dispatchEvent(new Event("blur")));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  expect(disconnected).toHaveBeenCalledOnce();
  geometry.mockClear();
  host.querySelector("#ancestor")!.dispatchEvent(new Event("scroll"));
  document.dispatchEvent(new Event("scroll"));
  window.dispatchEvent(new Event("scroll"));
  window.dispatchEvent(new Event("resize"));
  expect(geometry).not.toHaveBeenCalled();
});
