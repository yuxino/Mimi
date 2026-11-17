// @vitest-environment jsdom
import { act, Profiler } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { OverlayPointerMotion } from "../../lib/ipc";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { OverlayWindow } from "./OverlayWindow";

const listenPointer = vi.hoisted(() => vi.fn());
vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  isTauri: true,
  hasEmbeddedOverlayControls: true,
  listenOverlayPointerMotion: listenPointer,
}));
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => null }));

const original = useStore.getState();
let host: HTMLDivElement, root: Root;
let pointer: (point: OverlayPointerMotion) => void;
let previousHitTest: PropertyDescriptor | undefined;
const unlisten = vi.fn();

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  unlisten.mockClear();
  listenPointer.mockReset().mockImplementation((handler) => {
    pointer = handler;
    return Promise.resolve(unlisten);
  });
  previousHitTest = Object.getOwnPropertyDescriptor(document, "elementFromPoint");
  setStoredUiLanguage("en");
  useStore.setState({ ...original,
    session: { ...original.session, isActive: true, status: { kind: "listening" } },
    settings: { ...original.settings, isOverlayLocked: false, subtitleBlendsWithBackground: false, pulseAnimation: false, subtitleAnimation: false },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove(); useStore.setState(original, true); setStoredUiLanguage("system");
  if (previousHitTest) Object.defineProperty(document, "elementFromPoint", previousHitTest);
  else Reflect.deleteProperty(document, "elementFromPoint");
  vi.unstubAllGlobals(); vi.restoreAllMocks();
});

it("bridges inactive native hover without clicks, repeated window renders or an orphan listener", async () => {
  const commits = vi.fn();
  await act(async () => root.render(<Profiler id="overlay" onRender={commits}><OverlayWindow /></Profiler>));
  expect(listenPointer).toHaveBeenCalledOnce();
  const surface = host.querySelector(".overlay-surface")!;
  expect(surface.hasAttribute("data-overlay-hover")).toBe(false);
  const button = host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.pause}"]`)!;
  const hitTest = vi.fn(() => surface.hasAttribute("data-overlay-hover") ? button.querySelector("svg") : null);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: hitTest });
  const beforeEntering = commits.mock.calls.length;
  hitTest.mockReturnValueOnce(null);
  await act(async () => pointer({ x: 10, y: 10 }));
  expect(surface.hasAttribute("data-overlay-hover")).toBe(true);
  expect(commits).toHaveBeenCalledTimes(beforeEntering);
  await act(async () => pointer(null));
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.overlay.pause);
  expect(button.dataset.hovered).toBe("true");
  expect(document.activeElement).not.toBe(button);
  const afterEntering = commits.mock.calls.length;
  for (let i = 0; i < 30; i++) await act(async () => pointer({ x: 20 + i, y: 20 }));
  expect(commits).toHaveBeenCalledTimes(afterEntering);
  await act(async () => window.dispatchEvent(new Event("blur")));
  expect(surface.hasAttribute("data-overlay-hover")).toBe(true);
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
  await act(async () => pointer(null));
  expect(surface.hasAttribute("data-overlay-hover")).toBe(false);
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  expect(button.dataset.hovered).toBeUndefined();
  await act(async () => root.unmount());
  expect(unlisten).toHaveBeenCalledOnce();
  const previousCalls = hitTest.mock.calls.length;
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(hitTest).toHaveBeenCalledTimes(previousCalls);
});

it("keeps error recovery available when the native pointer is outside", async () => {
  useStore.setState(state => ({ session: { ...state.session, isActive: false,
    status: { kind: "error", message: "unavailable" },
  } }));
  await act(async () => root.render(<OverlayWindow />));
  const surface = host.querySelector(".overlay-surface")!;
  expect(surface.hasAttribute("data-overlay-hover")).toBe(false);
  expect(surface.hasAttribute("data-overlay-chrome-pinned")).toBe(true);
  expect(host.querySelector(".overlay-session-error-content")).not.toBeNull();
  await act(async () => useStore.setState(state => ({ session: { ...state.session,
    isActive: true, status: { kind: "listening" },
  } })));
  expect(surface.hasAttribute("data-overlay-chrome-pinned")).toBe(false);
});

it("releases a native listener that finishes registering after the overlay unmounts", async () => {
  let finish: (unlisten: () => void) => void;
  listenPointer.mockImplementation(() => new Promise<() => void>((resolve) => { finish = resolve; }));
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => root.unmount());
  await act(async () => finish(unlisten));
  expect(unlisten).toHaveBeenCalledOnce();
});

it("keeps the same toolbar buttons and hover target when another surface clears captions", async () => {
  const subtitles = {
    source: { text: "Synthetic recognition to clear.", isFinal: false },
    translation: { text: "", isFinal: false }, history: [], previewPair: null,
  };
  useStore.setState(state => ({
    clearSubtitles: async () => useStore.setState(current => ({ session: {
      ...current.session, subtitles: { ...subtitles, source: { text: "", isFinal: false } },
    } })),
    settings: { ...state.settings, sourceLanguage: "auto", targetLanguage: "zh", subtitleDisplayMode: "translation" },
    session: { ...state.session, subtitles, detectedLanguage: "en" },
  }));
  await act(async () => root.render(<OverlayWindow />));
  const buttons = Array.from(host.querySelectorAll<HTMLButtonElement>(".overlay-control-button"));
  expect(buttons.map(button => button.getAttribute("aria-label"))).toEqual([
    I18N.overlay.pause, I18N.overlay.collapseSubtitle, I18N.overlay.clearSubtitles,
    I18N.overlay.enterImmersiveMode, I18N.overlay.lockPosition, I18N.overlay.closeSubtitles,
  ]);
  const pause = buttons[0];
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => pause.querySelector("svg") });
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.overlay.pause);

  await act(async () => useStore.getState().clearSubtitles());
  const emptyButtons = Array.from(host.querySelectorAll<HTMLButtonElement>(".overlay-control-button"));
  expect(emptyButtons).toHaveLength(buttons.length);
  emptyButtons.forEach((button, index) => expect(button).toBe(buttons[index]));
  expect(emptyButtons[2].disabled).toBe(true);
  expect(pause.disabled).toBe(false);
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.overlay.pause);
});
