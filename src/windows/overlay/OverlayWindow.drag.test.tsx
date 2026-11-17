// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { isClickablePointerTarget } from "../../lib/overlayPointer";
import { useStore } from "../../lib/store";
import { OverlayWindow } from "./OverlayWindow";

const native = vi.hoisted(() => ({ move: vi.fn(), readPointer: vi.fn(), wheel: vi.fn() }));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(),
  isTauri: true,
  hasEmbeddedOverlayControls: false,
  listenOverlayPointerMotion: async () => () => {},
  overlayMoveStart: native.move,
}));
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => <div data-testid="resize-edge" /> }));
vi.mock("./Timeline", () => ({ Timeline: () => <div className="overlay-timeline" data-testid="subtitle-content"
  onPointerDown={native.readPointer} onWheel={native.wheel}><span>Visible subtitle text</span></div> }));

const initial = useStore.getState();
let root: Root, host: HTMLDivElement;
const collapse = vi.fn();

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("innerWidth", 640);
  vi.stubGlobal("innerHeight", 136);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  setStoredUiLanguage("en");
  native.move.mockReset().mockResolvedValue(undefined);
  native.readPointer.mockReset();
  native.wheel.mockReset();
  collapse.mockReset().mockResolvedValue(undefined);
  useStore.setState({
    ...initial,
    setOverlayCollapsed: collapse,
    session: { ...initial.session, isActive: true, status: { kind: "listening" },
      subtitles: { ...initial.session.subtitles,
        history: [{ source: "Synthetic original.", translation: "Synthetic translation.", createdAt: 1 }] } },
    settings: { ...initial.settings, activeProfileId: "configured",
      profiles: [{ id: "configured", provider: "alibabaCloud", name: "Configured service", credentialState: "present" }],
      isOverlayLocked: false, subtitleBlendsWithBackground: false, pulseAnimation: false, subtitleAnimation: false },
  }, true);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  useStore.setState(initial, true);
  setStoredUiLanguage("system");
  vi.unstubAllGlobals();
});

async function mount() { await act(async () => root.render(<OverlayWindow />)); }
function area() { return host.querySelector<HTMLButtonElement>('[data-testid="overlay-drag-area"]'); }
async function press(target: Element, detail = 1, button = 0) {
  const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true, detail, button });
  await act(async () => target.dispatchEvent(event));
  return event;
}

async function pointerPress(target: Element, pointerType = "mouse", button = 0, clientX = 0) {
  const event = new MouseEvent("pointerdown", { bubbles: true, cancelable: true, button, clientX });
  Object.defineProperties(event, { pointerType: { value: pointerType }, isPrimary: { value: true } });
  await act(async () => target.dispatchEvent(event));
  return event;
}

it("moves from empty top chrome without focusing or folding on double-click", async () => {
  await mount();
  const drag = area()!;
  expect(drag.tabIndex).toBe(-1);
  expect(drag.getAttribute("aria-hidden")).toBe("true");
  expect(isClickablePointerTarget(drag)).toBe(false);
  expect((await press(drag)).defaultPrevented).toBe(true);
  expect(document.activeElement).not.toBe(drag);
  expect(native.move).toHaveBeenCalledOnce();
  expect(collapse).not.toHaveBeenCalled();
  await press(drag, 2);
  expect(collapse).not.toHaveBeenCalled();
  expect(native.move).toHaveBeenCalledTimes(2);
  await press(drag, 1, 1);
  await press(drag, 1, 2);
  expect(native.move).toHaveBeenCalledTimes(2);
});

it("does not turn toolbar controls or resize edges into drag targets", async () => {
  await mount();
  for (const selector of [
    `button[aria-label="${I18N.overlay.pause}"]`,
    '[data-testid="collapse-subtitles"]',
    '[data-testid="resize-edge"]',
  ]) {
    const target = host.querySelector(selector)!;
    expect(target, selector).not.toBeNull();
    await press(target);
    await press(target, 2);
    expect((await pointerPress(target)).defaultPrevented).toBe(false);
  }
  expect(native.move).not.toHaveBeenCalled();
  expect(collapse).not.toHaveBeenCalled();
});

