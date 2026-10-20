// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ResizeHandles as ResizeHandlesComponent } from "./ResizeHandles";

const runtime = vi.hoisted(() => ({ isTauri: false }));
vi.mock("../../lib/ipc", () => ({ get isTauri() { return runtime.isTauri; } }));
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const captureDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "setPointerCapture");
const releaseDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "releasePointerCapture");
let host: HTMLDivElement;
let root: Root;
let onResize: ReturnType<typeof vi.fn>;
let ResizeHandles: typeof ResizeHandlesComponent;

beforeEach(async () => {
  // A fresh module represents an independent WebView for each test. Remounts
  // within a test intentionally share their dispatcher and pending requests.
  vi.resetModules();
  ({ ResizeHandles } = await import("./ResizeHandles"));
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("screenX", 400); vi.stubGlobal("screenY", 300);
  vi.stubGlobal("screen", { width: 1512, availHeight: 958 });
  HTMLElement.prototype.getBoundingClientRect = () => new DOMRect(0, 0, 640, 482);
  HTMLElement.prototype.setPointerCapture = vi.fn();
  HTMLElement.prototype.releasePointerCapture = vi.fn();
  runtime.isTauri = false;
  invoke.mockReset().mockResolvedValue(undefined); onResize = vi.fn();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  if (captureDescriptor) Object.defineProperty(HTMLElement.prototype, "setPointerCapture", captureDescriptor);
  else Reflect.deleteProperty(HTMLElement.prototype, "setPointerCapture");
  if (releaseDescriptor) Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", releaseDescriptor);
  else Reflect.deleteProperty(HTMLElement.prototype, "releasePointerCapture");
  vi.unstubAllGlobals();
});
async function mount(minimumHeight?: number, disabled = false) {
  await act(async () => root.render(<ResizeHandles disabled={disabled} onResize={onResize} minimumHeight={minimumHeight} />));
}
function topHandle(): HTMLElement {
  return [...host.querySelectorAll<HTMLElement>("div")]
    .find(node => node.style.cursor === "ns-resize" && node.style.top === "0px")!;
}
async function pointer(type: string, clientY: number) {
  await dispatchPointer(topHandle(), type, { clientX: 400, clientY });
}

function cornerHandle(vertical: "top" | "bottom", horizontal: "left" | "right"): HTMLElement {
  return [...host.querySelectorAll<HTMLElement>("div")]
    .find(node => node.style[vertical] === "0px" && node.style[horizontal] === "0px"
      && node.style.width === "14px" && node.style.height === "14px")!;
}

async function cornerPointer(handle: HTMLElement, type: string, clientX: number, clientY: number) {
  await dispatchPointer(handle, type, { clientX, clientY, screenX: clientX + 400, screenY: clientY + 300 });
}

async function dispatchPointer(target: EventTarget, type: string, init: MouseEventInit = {}, pointerId = 1, isPrimary = true) {
  await act(async () => target.dispatchEvent(pointerEvent(type, init, pointerId, isPrimary)));
}

function pointerEvent(type: string, init: MouseEventInit = {}, pointerId = 1, isPrimary = true) {
  const event = new MouseEvent(type, { bubbles: true, buttons: type === "pointerdown" || type === "pointermove" ? 1 : 0, ...init });
  Object.defineProperties(event, { pointerId: { value: pointerId }, isPrimary: { value: isPrimary } });
  return event;
}

it.each([136, 188, 200])("clamps browser top-edge resizing to %i px while preserving the opposite edge", async minimumHeight => {
  await mount(minimumHeight);
  await pointer("pointerdown", 300); await pointer("pointermove", 900);
  expect(onResize).toHaveBeenLastCalledWith(640, minimumHeight, 400, 782 - minimumHeight);
  expect(invoke).not.toHaveBeenCalled();
});

it("uses a new source requirement during an already active resize gesture", async () => {
  await mount(136); await pointer("pointerdown", 300);
  await mount(200); await pointer("pointermove", 900);
  expect(onResize).toHaveBeenLastCalledWith(640, 200, 400, 582);
});

it("preserves the fitting single-source minimum when no derived override is supplied", async () => {
  await mount(); await pointer("pointerdown", 300); await pointer("pointermove", 900);
  expect(onResize).toHaveBeenLastCalledWith(640, 136, 400, 646);
});

