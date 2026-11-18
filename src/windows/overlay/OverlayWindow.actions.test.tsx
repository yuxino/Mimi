// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { OverlayWindow } from "./OverlayWindow";
import { audioInputLabel } from "../../lib/audio/audioInput";
import { recordingStatusLabel } from "../../lib/audio/recordingStatus";

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
    settings: { ...initial.settings, activeProfileId: "configured", profiles: [{ id: "configured", provider: "alibabaCloud", name: "Configured service", credentialState: "present" }], isOverlayLocked: false, subtitleBlendsWithBackground: false, pulseAnimation: false, subtitleAnimation: false },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove(); useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});
function button(label: string) { return host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!; }

it("keeps recording state accessible when collapsed without adding another action", async () => {
  vi.stubGlobal("innerWidth", 280);
  vi.stubGlobal("innerHeight", 54);
  useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: true } }));
  await act(async () => root.render(<OverlayWindow />));
  const actionCount = host.querySelectorAll("button").length;
  expect(button(I18N.overlay.clearSubtitles)).toBeNull();
  expect(button(I18N.overlay.enterImmersiveMode)).toBeNull();
  expect(button(I18N.overlay.lockPosition)).toBeNull();
  for (const audioRecording of ["starting", "recording", "paused", "limited", "error", "off"] as const) {
    await act(async () => useStore.setState(state => ({ session: { ...state.session, audioRecording } })));
    expect(host.querySelectorAll("button")).toHaveLength(actionCount);
    const indicator = host.querySelector<HTMLElement>("[data-recording-status]");
    const label = recordingStatusLabel(audioRecording, "en");
    if (!label) expect(indicator).toBeNull();
    else {
      expect(indicator?.title).toBe(label);
      expect(indicator?.textContent).toBe("");
      expect(host.querySelector('[role="group"]')?.getAttribute("aria-label")).toContain(label);
    }
  }
});

it("keeps the error surface actionable and restores saved presentation after recovery", async () => {
  useStore.setState(state => ({ settings: {
    ...state.settings, isOverlayLocked: true, subtitleBlendsWithBackground: true,
  } }));
  await act(async () => root.render(<OverlayWindow />));
  expect(host.querySelector('[data-presentation="background-blend"]')).not.toBeNull();
  expect(host.querySelector(".overlay-action-strip")).toBeNull();

  await act(async () => useStore.setState(state => ({ session: {
    ...state.session, isActive: false, status: { kind: "error", message: "unavailable" },
  } })));
  expect(host.querySelector('[data-presentation="background-blend"]')).toBeNull();
  expect(button(I18N.overlay.closeSubtitles)).not.toBeNull();

  await act(async () => useStore.setState(state => ({ session: {
    ...state.session, isActive: true, status: { kind: "listening" },
  } })));
  expect(host.querySelector('[data-presentation="background-blend"]')).not.toBeNull();
  expect(host.querySelector(".overlay-action-strip")).toBeNull();
});

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
  ["stop", () => I18N.overlay.closeSubtitles],
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
  expect(button(I18N.overlay.pause).disabled).toBe(true);
  expect(button(I18N.overlay.clearSubtitles).disabled).toBe(true);
  expect(button(I18N.overlay.enterImmersiveMode).disabled).toBe(true);
  expect(button(I18N.overlay.lockPosition).disabled).toBe(true);
  expect(host.querySelectorAll('[aria-busy="true"]')).toHaveLength(1);
  await act(async () => reject(new Error("synthetic private detail must not be shown")));
  expect(selected.disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(host.textContent).not.toContain("synthetic private detail");
  await act(async () => selected.click());
  expect(operation).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("enables Clear for a retained subtitle and disables it again after clearing", async () => {
  const empty = initial.session.subtitles;
  const clearSubtitles = vi.fn(async () => useStore.setState(state => ({ session: { ...state.session, subtitles: empty } })));
  useStore.setState(state => ({ clearSubtitles, session: { ...state.session, subtitles: empty } }));
  await act(async () => root.render(<OverlayWindow />));
  const clear = button(I18N.overlay.clearSubtitles);
  expect(clear.disabled).toBe(true);
  await act(async () => clear.click());
  expect(clearSubtitles).not.toHaveBeenCalled();

  const history = [{ source: "Retained synthetic caption.", translation: "", createdAt: 1 }];
  await act(async () => useStore.setState(state => ({ session: { ...state.session, subtitles: { ...empty, history } } })));
  expect(clear.disabled).toBe(false);
  await act(async () => clear.click());
  expect(clearSubtitles).toHaveBeenCalledOnce();
  expect(clear.disabled).toBe(true);
  expect(useStore.getState().session).toMatchObject({ isActive: true, subtitles: empty });
});

it("blocks Clear, Immersive and Lock while a session action is pending", async () => {
  let settle!: () => void;
  const togglePaused = vi.fn(() => new Promise<void>(resolve => { settle = resolve; }));
  const clearSubtitles = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ togglePaused, clearSubtitles });
  await act(async () => root.render(<OverlayWindow />));
  const clear = button(I18N.overlay.clearSubtitles);
  await act(async () => button(I18N.overlay.pause).click());
  expect(clear.disabled).toBe(true);
  expect(button(I18N.overlay.enterImmersiveMode).disabled).toBe(true);
  expect(button(I18N.overlay.lockPosition).disabled).toBe(true);
  await act(async () => clear.click());
  expect(clearSubtitles).not.toHaveBeenCalled();
  await act(async () => settle());
  expect(clear.disabled).toBe(false);
});

