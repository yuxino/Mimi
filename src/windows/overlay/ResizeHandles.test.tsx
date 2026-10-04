// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResizeHandles } from "./ResizeHandles";

const runtime = vi.hoisted(() => ({ isTauri: false }));
vi.mock("../../lib/ipc", () => ({ get isTauri() { return runtime.isTauri; } }));
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const captureDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "setPointerCapture");
let host: HTMLDivElement;
let root: Root;
let onResize: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("screenX", 400); vi.stubGlobal("screenY", 300);
  vi.stubGlobal("screen", { width: 1512, availHeight: 958 });
  HTMLElement.prototype.getBoundingClientRect = () => new DOMRect(0, 0, 640, 482);
  HTMLElement.prototype.setPointerCapture = vi.fn();
  runtime.isTauri = false;
  invoke.mockReset().mockResolvedValue(undefined); onResize = vi.fn();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  if (captureDescriptor) Object.defineProperty(HTMLElement.prototype, "setPointerCapture", captureDescriptor);
  else Reflect.deleteProperty(HTMLElement.prototype, "setPointerCapture");
  vi.unstubAllGlobals();
});
async function mount(minimumHeight?: number) {
  await act(async () => root.render(<ResizeHandles disabled={false} onResize={onResize} minimumHeight={minimumHeight} />));
}
function topHandle(): HTMLElement {
  return [...host.querySelectorAll<HTMLElement>("div")]
    .find(node => node.style.cursor === "ns-resize" && node.style.top === "0px")!;
}
async function pointer(type: string, clientY: number) {
  await act(async () => {
    const event = new MouseEvent(type, { bubbles: true, clientX: 400, clientY });
    Object.defineProperty(event, "pointerId", { value: 1 });
    topHandle().dispatchEvent(event);
  });
}

function cornerHandle(vertical: "top" | "bottom", horizontal: "left" | "right"): HTMLElement {
  return [...host.querySelectorAll<HTMLElement>("div")]
    .find(node => node.style[vertical] === "0px" && node.style[horizontal] === "0px"
      && node.style.width === "14px" && node.style.height === "14px")!;
}

async function cornerPointer(handle: HTMLElement, type: string, clientX: number, clientY: number) {
  await act(async () => {
    const event = new MouseEvent(type, { bubbles: true, clientX, clientY, screenX: clientX + 400, screenY: clientY + 300 });
    Object.defineProperty(event, "pointerId", { value: 1 });
    handle.dispatchEvent(event);
  });
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
