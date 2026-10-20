// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { OverlayControlPanel } from "./OverlayControlPanel";
import { overlayControlPanelModel } from "./overlayControlModel";

const native = vi.hoisted(() => ({ reportHeight: vi.fn<() => Promise<void>>() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(),
  isTauri: true,
  overlayControlSetPanelHeight: native.reportHeight,
}));

let host: HTMLDivElement;
let root: Root;
let frameId = 0;
let frames: Map<number, FrameRequestCallback>;
let acknowledge: (() => void)[];
let observe: (() => void)[];
let contentHeight: number;
const originalScroll = Object.getOwnPropertyDescriptor(Element.prototype, "scrollIntoView");

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Windows NT 10.0");
  vi.stubGlobal("innerHeight", 30);
  vi.stubGlobal("devicePixelRatio", 1);
  frames = new Map();
  acknowledge = [];
  observe = [];
  contentHeight = 356;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = ++frameId;
    frames.set(id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { observe.push(callback); }
    observe() {}
    disconnect() {}
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return { height: this.classList.contains("overlay-control-header") ? 30 : contentHeight } as DOMRect;
  });
  Element.prototype.scrollIntoView = vi.fn();
  native.reportHeight.mockReset().mockImplementation(() => new Promise<void>(resolve => acknowledge.push(resolve)));
  setStoredUiLanguage("en");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  setStoredUiLanguage("system");
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  if (originalScroll) Object.defineProperty(Element.prototype, "scrollIntoView", originalScroll);
  else Reflect.deleteProperty(Element.prototype, "scrollIntoView");
});

async function mount() {
  const settings = { ...useStore.getState().settings, sourceLanguage: "auto" as const, targetLanguage: "zh" as const };
  const operation = async () => {};
  await act(async () => root.render(<OverlayControlPanel
    settings={settings} model={overlayControlPanelModel(settings)} phase="listening"
    status={{ source: "Automatic", separator: "→", target: "Chinese" }}
    isPaused={false} isWaitingForFinalTranslation={false} isChangingSession={false}
    onDismiss={() => {}} onSelectProfile={operation} onSwitchSourceLanguage={operation}
    onSwitchTargetLanguage={operation} onSetSkipTranslation={operation}
    onSetIntermediateSubtitles={operation} onSetSubtitleDividers={operation}
    onSetSubtitleTimestamps={operation} onSetSubtitleDisplayMode={operation}
    onSetImmersiveMode={operation} onSetOverlayLocked={operation} onShowSettings={operation}
  />));
}

async function nextFrame() {
  const pending = [...frames.values()];
  frames.clear();
  await act(async () => pending.forEach(callback => callback(0)));
}

it("keeps the island viewport from scrolling until the acknowledged Windows resize reaches the WebView", async () => {
  const focus = vi.spyOn(HTMLElement.prototype, "focus");
  await mount();
  await nextFrame();
  expect(focus).toHaveBeenCalledWith({ preventScroll: true });
  expect(native.reportHeight).toHaveBeenCalledExactlyOnceWith(356);
  await act(async () => acknowledge[0]());
  await nextFrame();
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  vi.stubGlobal("innerHeight", 356);
  window.dispatchEvent(new Event("resize"));
  await nextFrame();
  expect(Element.prototype.scrollIntoView).toHaveBeenCalledExactlyOnceWith({ block: "nearest" });
  window.dispatchEvent(new Event("resize"));
  await nextFrame();
  expect(Element.prototype.scrollIntoView).toHaveBeenCalledOnce();
  expect(native.reportHeight).toHaveBeenCalledOnce();
});

it.each(["Macintosh", "Linux x86_64"])("preserves ordinary focus and ignores DPI-only remeasurements on %s", async platform => {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(platform);
  const focus = vi.spyOn(HTMLElement.prototype, "focus");
  await mount();
  await nextFrame();
  expect(focus).toHaveBeenCalledWith();
  await act(async () => acknowledge[0]());
  vi.stubGlobal("innerHeight", 356);
  vi.stubGlobal("devicePixelRatio", 1.5);
  window.dispatchEvent(new Event("resize"));
  observe.forEach(callback => callback());
  await nextFrame();
  expect(native.reportHeight).toHaveBeenCalledOnce();
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  contentHeight = 400;
  observe.forEach(callback => callback());
  await nextFrame();
  expect(native.reportHeight).toHaveBeenLastCalledWith(400);
});

it("remeasures unchanged Windows content at new DPI and ignores an older resize completion", async () => {
  await mount();
  await nextFrame();
  vi.stubGlobal("devicePixelRatio", 1.5);
  vi.stubGlobal("innerHeight", 356);
  window.dispatchEvent(new Event("resize"));
  await nextFrame();
  expect(native.reportHeight).toHaveBeenCalledTimes(2);
  expect(native.reportHeight).toHaveBeenLastCalledWith(356);
  await act(async () => acknowledge[0]());
  await nextFrame();
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  await act(async () => acknowledge[1]());
  await nextFrame();
  expect(Element.prototype.scrollIntoView).toHaveBeenCalledOnce();
});

it("does not let a closed panel's delayed resize scroll a newly opened panel", async () => {
  await mount();
  await nextFrame();
  const oldObservers = [...observe];
  await act(async () => root.render(null));
  await mount();
  await nextFrame();
  vi.stubGlobal("innerHeight", 356);
  await act(async () => acknowledge[0]());
  oldObservers.forEach(callback => callback());
  await nextFrame();
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  expect(native.reportHeight).toHaveBeenCalledTimes(2);
  await act(async () => acknowledge[1]());
  await nextFrame();
  expect(Element.prototype.scrollIntoView).toHaveBeenCalledOnce();
});