it.each([
  { vertical: "top", horizontal: "left", expected: [590, 452, 450, 330] },
  { vertical: "top", horizontal: "right", expected: [690, 452, 400, 330] },
  { vertical: "bottom", horizontal: "left", expected: [590, 512, 450, 300] },
  { vertical: "bottom", horizontal: "right", expected: [690, 512, 400, 300] },
] as const)("resizes both dimensions from the $vertical-$horizontal browser corner and anchors the opposite corner", async ({ vertical, horizontal, expected }) => {
  await mount();
  const handle = cornerHandle(vertical, horizontal);
  await cornerPointer(handle, "pointerdown", 400, 300);
  await cornerPointer(handle, "pointermove", 450, 330);
  expect(onResize).toHaveBeenLastCalledWith(...expected);
  expect(invoke).not.toHaveBeenCalled();
  await cornerPointer(handle, "pointerup", 450, 330);
  await cornerPointer(handle, "pointermove", 500, 360);
  expect(onResize).toHaveBeenCalledOnce();
});

it.each([
  { vertical: "top", horizontal: "left", x: 1400, y: 1300, expected: [360, 188, 680, 594] },
  { vertical: "top", horizontal: "right", x: -600, y: 1300, expected: [360, 188, 400, 594] },
  { vertical: "bottom", horizontal: "left", x: 1400, y: -700, expected: [360, 188, 680, 300] },
  { vertical: "bottom", horizontal: "right", x: -600, y: -700, expected: [360, 188, 400, 300] },
] as const)("clamps both dimensions at the $vertical-$horizontal browser corner without shifting the opposite corner", async ({ vertical, horizontal, x, y, expected }) => {
  await mount(188);
  const handle = cornerHandle(vertical, horizontal);
  await cornerPointer(handle, "pointerdown", 400, 300);
  await cornerPointer(handle, "pointermove", x, y);
  expect(onResize).toHaveBeenLastCalledWith(...expected);
});

it("still forwards the original native corner name and screen positions without browser resize math", async () => {
  runtime.isTauri = true;
  await mount();
  const handle = cornerHandle("top", "left");
  await cornerPointer(handle, "pointerdown", 400, 300);
  await cornerPointer(handle, "pointermove", 450, 330);
  await cornerPointer(handle, "pointerup", 450, 330);
  expect(invoke.mock.calls).toEqual([
    ["resize_start", { region: "topLeft", x: 800, y: 600 }],
    ["resize_move", { x: 850, y: 630 }],
    ["resize_end"],
  ]);
  expect(onResize).not.toHaveBeenCalled();
});

it.each([false, true])("finishes a capture-lost drag without forwarding later hover movement (native=%s)", async native => {
  runtime.isTauri = native;
  await mount();
  const handle = cornerHandle("top", "left");
  await cornerPointer(handle, "pointerdown", 400, 300);
  await dispatchPointer(handle, "lostpointercapture");
  await cornerPointer(handle, "pointermove", 450, 330);
  expect(onResize).not.toHaveBeenCalled();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(native ? ["resize_start", "resize_end"] : []);
  expect(HTMLElement.prototype.releasePointerCapture).toHaveBeenCalledWith(1);
});

it.each([false, true])("finishes when the owning pointer moves with its primary button released (native=%s)", async native => {
  runtime.isTauri = native;
  await mount();
  const handle = cornerHandle("top", "left");
  await cornerPointer(handle, "pointerdown", 400, 300);
  await dispatchPointer(handle, "pointermove", { buttons: 0, clientX: 450, clientY: 330 });
  await cornerPointer(handle, "pointermove", 500, 360);
  expect(onResize).not.toHaveBeenCalled();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(native ? ["resize_start", "resize_end"] : []);
});

it.each([false, true])("cleans up an active drag on disable and permits a fresh drag after reenable (native=%s)", async native => {
  runtime.isTauri = native;
  await mount();
  const handle = cornerHandle("top", "left");
  await cornerPointer(handle, "pointerdown", 400, 300);
  await mount(undefined, true);
  await cornerPointer(handle, "pointermove", 450, 330);
  await cornerPointer(handle, "pointerdown", 450, 330);
  expect(onResize).not.toHaveBeenCalled();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(native ? ["resize_start", "resize_end"] : []);
  await mount();
  await cornerPointer(handle, "pointerdown", 400, 300);
  await cornerPointer(handle, "pointermove", 450, 330);
  if (native) expect(invoke.mock.calls.map(([command]) => command)).toEqual(["resize_start", "resize_end", "resize_start", "resize_move"]);
  else expect(onResize).toHaveBeenCalledExactlyOnceWith(590, 452, 450, 330);
});

