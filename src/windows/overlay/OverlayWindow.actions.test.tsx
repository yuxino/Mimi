// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { OverlayWindow } from "./OverlayWindow";
import { audioInputLabel } from "../../lib/audioInput";

const nativeFeedback = vi.hoisted(() => ({ callback: undefined as ((event: import("../../lib/ipc").ProfileSwitchFeedback) => void) | undefined }));
vi.mock("../../lib/ipc", async (original) => ({
  ...await original<typeof import("../../lib/ipc")>(),
  isTauri: true,
  listenOverlayPointerMotion: async () => () => {},
  listenProfileSwitchFeedback: async (callback: (event: import("../../lib/ipc").ProfileSwitchFeedback) => void) => { nativeFeedback.callback = callback; return () => { nativeFeedback.callback = undefined; }; },
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

it("shows the independent translation mark and opens settings with guarded failure feedback", async () => {
  const showSettings = vi.fn().mockRejectedValueOnce(new Error("synthetic-private-error")).mockResolvedValue(undefined);
  useStore.setState(state => ({ showSettings, settings: { ...state.settings,
    profiles: [{ id: "test", name: "Translation profile", provider: "alibabaCloud", textTranslation: "deepL", credentialState: "present" }],
    activeProfileId: "test",
  } }));
  await act(async () => root.render(<OverlayWindow />));
  const service = host.querySelector<HTMLButtonElement>(".overlay-service__button")!;
  expect(service.textContent).toBe("Alibaba CloudDeepL");
  expect(service.querySelector('[data-stage="recognition"] [data-provider="alibabaCloud"]')).not.toBeNull();
  expect(service.querySelector('[data-provider="deepL"]')).not.toBeNull();
  expect(service.getAttribute("aria-label")).toContain("Translation profile");
  await act(async () => service.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  await act(async () => service.click());
  expect(showSettings).toHaveBeenCalledTimes(2);
  expect(showSettings).toHaveBeenCalledWith("activeProfile");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("keeps the service visible through errors and restores immersive or collapsed chrome after recovery", async () => {
  await act(async () => root.render(<OverlayWindow />));
  for (const change of [{ isPaused: true }, { isActive: false, status: { kind: "error" as const, message: "unavailable" } }]) {
    await act(async () => useStore.setState(state => ({ session: { ...state.session, ...change } })));
    expect(host.querySelector(".overlay-service")).not.toBeNull();
  }
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: true } })));
  expect(host.querySelector(".overlay-service")).not.toBeNull();
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: true } })));
  expect(host.querySelector(".overlay-service")).not.toBeNull();
  await act(async () => useStore.setState(state => ({ session: { ...state.session, status: { kind: "listening" }, isActive: true } })));
  expect(host.querySelector(".overlay-service")).toBeNull();
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: false }, session: { ...state.session, isOverlayCollapsed: true } })));
  expect(host.querySelector(".overlay-service")).toBeNull();
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
  ["showSettings", () => I18N.overlay.openSettings],
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

it("retains direct close at the minimum width without overlapping the capsule", async () => {
  vi.stubGlobal("innerWidth", 360);
  await act(async () => root.render(<OverlayWindow />));
  expect(button(I18N.overlay.closeSubtitles)).not.toBeNull();
  expect(button(I18N.overlay.pause)).toBeNull();
  expect(host.querySelectorAll(".overlay-control-button")).toHaveLength(1);
});


it.each(["normal", "immersive", "collapsed"])("shows rejected profile selection on the %s canvas after the popover closes", async mode => {
  useStore.setState(state => ({ settings: { ...state.settings, activeProfileId: "test", subtitleBlendsWithBackground: mode === "immersive" },
    session: { ...state.session, isOverlayCollapsed: mode === "collapsed" } }));
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => nativeFeedback.callback?.({ requestId: 1, pending: true, error: null }));
  expect(host.querySelector('.overlay-action-feedback')?.getAttribute('role')).toBe('status');
  await act(async () => nativeFeedback.callback?.({ requestId: 1, pending: false, error: "apple_speech_status_failed" }));
  expect(host.querySelector('.overlay-action-feedback[role="alert"]')?.textContent).toBe(I18N.settings.appleSpeechLoadFailed);
  expect(useStore.getState().settings.activeProfileId).toBe("test");
  expect(useStore.getState().session.status.kind).toBe("listening");
  await act(async () => nativeFeedback.callback?.({ requestId: 2, pending: true, error: null }));
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await act(async () => nativeFeedback.callback?.({ requestId: 2, pending: false, error: null }));
  expect(host.querySelector('.overlay-action-feedback')).toBeNull();
});
