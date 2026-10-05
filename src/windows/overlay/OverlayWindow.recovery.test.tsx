// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import type { OverlayPointerMotion } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import type { SessionStateEvent, SettingsSnapshot } from "../../lib/types";
import { OverlayWindow } from "./OverlayWindow";

const ipc = vi.hoisted(() => ({
  sessionStart: vi.fn<() => Promise<void>>(),
  sessionTogglePaused: vi.fn<() => Promise<void>>(),
  settingsSave: vi.fn(),
  listenPointer: vi.fn(),
}));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(),
  isTauri: true,
  sessionStart: ipc.sessionStart,
  sessionTogglePaused: ipc.sessionTogglePaused,
  settingsSave: ipc.settingsSave,
  listenOverlayPointerMotion: ipc.listenPointer,
  listenOverlayControlMode: async () => () => {},
  overlayControlGetState: async () => "hidden",
}));
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => null }));
vi.mock("./Timeline", () => ({ Timeline: () => null }));

// Node URL avoids Vite rewriting a stylesheet URL to its browser asset path.
const initial = useStore.getState();
const failure = "The Tencent Cloud realtime translation connection stopped responding.";
let root: Root, host: HTMLDivElement;
let pointer: (point: OverlayPointerMotion) => void;
let previousHitTest: PropertyDescriptor | undefined;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("innerWidth", 640);
  vi.stubGlobal("innerHeight", 280);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  ipc.sessionStart.mockReset();
  ipc.sessionTogglePaused.mockReset();
  ipc.settingsSave.mockReset();
  ipc.listenPointer.mockReset().mockImplementation(handler => {
    pointer = handler;
    return Promise.resolve(() => {});
  });
  previousHitTest = Object.getOwnPropertyDescriptor(document, "elementFromPoint");
  // Model motion over the subtitle canvas; no native cursor IPC is needed.
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: () => host });
  setStoredUiLanguage("en");
  useStore.setState({ ...initial,
    settings: { ...initial.settings, isOverlayLocked: false, subtitleBlendsWithBackground: false,
      subtitleBackgroundOpacity: 75, pulseAnimation: false, subtitleAnimation: false },
    session: { ...initial.session, status: { kind: "error", message: failure },
      isActive: false, isPaused: false, isOverlayCollapsed: false },
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
  if (previousHitTest) Object.defineProperty(document, "elementFromPoint", previousHitTest);
  else Reflect.deleteProperty(document, "elementFromPoint");
  vi.unstubAllGlobals();
});

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

function button(label: string) {
  const element = host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  expect(element).not.toBeNull();
  return element!;
}

async function publishSession(change: Partial<SessionStateEvent>) {
  await act(async () => useStore.setState(state => ({ session: { ...state.session, ...change } })));
}

function expectCard(background: string) {
  expect(host.querySelector('[data-presentation="background-blend"]')).toBeNull();
  const card = host.querySelector<HTMLElement>(".overlay-swap-expanded > div");
  expect(card).not.toBeNull();
  expect(card!.style.background).toBe(background);
  expect(card!.style.border).not.toBe("");
  expect(host.querySelector(".overlay-service")).not.toBeNull();
  return card!;
}

function expectPreferencesUnchanged(settings: SettingsSnapshot) {
  expect(useStore.getState().settings).toBe(settings);
  expect(ipc.settingsSave).not.toHaveBeenCalled();
}

