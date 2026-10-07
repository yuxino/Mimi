// @vitest-environment jsdom
import { act, Profiler } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { OverlayPointerMotion } from "../../lib/ipc";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { isClickablePointerTarget } from "../../lib/overlayPointer";
import { useStore } from "../../lib/store";
import { OverlayWindow } from "./OverlayWindow";

const listenPointer = vi.hoisted(() => vi.fn());
vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  isTauri: true,
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
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
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
  const button = host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.openSettings}"]`)!;
  const hitTest = vi.fn(() => button.querySelector("svg"));
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: hitTest });
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.overlay.openSettings);
  expect(button.dataset.hovered).toBe("true");
  expect(document.activeElement).not.toBe(button);
  const afterEntering = commits.mock.calls.length;
  for (let i = 0; i < 30; i++) await act(async () => pointer({ x: 20 + i, y: 20 }));
  expect(commits).toHaveBeenCalledTimes(afterEntering);
  await act(async () => window.dispatchEvent(new Event("blur")));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
  await act(async () => pointer(null));
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  expect(button.dataset.hovered).toBeUndefined();
  await act(async () => root.unmount());
  expect(unlisten).toHaveBeenCalledOnce();
  const previousCalls = hitTest.mock.calls.length;
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(hitTest).toHaveBeenCalledTimes(previousCalls);
});

it("releases a native listener that finishes registering after the overlay unmounts", async () => {
  let finish: (unlisten: () => void) => void;
  listenPointer.mockImplementation(() => new Promise<() => void>((resolve) => { finish = resolve; }));
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => root.unmount());
  await act(async () => finish(unlisten));
  expect(unlisten).toHaveBeenCalledOnce();
});

it("keeps every toolbar action in place after clearing and rejects clicks on the empty clear slot", async () => {
  const subtitles = {
    source: { text: "Synthetic recognition to clear.", isFinal: false },
    translation: { text: "", isFinal: false }, history: [], previewPair: null,
  };
  const clearSubtitles = vi.fn(async () => {
    useStore.setState(state => ({ session: { ...state.session, subtitles: {
      source: { text: "", isFinal: false }, translation: { text: "", isFinal: false },
      history: [], previewPair: null,
    } } }));
  });
  useStore.setState(state => ({
    clearSubtitles,
    settings: { ...state.settings, sourceLanguage: "auto", targetLanguage: "zh", subtitleDisplayMode: "translation" },
    session: { ...state.session, subtitles, detectedLanguage: "en" },
  }));
  await act(async () => root.render(<OverlayWindow />));
  const buttons = Array.from(host.querySelectorAll<HTMLButtonElement>(".overlay-control-button"));
  expect(buttons.map(button => button.getAttribute("aria-label"))).toEqual([
    I18N.overlay.pause, I18N.overlay.collapseSubtitle, I18N.overlay.clearSubtitles,
    I18N.overlay.enterImmersiveMode, I18N.overlay.lockPosition, I18N.overlay.openSettings, I18N.overlay.closeSubtitles,
  ]);
  const clear = buttons[2];
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => clear.querySelector("svg") });
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.overlay.clearSubtitles);
  expect(isClickablePointerTarget(clear)).toBe(true);

  await act(async () => clear.click());
  const emptyButtons = Array.from(host.querySelectorAll<HTMLButtonElement>(".overlay-control-button"));
  expect(emptyButtons).toHaveLength(buttons.length);
  emptyButtons.forEach((button, index) => expect(button).toBe(buttons[index]));
  expect(clear.disabled).toBe(true);
  expect(isClickablePointerTarget(clear)).toBe(false);
  await act(async () => { clear.click(); pointer({ x: 21, y: 20 }); });
  expect(clearSubtitles).toHaveBeenCalledOnce();
  expect(document.querySelector('[role="tooltip"]')).toBeNull();
  await act(async () => pointer(null));
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.overlay.clearSubtitles);

  await act(async () => useStore.setState(state => ({ session: { ...state.session, subtitles } })));
  expect(host.querySelector(`button[aria-label="${I18N.overlay.clearSubtitles}"]`)).toBe(clear);
  expect(clear.disabled).toBe(false);
});
