// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { isClickablePointerTarget } from "../../lib/overlayPointer";
import { useStore } from "../../lib/store";
import { OverlayWindow } from "./OverlayWindow";

const native = vi.hoisted(() => ({ move: vi.fn() }));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(),
  isTauri: true,
  hasEmbeddedOverlayControls: false,
  listenOverlayPointerMotion: async () => () => {},
  overlayMoveStart: native.move,
}));
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => <div data-testid="resize-edge" /> }));
vi.mock("./Timeline", () => ({ Timeline: () => <div data-testid="subtitle-content" /> }));

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

it("moves from empty top chrome without focusing and keeps double-click collapse", async () => {
  await mount();
  const drag = area()!;
  expect(drag.tabIndex).toBe(-1);
  expect(drag.getAttribute("aria-hidden")).toBe("true");
  expect(isClickablePointerTarget(drag)).toBe(true);
  expect((await press(drag)).defaultPrevented).toBe(true);
  expect(document.activeElement).not.toBe(drag);
  expect(native.move).toHaveBeenCalledOnce();
  expect(collapse).not.toHaveBeenCalled();
  await press(drag, 2);
  expect(collapse).toHaveBeenCalledExactlyOnceWith(true);
  expect(native.move).toHaveBeenCalledOnce();
  await press(drag, 1, 1);
  await press(drag, 1, 2);
  expect(native.move).toHaveBeenCalledOnce();
});

it("does not turn controls, service help, captions or resize edges into drag targets", async () => {
  await mount();
  for (const selector of [
    `button[aria-label="${I18N.overlay.openSettings}"]`,
    ".overlay-service__button",
    '[data-testid="subtitle-content"]',
    '[data-testid="resize-edge"]',
  ]) {
    const target = host.querySelector(selector)!;
    expect(target, selector).not.toBeNull();
    await press(target);
    await press(target, 2);
  }
  expect(native.move).not.toHaveBeenCalled();
  expect(collapse).not.toHaveBeenCalled();
});

it("shares the in-flight control guard with the visible handle", async () => {
  let resolve!: () => void;
  collapse.mockImplementationOnce(() => new Promise<void>(finish => { resolve = finish; }));
  await mount();
  await press(area()!, 2);
  expect(area()!.disabled).toBe(true);
  expect(isClickablePointerTarget(area())).toBe(false);
  expect(host.querySelector<HTMLButtonElement>('[data-testid="drag-handle"]')!.disabled).toBe(true);
  await press(area()!);
  await press(area()!, 2);
  expect(native.move).not.toHaveBeenCalled();
  expect(collapse).toHaveBeenCalledOnce();
  await act(async () => resolve());
  await press(area()!);
  expect(native.move).toHaveBeenCalledOnce();
});

it.each(["locked", "immersive", "collapsed"] as const)("keeps the empty-top gesture unavailable in %s mode", async mode => {
  useStore.setState(state => ({
    settings: { ...state.settings, isOverlayLocked: mode === "locked", subtitleBlendsWithBackground: mode === "immersive" },
    session: { ...state.session, isOverlayCollapsed: mode === "collapsed" },
  }));
  await mount();
  if (mode === "locked") {
    expect(area()!.disabled).toBe(true);
    expect(isClickablePointerTarget(area())).toBe(false);
    await press(area()!);
  } else {
    expect(area()).toBeNull();
    if (mode === "collapsed") expect(host.querySelector('[data-testid="drag-handle"]')).not.toBeNull();
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
  expect(native.move).toHaveBeenCalledOnce();
  expect(collapse).not.toHaveBeenCalled();
  expect(useStore.getState().settings).toMatchObject({ isOverlayLocked: true, subtitleBlendsWithBackground: true });
});