it.each([75, 0])("keeps the saved %s%% background through a real retry, connecting and listening", async opacity => {
  const start = deferred();
  ipc.sessionStart.mockReturnValueOnce(start.promise);
  useStore.setState(state => ({ settings: { ...state.settings, subtitleBackgroundOpacity: opacity } }));
  const settings = useStore.getState().settings;
  const background = `rgba(0, 0, 0, ${opacity / 100})`;
  await act(async () => root.render(<OverlayWindow />));
  expectCard(background);
  await act(async () => { button(I18N.overlay.retry).click(); });
  expect(ipc.sessionStart).toHaveBeenCalledOnce();
  expect(button(I18N.overlay.connecting).disabled).toBe(true);
  expectCard(background);
  expectPreferencesUnchanged(settings);

  await publishSession({ status: { kind: "connecting" }, isActive: true });
  expectCard(background);
  await act(async () => pointer({ x: 20, y: 20 }));
  expect(expectCard(background).style.borderWidth).toBe("1px");
  await act(async () => window.dispatchEvent(new Event("blur")));
  expect(expectCard(background).style.borderWidth).toBe("0.75px");
  await act(async () => pointer({ x: 21, y: 20 }));
  expect(expectCard(background).style.borderWidth).toBe("1px");
  expectPreferencesUnchanged(settings);

  await publishSession({ status: { kind: "listening" } });
  // The session event can precede the IPC acknowledgement.
  expect(button(I18N.overlay.connecting).disabled).toBe(true);
  expectCard(background);
  await act(async () => start.resolve());
  expect(button(I18N.overlay.pause).disabled).toBe(false);
  expectCard(background);
  expectPreferencesUnchanged(settings);
});

it("preserves the saved background after a failed retry without persisting preferences", async () => {
  const start = deferred();
  ipc.sessionStart.mockReturnValueOnce(start.promise);
  const settings = useStore.getState().settings;
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => { button(I18N.overlay.retry).click(); });
  await publishSession({ status: { kind: "connecting" }, isActive: true });
  expectCard("rgba(0, 0, 0, 0.75)");
  await publishSession({ status: { kind: "error", message: failure }, isActive: false });
  await act(async () => start.reject(new Error(failure)));
  expect(button(I18N.overlay.retry).disabled).toBe(false);
  expect(ipc.sessionStart).toHaveBeenCalledOnce();
  expectCard("rgba(0, 0, 0, 0.75)");
  expectPreferencesUnchanged(settings);
});

it.each(["success", "failure"] as const)("preserves the non-immersive background when paused resume ends in %s", async outcome => {
  const resume = deferred();
  ipc.sessionTogglePaused.mockReturnValueOnce(resume.promise);
  await publishSession({ status: { kind: "listening" }, isActive: true, isPaused: true });
  const settings = useStore.getState().settings;
  await act(async () => root.render(<OverlayWindow />));
  expectCard("rgba(0, 0, 0, 0.75)");
  await act(async () => { button(I18N.overlay.resume).click(); });
  expect(ipc.sessionTogglePaused).toHaveBeenCalledOnce();
  expect(ipc.sessionStart).not.toHaveBeenCalled();
  expect(button(I18N.overlay.connecting).disabled).toBe(true);
  expectCard("rgba(0, 0, 0, 0.75)");
  if (outcome === "success") {
    await publishSession({ status: { kind: "connecting" } });
    expectCard("rgba(0, 0, 0, 0.75)");
    await publishSession({ status: { kind: "listening" }, isPaused: false });
    await act(async () => resume.resolve());
    expect(button(I18N.overlay.pause).disabled).toBe(false);
  } else {
    await act(async () => resume.reject(new Error(failure)));
    expect(button(I18N.overlay.resume).disabled).toBe(false);
    expect(useStore.getState().session.isPaused).toBe(true);
    expect(host.querySelector('[role="alert"]')).not.toBeNull();
  }
  expectCard("rgba(0, 0, 0, 0.75)");
  expectPreferencesUnchanged(settings);
});

it("respects saved immersive mode as soon as retry leaves the temporary error presentation", async () => {
  const start = deferred();
  ipc.sessionStart.mockReturnValueOnce(start.promise);
  useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: true } }));
  const settings = useStore.getState().settings;
  await act(async () => root.render(<OverlayWindow />));
  expectCard("rgba(16, 16, 16, 0.96)");
  await act(async () => { button(I18N.overlay.retry).click(); });
  expect(ipc.sessionStart).toHaveBeenCalledOnce();
  expectCard("rgba(16, 16, 16, 0.96)");
  await publishSession({ status: { kind: "connecting" }, isActive: true });
  expect(host.querySelector('[data-presentation="background-blend"]')).not.toBeNull();
  expect(host.querySelector(".overlay-service")).toBeNull();
  expectPreferencesUnchanged(settings);
  await publishSession({ status: { kind: "listening" } });
  await act(async () => start.resolve());
  expect(host.querySelector('[data-presentation="background-blend"]')).not.toBeNull();
  expectPreferencesUnchanged(settings);
});