it("enters immersive mode through the existing setting and hides the toolbar", async () => {
  const saveSettings = vi.fn<typeof initial.saveSettings>(async (draft) => {
    useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: draft.subtitleBlendsWithBackground ?? false } }));
  });
  useStore.setState({ saveSettings });
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => button(I18N.overlay.enterImmersiveMode).click());
  expect(saveSettings).toHaveBeenCalledExactlyOnceWith({ subtitleBlendsWithBackground: true });
  expect(host.querySelector('[data-presentation="background-blend"]')).not.toBeNull();
  expect(host.querySelector(".overlay-action-strip")).toBeNull();
});

it("locks through the existing action and gives up subtitle toolbar targets", async () => {
  const setOverlayLocked = vi.fn<typeof initial.setOverlayLocked>(async (locked) => {
    useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: locked } }));
  });
  useStore.setState({ setOverlayLocked });
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => button(I18N.overlay.lockPosition).click());
  expect(setOverlayLocked).toHaveBeenCalledExactlyOnceWith(true);
  expect(host.querySelector(".overlay-expanded-canvas")?.hasAttribute("data-overlay-locked")).toBe(true);
  expect(host.querySelector(".overlay-action-strip")).toBeNull();
  expect(host.querySelector<HTMLButtonElement>('[data-testid="overlay-drag-area"]')?.disabled).toBe(true);
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

it("does not turn a repeated pause click into settings navigation after a configuration error", async () => {
  const showSettings = vi.fn().mockResolvedValue(undefined);
  const togglePaused = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ showSettings, togglePaused });
  await act(async () => root.render(<OverlayWindow />));
  const pause = button(I18N.overlay.pause);
  await act(async () => pause.click());

  await act(async () => useStore.setState(state => ({ session: { ...state.session,
    isActive: false, status: { kind: "error", message: "credential_authentication_failed" },
  } })));
  await act(async () => pause.click());
  expect(showSettings).not.toHaveBeenCalled();
  expect(togglePaused).toHaveBeenCalledOnce();
  expect(pause.isConnected).toBe(false);

  const configure = host.querySelector<HTMLButtonElement>(".session-error-feedback button")!;
  await act(async () => configure.click());
  expect(showSettings).toHaveBeenCalledExactlyOnceWith("service");
});

it.each(["listening", "paused", "connecting", "error", "collapsed"])("can close the %s surface through the existing stop action", async mode => {
  const stop = vi.fn().mockResolvedValue(undefined);
  useStore.setState(state => ({ stop, session: { ...state.session,
    isPaused: mode === "paused", isOverlayCollapsed: mode === "collapsed",
    isActive: mode !== "error",
    status: mode === "error" ? { kind: "error", message: "unavailable" }
      : mode === "connecting" ? { kind: "connecting" } : { kind: "listening" },
  } }));
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => button(I18N.overlay.closeSubtitles).click());
  expect(stop).toHaveBeenCalledOnce();
});

it("keeps a collapsed close failure visible and disables close while stopping", async () => {
  const stop = vi.fn().mockRejectedValueOnce(new Error("private stop failure")).mockResolvedValue(undefined);
  useStore.setState(state => ({ stop, session: { ...state.session, isOverlayCollapsed: true } }));
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => button(I18N.overlay.closeSubtitles).click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(host.textContent).not.toContain("private stop failure");
  await act(async () => button(I18N.overlay.closeSubtitles).click());
  expect(stop).toHaveBeenCalledTimes(2);
  await act(async () => useStore.setState(state => ({ session: { ...state.session, status: { kind: "stopping" } } })));
  expect(button(I18N.overlay.closeSubtitles).disabled).toBe(true);
});

it("retains all six direct actions at the minimum width", async () => {
  vi.stubGlobal("innerWidth", 360);
  await act(async () => root.render(<OverlayWindow />));
  expect(Array.from(host.querySelectorAll(".overlay-control-button"), button => button.getAttribute("aria-label"))).toEqual([
    I18N.overlay.pause, I18N.overlay.collapseSubtitle, I18N.overlay.clearSubtitles,
    I18N.overlay.enterImmersiveMode, I18N.overlay.lockPosition, I18N.overlay.closeSubtitles,
  ]);
  expect(Array.from(host.querySelectorAll(".overlay-action-group"), group =>
    Array.from(group.querySelectorAll("button"), button => button.getAttribute("aria-label")))).toEqual([
    [I18N.overlay.pause, I18N.overlay.collapseSubtitle, I18N.overlay.clearSubtitles],
    [I18N.overlay.enterImmersiveMode, I18N.overlay.lockPosition],
  ]);
  expect(host.querySelector(".overlay-action-strip > span:last-child button")).toBe(button(I18N.overlay.closeSubtitles));
});
