// @vitest-environment jsdom
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { applicationAudioCopy } from "../../lib/applicationAudio";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import type { SettingsSnapshot, TargetLanguage } from "../../lib/types";
import type { OverlayControlMode } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import { OverlayControlWindow } from "./OverlayControlWindow";

const native = vi.hoisted(() => ({
  onMode: undefined as ((mode: OverlayControlMode) => void) | undefined,
  hide: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(), isTauri: true,
  overlayControlGetState: async () => "panel",
  overlayControlSetIslandWidth: async () => {}, overlayControlSetPanelHeight: async () => {},
  overlayPopoverHide: native.hide,
  listenOverlayControlMode: async (callback: (mode: OverlayControlMode) => void) => {
    native.onMode = callback; return () => { native.onMode = undefined; };
  },
}));

it("closes focused source help before Escape dismisses the actual control panel", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  // Complete the panel's opening autofocus before the user tabs to help.
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  const initial = useStore.getState();
  useStore.setState({ settings: { ...initial.settings, audioInput: "both" } });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<OverlayControlWindow />));
    expect(host.querySelector('[role="dialog"]')).not.toBeNull();
    const help = host.querySelector<HTMLButtonElement>('.overlay-control-capture .settings-help-control__button')!;
    await act(async () => help.focus());
    expect(document.querySelector('[role="tooltip"]')).not.toBeNull();
    const escape = () => help.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await act(async () => { escape(); });
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    expect(document.activeElement).toBe(help);
    expect(native.hide).not.toHaveBeenCalled();
    await act(async () => { escape(); });
    expect(native.hide).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount()); host.remove();
    useStore.setState(initial, true); native.hide.mockClear(); vi.unstubAllGlobals();
  }
});

it("restores the chosen translation target after dismissing and reopening the floating panel", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  const initial = useStore.getState();
  const switchTargetLanguage = vi.fn(async (targetLanguage: TargetLanguage) => {
    useStore.setState({ settings: { ...useStore.getState().settings, targetLanguage } });
  });
  useStore.setState({ settings: { ...initial.settings, sourceLanguage: "en", targetLanguage: "ja", languageCapabilities: undefined,
    activeProfileId: "ali", profiles: [{ id: "ali", name: "Alibaba", provider: "alibabaCloud", credentialState: "present" }] },
    session: { ...initial.session, status: { kind: "listening" }, isActive: true }, switchTargetLanguage });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<OverlayControlWindow />));
    const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.skipTranslation}"]`)!;
    await act(async () => toggle().click());
    expect(switchTargetLanguage).toHaveBeenCalledExactlyOnceWith("original");
    expect(toggle().getAttribute("aria-checked")).toBe("true");
    await act(async () => native.onMode!("island"));
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => native.onMode!("panel"));
    await act(async () => toggle().click());
    expect(switchTargetLanguage).toHaveBeenLastCalledWith("ja");
    expect(toggle().getAttribute("aria-checked")).toBe("false");
  } finally {
    await act(async () => root.unmount()); host.remove();
    useStore.setState(initial, true); native.hide.mockClear(); vi.unstubAllGlobals();
  }
});


it.each(["zh", "en", "ja"] as const)("shares the early subtitle preference and contextual help with other windows in %s", async language => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  setStoredUiLanguage(language);
  const initial = useStore.getState();
  const saveSettings = vi.fn(async (draft: Partial<SettingsSnapshot>) => {
    useStore.setState({ settings: { ...useStore.getState().settings, ...draft } });
  });
  const session = { ...initial.session, status: { kind: "listening" as const }, isActive: true };
  useStore.setState({ settings: { ...initial.settings, showIntermediateSubtitles: true }, session, saveSettings });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<OverlayControlWindow />));
    const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.showIntermediateSubtitles}"]`)!;
    const help = toggle().closest('.overlay-control-setting-row')!.querySelector<HTMLButtonElement>('.settings-help-control__button')!;
    expect(help.closest('button[role="switch"]')).toBeNull();
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    expect(document.getElementById(toggle().getAttribute("aria-describedby")!)?.textContent).toBe(I18N.settings.showIntermediateSubtitlesHelp);
    await act(async () => help.focus());
    expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.settings.showIntermediateSubtitlesHelp);
    expect(saveSettings).not.toHaveBeenCalled();
    await act(async () => help.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    expect(native.hide).not.toHaveBeenCalled();
    await act(async () => toggle().click());
    expect(saveSettings).toHaveBeenCalledExactlyOnceWith({ showIntermediateSubtitles: false });
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(useStore.getState().session).toBe(session);
    expect(native.hide).not.toHaveBeenCalled();
    // A settings/tray snapshot immediately updates this control; it has no
    // separate local toggle state that could disagree with another window.
    await act(async () => useStore.setState({ settings: { ...useStore.getState().settings, showIntermediateSubtitles: true } }));
    expect(toggle().getAttribute("aria-checked")).toBe("true");
    expect(saveSettings).toHaveBeenCalledTimes(1);
  } finally {
    await act(async () => root.unmount()); host.remove();
    useStore.setState(initial, true); setStoredUiLanguage("system"); native.hide.mockClear(); vi.unstubAllGlobals();
  }
});