it.each([false, true])("cleans up capture and native state before handles unmount (native=%s)", async native => {
  runtime.isTauri = native;
  await mount();
  await cornerPointer(cornerHandle("top", "left"), "pointerdown", 400, 300);
  await act(async () => root.render(null));
  await dispatchPointer(window, "pointerup");
  await act(async () => window.dispatchEvent(new Event("blur")));
  expect(HTMLElement.prototype.releasePointerCapture).toHaveBeenCalledExactlyOnceWith(1);
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(native ? ["resize_start", "resize_end"] : []);
});

it.each([false, true].flatMap(native => ["pointerup", "pointercancel", "blur"].map(type => ({ native, type }))))("ends drags on window $type when capture is unavailable (native=$native)", async ({ native, type }) => {
  runtime.isTauri = native;
  HTMLElement.prototype.setPointerCapture = vi.fn(() => { throw new Error("capture unavailable"); });
  await mount();
  const handle = cornerHandle("top", "left");
  await cornerPointer(handle, "pointerdown", 400, 300);
  if (type === "blur") await act(async () => window.dispatchEvent(new Event(type)));
  else await dispatchPointer(window, type);
  await cornerPointer(handle, "pointermove", 450, 330);
  expect(onResize).not.toHaveBeenCalled();
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(native ? ["resize_start", "resize_end"] : []);
});

it.each([false, true])("keeps a drag owned by its original pointer and ignores other-pointer releases (native=%s)", async native => {
  runtime.isTauri = native;
  await mount();
  const handle = cornerHandle("top", "left");
  await cornerPointer(handle, "pointerdown", 400, 300);
  // Another device can have its own primary pointer; it still cannot take over.
  await dispatchPointer(handle, "pointerdown", { clientX: 800, clientY: 800 }, 2);
  await dispatchPointer(handle, "pointermove", { clientX: 800, clientY: 800 }, 2);
  await dispatchPointer(window, "pointerup", {}, 2, false);
  await dispatchPointer(window, "pointercancel", {}, 2, false);
  await dispatchPointer(handle, "lostpointercapture", {}, 2, false);
  await cornerPointer(handle, "pointermove", 450, 330);
  if (native) expect(invoke.mock.calls).toEqual([
    ["resize_start", { region: "topLeft", x: 800, y: 600 }],
    ["resize_move", { x: 850, y: 630 }],
  ]);
  else expect(onResize).toHaveBeenCalledExactlyOnceWith(590, 452, 450, 330);
  await cornerPointer(handle, "pointerup", 450, 330);
});

it.each([false, true])("ignores right-click and nonprimary starts (native=%s)", async native => {
  runtime.isTauri = native;
  await mount();
  const handle = cornerHandle("top", "left");
  await dispatchPointer(handle, "pointerdown", { button: 2, buttons: 2 });
  await cornerPointer(handle, "pointermove", 450, 330);
  await dispatchPointer(handle, "pointerdown", {}, 2, false);
  await dispatchPointer(handle, "pointermove", { clientX: 450, clientY: 330 }, 2, false);
  expect(invoke).not.toHaveBeenCalled();
  expect(onResize).not.toHaveBeenCalled();
  expect(HTMLElement.prototype.setPointerCapture).not.toHaveBeenCalled();
});

it.each(["pointerup", "pointercancel", "blur"])("ends a native drag once across %s, capture loss, disable and unmount", async type => {
  runtime.isTauri = true;
  await mount();
  const handle = cornerHandle("top", "left");
  HTMLElement.prototype.releasePointerCapture = vi.fn(() => {
    const event = new MouseEvent("lostpointercapture", { bubbles: true });
    Object.defineProperty(event, "pointerId", { value: 1 });
    handle.dispatchEvent(event);
  });
  await cornerPointer(handle, "pointerdown", 400, 300);
  if (type === "blur") await act(async () => window.dispatchEvent(new Event(type)));
  else await dispatchPointer(window, type);
  await dispatchPointer(handle, "lostpointercapture");
  await dispatchPointer(window, "pointerup");
  await act(async () => window.dispatchEvent(new Event("blur")));
  await mount(undefined, true);
  await act(async () => root.render(null));
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["resize_start", "resize_end"]);
  expect(HTMLElement.prototype.releasePointerCapture).toHaveBeenCalledExactlyOnceWith(1);
});

