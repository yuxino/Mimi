// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
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
  </Tooltip></div><div id="timeline" /></>));
  await act(() => host.querySelector("button")!.dispatchEvent(new MouseEvent("mousemove", { bubbles: true })));
  return document.querySelector<HTMLElement>('[role="tooltip"]')!;
}

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
