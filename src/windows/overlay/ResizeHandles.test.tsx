// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResizeHandles } from "./ResizeHandles";

vi.mock("../../lib/ipc", () => ({ isTauri: false }));
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
  invoke.mockReset(); onResize = vi.fn();
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