it("does not notify native resize_end for ordinary capture loss without a resize owner", async () => {
  runtime.isTauri = true;
  await mount();
  await dispatchPointer(cornerHandle("top", "left"), "lostpointercapture");
  await dispatchPointer(window, "pointerup");
  await act(async () => window.dispatchEvent(new Event("blur")));
  expect(invoke).not.toHaveBeenCalled();
});

it.each(["resize_start", "resize_move", "resize_end"])("keeps cleanup safe when %s throws synchronously", async failedCommand => {
  runtime.isTauri = true;
  invoke.mockImplementation((command: string) => {
    if (command === failedCommand) throw new Error("bridge unavailable");
    return Promise.resolve();
  });
  await mount();
  const handle = cornerHandle("top", "left");
  await cornerPointer(handle, "pointerdown", 400, 300);
  await cornerPointer(handle, "pointermove", 450, 330);
  await cornerPointer(handle, "pointerup", 450, 330);
  await dispatchPointer(handle, "lostpointercapture");
  await act(async () => root.render(null));
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(failedCommand === "resize_start"
    ? ["resize_start", "resize_end"]
    : failedCommand === "resize_move"
      ? ["resize_start", "resize_move", "resize_end"]
      : ["resize_start", "resize_move", "resize_end", "resize_end"]);
  expect(HTMLElement.prototype.releasePointerCapture).toHaveBeenCalledExactlyOnceWith(1);
});

it("contains rejected IPC promises while retaining a single cleanup", async () => {
  runtime.isTauri = true;
  invoke.mockRejectedValue(new Error("bridge unavailable"));
  await mount();
  const handle = cornerHandle("top", "left");
  await cornerPointer(handle, "pointerdown", 400, 300);
  await cornerPointer(handle, "pointermove", 450, 330);
  await cornerPointer(handle, "pointerup", 450, 330);
  await dispatchPointer(handle, "lostpointercapture");
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["resize_start", "resize_end"]);
});

it("bounds real pointer bursts and keeps the previous mount's end ahead of a new native gesture", async () => {
  runtime.isTauri = true;
  const acknowledgements: Array<() => void> = [];
  let inFlight = 0;
  let peakInFlight = 0;
  invoke.mockImplementation(() => new Promise<void>(resolve => {
    peakInFlight = Math.max(peakInFlight, ++inFlight);
    acknowledgements.push(() => { inFlight--; resolve(); });
  }));
  const acknowledge = async (index: number) => act(async () => acknowledgements[index]());
  await mount();
  const old = cornerHandle("top", "left");
  await cornerPointer(old, "pointerdown", 400, 300);
  await act(async () => {
    for (let i = 0; i < 120; i++) old.dispatchEvent(pointerEvent("pointermove", { screenX: 801 + i, screenY: 601 + i }));
  });
  expect(invoke).toHaveBeenCalledOnce();
  await act(async () => root.render(null));
  await mount();
  const replacement = cornerHandle("bottom", "right");
  await cornerPointer(replacement, "pointerdown", 500, 400);
  await cornerPointer(replacement, "pointermove", 550, 450);
  expect(invoke).toHaveBeenCalledOnce();
  await acknowledge(0);
  expect(invoke.mock.lastCall).toEqual(["resize_move", { x: 920, y: 720 }]);
  await acknowledge(1);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  expect(invoke).toHaveBeenCalledTimes(3);
  await acknowledge(2);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "bottomRight", x: 900, y: 700 }]);
  await acknowledge(3);
  expect(invoke.mock.lastCall).toEqual(["resize_move", { x: 950, y: 750 }]);
  await cornerPointer(replacement, "pointerup", 550, 450);
  await acknowledge(4);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  await acknowledge(5);
  expect(inFlight).toBe(0);
  expect(peakInFlight).toBe(1);
  expect(onResize).not.toHaveBeenCalled();
});
