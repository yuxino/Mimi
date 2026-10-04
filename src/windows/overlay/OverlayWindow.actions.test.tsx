// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { OverlayWindow } from "./OverlayWindow";
import { audioInputLabel } from "../../lib/audioInput";

vi.mock("../../lib/ipc", async (original) => ({
  ...await original<typeof import("../../lib/ipc")>(),
  isTauri: true,
  listenOverlayPointerMotion: async () => () => {},
}));
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => null }));
vi.mock("./Timeline", () => ({ Timeline: () => null }));

const initial = useStore.getState();
let root: Root, host: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("innerWidth", 640);
  vi.stubGlobal("innerHeight", 136);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  setStoredUiLanguage("en");
  useStore.setState({ ...initial,
    session: { ...initial.session, isActive: true, status: { kind: "listening" },
      subtitles: { ...initial.session.subtitles, source: { text: "Synthetic clearable draft.", isFinal: false } } },
    settings: { ...initial.settings, isOverlayLocked: false, subtitleBlendsWithBackground: false, pulseAnimation: false, subtitleAnimation: false },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove(); useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});
function button(label: string) { return host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!; }

it.each(["system", "microphone", "both"] as const)("keeps %s sources identifiable in the collapsed and paused surface", async audioInput => {
  vi.stubGlobal("innerHeight", 54);
  vi.stubGlobal("innerWidth", 280);
  useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: true, isPaused: true },
    settings: { ...state.settings, audioInput },
  }));
  await act(async () => root.render(<OverlayWindow />));
  expect(host.querySelector('[role="group"]')?.getAttribute("aria-label")).toContain(audioInputLabel(audioInput));
  if (audioInput === "system") expect(host.querySelector('[role="img"]')).toBeNull();
  else expect(host.querySelector('[role="img"]')?.getAttribute("aria-label")).toBe(audioInputLabel(audioInput));
  expect(button(I18N.overlay.resume)).not.toBeNull();
  expect(button(I18N.overlay.expandSubtitle)).not.toBeNull();
});

it.each([
  ["setOverlayCollapsed", () => I18N.overlay.collapseSubtitle],
  ["clearSubtitles", () => I18N.overlay.clearSubtitles],
  ["saveSettings", () => I18N.overlay.enterImmersiveMode],
  ["setOverlayLocked", () => I18N.overlay.lockPosition],
  ["showSettings", () => I18N.overlay.openSettings],
] as const)("guards %s until settled, shows a safe failure and permits a manual retry", async (action, label) => {
  let reject!: (error: Error) => void;
  const operation = vi.fn().mockImplementationOnce(() => new Promise<void>((_resolve, fail) => { reject = fail; })).mockResolvedValue(undefined);
  useStore.setState({ [action]: operation });
  await act(async () => root.render(<OverlayWindow />));
  const selected = button(label());
  await act(async () => { selected.click(); selected.click(); });
  expect(operation).toHaveBeenCalledOnce();
  expect(selected.getAttribute("aria-busy")).toBe("true");
  expect(selected.disabled).toBe(true);
  expect(button(I18N.overlay.openSettings).disabled).toBe(true);
  // Both collapse affordances share the in-flight action; the keyboard drag
  // handle must not remain active while the icon button is busy (or vice versa).
  expect(host.querySelectorAll('[aria-busy="true"]')).toHaveLength(action === "setOverlayCollapsed" ? 2 : 1);
  await act(async () => reject(new Error("synthetic private detail must not be shown")));
  expect(selected.disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(host.textContent).not.toContain("synthetic private detail");
  await act(async () => selected.click());
  expect(operation).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("shows expansion failures within the 54px collapsed surface and keeps its retry action", async () => {
  vi.stubGlobal("innerHeight", 54);
  useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: true },
    setOverlayCollapsed: vi.fn().mockRejectedValueOnce(new Error("synthetic expansion failure")).mockResolvedValue(undefined),
  }));
  await act(async () => root.render(<OverlayWindow />));
  const expand = button(I18N.overlay.expandSubtitle);
  await act(async () => expand.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(button(I18N.overlay.expandSubtitle)).toBe(expand);
  await act(async () => expand.click());
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("keeps a failed pause action visible and retryable in the collapsed surface", async () => {
  vi.stubGlobal("innerHeight", 54);
  const togglePaused = vi.fn().mockRejectedValueOnce(new Error("synthetic pause failure")).mockResolvedValue(undefined);
  useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: true }, togglePaused }));
  await act(async () => root.render(<OverlayWindow />));
  const pause = button(I18N.overlay.pause);
  await act(async () => pause.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(pause.disabled).toBe(false);
  await act(async () => pause.click());
  expect(togglePaused).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});