it.each(["system", "microphone", "both"] as const)("offers shared time display and sanitized save retry for %s input", async audioInput => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  const initial = useStore.getState();
  const saveSettings = vi.fn(async (draft: Partial<SettingsSnapshot>) => {
    useStore.setState({ settings: { ...useStore.getState().settings, ...draft } });
  }).mockRejectedValueOnce(new Error("synthetic-private-save-failure"));
  const session = { ...initial.session, status: { kind: "listening" as const }, isActive: true };
  useStore.setState({ settings: { ...initial.settings, audioInput, showSubtitleTimestamps: false }, session, saveSettings });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<OverlayControlWindow />));
    const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.subtitleTimestamps}"]`)!;
    const help = toggle().closest('.overlay-control-setting-row')!.querySelector<HTMLButtonElement>('.settings-help-control__button')!;
    expect(toggle().disabled).toBe(false);
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(help.closest('button[role="switch"]')).toBeNull();
    expect(document.getElementById(toggle().getAttribute("aria-describedby")!)?.textContent).toBe(I18N.settings.subtitleTimestampsHelp);
    await act(async () => help.focus());
    expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.settings.subtitleTimestampsHelp);
    expect(saveSettings).not.toHaveBeenCalled();
    await act(async () => toggle().click());
    expect(saveSettings).toHaveBeenCalledExactlyOnceWith({ showSubtitleTimestamps: true });
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(host.querySelector('.overlay-control-alert[role="alert"]')?.textContent).toBe(I18N.settings.settingSaveFailed(I18N.settings.subtitleTimestamps));
    expect(host.textContent).not.toContain("synthetic-private-save-failure");
    await act(async () => toggle().click());
    expect(toggle().getAttribute("aria-checked")).toBe("true");
    expect(host.querySelector('.overlay-control-alert')).toBeNull();
    expect(native.hide).not.toHaveBeenCalled();
    expect(useStore.getState().session).toBe(session);
    await act(async () => native.onMode!("island"));
    await act(async () => native.onMode!("panel"));
    expect(toggle().getAttribute("aria-checked")).toBe("true");
    await act(async () => useStore.setState({ settings: { ...useStore.getState().settings, showSubtitleTimestamps: false } }));
    expect(toggle().getAttribute("aria-checked")).toBe("false");
  } finally {
    await act(async () => root.unmount()); host.remove();
    useStore.setState(initial, true); native.hide.mockClear(); vi.unstubAllGlobals();
  }
});

it("pauses and resumes the actual immersive session from its floating panel", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  const initial = useStore.getState();
  const togglePaused = vi.fn(async () => {
    const session = useStore.getState().session;
    useStore.setState({ session: { ...session, isPaused: !session.isPaused } });
  });
  useStore.setState({ settings: { ...initial.settings, audioInput: "microphone", subtitleBlendsWithBackground: true },
    session: { ...initial.session, status: { kind: "listening" }, isActive: true, isPaused: false }, togglePaused });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<OverlayControlWindow />));
    const action = () => host.querySelector<HTMLButtonElement>(".overlay-control-session-action")!;
    expect(action().getAttribute("aria-label")).toBe(I18N.overlay.pause);
    await act(async () => action().click());
    expect(useStore.getState().session.isPaused).toBe(true);
    expect(action().getAttribute("aria-label")).toBe(I18N.overlay.resume);
    await act(async () => action().click());
    expect(useStore.getState().session.isPaused).toBe(false);
    expect(togglePaused).toHaveBeenCalledTimes(2);
    expect(useStore.getState().settings.subtitleBlendsWithBackground).toBe(true);
    expect(native.hide).not.toHaveBeenCalled();
    for (const kind of ["connecting", "stopping"] as const) {
      await act(async () => useStore.setState({ session: { ...useStore.getState().session, status: { kind } } }));
      expect(action().disabled).toBe(true);
    }
  } finally {
    await act(async () => root.unmount()); host.remove();
    useStore.setState(initial, true); native.hide.mockClear(); vi.unstubAllGlobals();
  }
});


it("lets IME Escape cancel composition without dismissing application search or the floating panel", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("navigator", { userAgent: "Macintosh" });
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  vi.mocked(invoke).mockImplementation(async command => command === "audio_applications" ? { supported: true, applications: [] } : null);
  const initial = useStore.getState();
  useStore.setState({ initializationStatus: "ready", settings: { ...initial.settings, systemAudioTarget: { kind: "system" } } });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<OverlayControlWindow />));
    const trigger = host.querySelector<HTMLButtonElement>(`button[aria-label="${applicationAudioCopy().title}"]`)!;
    await act(async () => trigger.click());
    const input = document.querySelector<HTMLInputElement>('.mimi-select__search')!;
    expect(document.activeElement).toBe(input);
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", isComposing: true, bubbles: true, cancelable: true })));
    expect(document.querySelector('.mimi-select__search')).toBe(input);
    expect(host.querySelector('[role="dialog"]')).not.toBeNull();
    expect(native.hide).not.toHaveBeenCalled();
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(document.querySelector('.mimi-select__search')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    expect(native.hide).not.toHaveBeenCalled();
    await act(async () => trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(native.hide).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount()); host.remove();
    useStore.setState(initial, true); native.hide.mockClear(); vi.mocked(invoke).mockImplementation(async () => null); vi.unstubAllGlobals();
  }
});