it("moves from the canvas and subtitle text without starting history reading or changing display mode", async () => {
  await mount();
  const canvas = host.querySelector(".overlay-expanded-canvas")!;
  const caption = host.querySelector('[data-testid="subtitle-content"] span')!;
  expect((await pointerPress(canvas)).defaultPrevented).toBe(true);
  expect((await pointerPress(caption)).defaultPrevented).toBe(true);
  expect(native.move).toHaveBeenCalledTimes(2);
  expect(native.readPointer).not.toHaveBeenCalled();
  expect(collapse).not.toHaveBeenCalled();

  const wheel = new WheelEvent("wheel", { bubbles: true, cancelable: true, deltaY: -40 });
  await act(async () => caption.dispatchEvent(wheel));
  expect(wheel.defaultPrevented).toBe(false);
  expect(native.wheel).toHaveBeenCalledOnce();
  await pointerPress(caption, "touch");
  await pointerPress(caption, "mouse", 2);
  expect(native.move).toHaveBeenCalledTimes(2);
});

it("lets the visible timeline scrollbar retain its pointer gesture", async () => {
  await mount();
  const timeline = host.querySelector<HTMLElement>(".overlay-timeline")!;
  Object.defineProperties(timeline, {
    scrollHeight: { value: 400 }, clientHeight: { value: 100 },
    clientWidth: { value: 300 }, offsetWidth: { value: 300 },
  });
  vi.spyOn(timeline, "getBoundingClientRect").mockReturnValue({ right: 340 } as DOMRect);
  expect((await pointerPress(timeline, "mouse", 0, 338)).defaultPrevented).toBe(false);
  expect(native.readPointer).toHaveBeenCalledOnce();
  expect(native.move).not.toHaveBeenCalled();
  expect((await pointerPress(timeline, "mouse", 0, 200)).defaultPrevented).toBe(true);
  expect(native.move).toHaveBeenCalledOnce();
});

it("moves from the empty listening state without a top-only hit area", async () => {
  useStore.setState(state => ({ session: { ...state.session, subtitles: initial.session.subtitles } }));
  await mount();
  const listening = Array.from(host.querySelectorAll("div")).find(element =>
    element.children.length === 0 && element.textContent === "Listening")!;
  expect(listening).toBeDefined();
  expect((await pointerPress(listening)).defaultPrevented).toBe(true);
  expect(native.move).toHaveBeenCalledOnce();
});

it("uses the explicit collapse button and guards the drag while folding", async () => {
  let resolve!: () => void;
  collapse.mockImplementationOnce(() => new Promise<void>(finish => { resolve = finish; }));
  await mount();
  await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="collapse-subtitles"]')!.click());
  expect(area()!.disabled).toBe(true);
  expect(isClickablePointerTarget(area())).toBe(false);
  await press(area()!);
  await press(area()!, 2);
  await pointerPress(host.querySelector(".overlay-expanded-canvas")!);
  expect(native.move).not.toHaveBeenCalled();
  expect(collapse).toHaveBeenCalledOnce();
  await act(async () => resolve());
  await press(area()!);
  expect(native.move).toHaveBeenCalledOnce();
});

it.each(["locked", "immersive"] as const)("keeps the empty-top gesture unavailable in %s mode", async mode => {
  useStore.setState(state => ({
    settings: { ...state.settings, isOverlayLocked: mode === "locked", subtitleBlendsWithBackground: mode === "immersive" },
  }));
  await mount();
  if (mode === "locked") {
    expect(area()!.disabled).toBe(true);
    expect(isClickablePointerTarget(area())).toBe(false);
    await press(area()!);
    expect((await pointerPress(host.querySelector(".overlay-expanded-canvas")!)).defaultPrevented).toBe(false);
  } else {
    expect(area()).toBeNull();
  }
  expect(native.move).not.toHaveBeenCalled();
  expect(collapse).not.toHaveBeenCalled();
});

it("allows error-state movement without collapsing or changing saved lock and immersive preferences", async () => {
  useStore.setState(state => ({
    settings: { ...state.settings, isOverlayLocked: true, subtitleBlendsWithBackground: true },
    session: { ...state.session, isActive: false, status: { kind: "error", message: "unavailable" } },
  }));
  await mount();
  expect(area()!.disabled).toBe(false);
  await press(area()!);
  await press(area()!, 2);
  expect(native.move).toHaveBeenCalledTimes(2);
  expect(collapse).not.toHaveBeenCalled();
  expect(useStore.getState().settings).toMatchObject({ isOverlayLocked: true, subtitleBlendsWithBackground: true });
});

it("lets the compact status area move without folding and expands from its button", async () => {
  useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: true } }));
  await mount();
  await press(area()!, 2);
  expect(native.move).toHaveBeenCalledOnce();
  expect(collapse).not.toHaveBeenCalled();
  await act(async () => host.querySelector<HTMLButtonElement>('[data-testid="expand-subtitles"]')!.click());
  expect(collapse).toHaveBeenCalledExactlyOnceWith(false);
});
