// @vitest-environment jsdom
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { applicationAudioCopy } from "../../lib/applicationAudio";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import type { SettingsSnapshot, TargetLanguage } from "../../lib/types";
import type { OverlayControlMode } from "../../lib/ipc";
import { disposeStoreSnapshotStreams, useStore } from "../../lib/store";
import { Timeline } from "../overlay/Timeline";
import { OverlayControlWindow } from "./OverlayControlWindow";

const native = vi.hoisted(() => ({
  onMode: undefined as ((mode: OverlayControlMode) => void) | undefined,
  onSettings: undefined as ((settings: SettingsSnapshot) => void) | undefined,
  hide: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => null) }));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(), isTauri: true,
  overlayControlGetState: async () => "panel",
  overlayControlSetIslandWidth: async () => {}, overlayControlSetPanelHeight: async () => {},
  overlayPopoverHide: native.hide,
  listenSettingsChanged: async (callback: (settings: SettingsSnapshot) => void) => {
    native.onSettings = callback; return () => { native.onSettings = undefined; };
  },
  listenSessionState: async () => () => {},
  listenOverlayControlMode: async (callback: (mode: OverlayControlMode) => void) => {
    native.onMode = callback; return () => { native.onMode = undefined; };
  },
}));

it.each(["zh", "en", "ja"] as const)("shares sentence dividers through native settings saves and broadcasts in %s", async language => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  disposeStoreSnapshotStreams();
  const initial = useStore.getState();
  let savedSettings: SettingsSnapshot = { ...initial.settings, uiLanguage: language,
    showSubtitleDividers: false, showSubtitleTimestamps: true, subtitleBlendsWithBackground: false };
  const session = { ...initial.session, status: { kind: "listening" as const }, isActive: true };
  let rejectSave!: (reason: Error) => void;
  const save = vi.fn(async (draft: Partial<SettingsSnapshot>) => {
    savedSettings = { ...savedSettings, ...draft };
    native.onSettings!(savedSettings);
    return savedSettings;
  }).mockImplementationOnce(() => new Promise<SettingsSnapshot>((_resolve, reject) => { rejectSave = reject; }));
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "settings_get") return savedSettings;
    if (command === "session_get_state") return session;
    if (command === "settings_save") return save((args as { draft: Partial<SettingsSnapshot> }).draft);
    return null;
  });
  useStore.setState({ ...initial, initialized: false }, true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const subtitleMarkup = () => {
    const settings = useStore.getState().settings;
    return renderToStaticMarkup(<Timeline
      blocks={[
        { id: "first", createdAt: 1, presentation: "history", source: "First fixture", translation: null },
        { id: "second", createdAt: 2, presentation: "history", source: "Second fixture", translation: null },
      ]}
      fontSize={18} alignment="center" color="white" displayMode="original"
      showSubtitleDividers={settings.showSubtitleDividers}
      blendsWithBackground={settings.subtitleBlendsWithBackground}
    />);
  };
  try {
    await act(async () => { await useStore.getState().init(); root.render(<OverlayControlWindow />); });
    expect(useStore.getState().initializationStatus).toBe("ready");
    const toggle = () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.settings.subtitleDividers}"]`)!;
    const expectEnabled = (enabled: boolean) => {
      expect(toggle().getAttribute("aria-checked")).toBe(String(enabled));
      expect(toggle().classList.contains("is-on")).toBe(enabled);
      expect(useStore.getState().settings.showSubtitleDividers).toBe(enabled);
    };
    const help = toggle().closest('.overlay-control-setting-row')!.querySelector<HTMLButtonElement>('.settings-help-control__button')!;
    expectEnabled(false);
    expect(subtitleMarkup()).not.toContain("subtitle-separator");
    expect([...host.querySelectorAll('.overlay-control-setting-row strong')].map(label => label.textContent)).toEqual([
      I18N.settings.showIntermediateSubtitles, I18N.settings.subtitleDividers, I18N.settings.subtitleTimestamps,
    ]);
    expect(help.closest('button[role="switch"]')).toBeNull();
    expect(document.getElementById(toggle().getAttribute("aria-describedby")!)?.textContent).toBe(I18N.settings.subtitleDividersHelp);
    await act(async () => help.focus());
    expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(I18N.settings.subtitleDividersHelp);
    expect(save).not.toHaveBeenCalled();
    await act(async () => help.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(document.querySelector('[role="tooltip"]')).toBeNull();
    expect(native.hide).not.toHaveBeenCalled();

    await act(async () => { toggle().click(); toggle().click(); });
    expect(save).toHaveBeenCalledExactlyOnceWith({ showSubtitleDividers: true });
    expect(toggle().disabled).toBe(true);
    // Shared preference saves preview immediately, then roll back on rejection.
    expectEnabled(true);
    await act(async () => rejectSave(new Error("synthetic-private-divider-save-error")));
    expectEnabled(false);
    expect(toggle().disabled).toBe(false);
    expect(host.querySelector('.overlay-control-alert[role="alert"]')?.textContent).toBe(I18N.settings.settingSaveFailed(I18N.settings.subtitleDividers));
    expect(host.textContent).not.toContain("synthetic-private-divider-save-error");
    await act(async () => toggle().click());
    expectEnabled(true);
    expect(save).toHaveBeenCalledTimes(2);
    expect(host.querySelector('.overlay-control-alert')).toBeNull();
    expect(native.hide).not.toHaveBeenCalled();
    expect(useStore.getState().session).toBe(session);
    expect(useStore.getState().settings.showSubtitleTimestamps).toBe(true);
    expect(subtitleMarkup()).toContain("subtitle-separator");

    // The same settings-changed stream carries changes made in the settings window.
    for (const enabled of [false, true]) {
      await act(async () => {
        savedSettings = { ...savedSettings, showSubtitleDividers: enabled };
        native.onSettings!(savedSettings);
      });
      expectEnabled(enabled);
      expect(subtitleMarkup().includes("subtitle-separator")).toBe(enabled);
    }
    expect(save).toHaveBeenCalledTimes(2);
    await act(async () => native.onMode!("island"));
    await act(async () => native.onMode!("panel"));
    expectEnabled(true);

    for (const immersive of [true, false]) {
      await act(async () => host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${I18N.overlay.immersiveMode}"]`)!.click());
      expect(save).toHaveBeenLastCalledWith({ subtitleBlendsWithBackground: immersive });
      expectEnabled(true);
      expect(subtitleMarkup().includes("subtitle-separator")).toBe(!immersive);
    }
    expect(useStore.getState().session).toBe(session);
  } finally {
    await act(async () => root.unmount()); host.remove(); disposeStoreSnapshotStreams();
    useStore.setState(initial, true); setStoredUiLanguage("system"); native.hide.mockClear();
    vi.mocked(invoke).mockImplementation(async () => null); vi.unstubAllGlobals();
  }
});

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

it("switches saved profiles through the shared store from the paused floating panel", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  const initial = useStore.getState();
  const session = { ...initial.session, status: { kind: "listening" as const }, isActive: true, isPaused: true };
  const settings: SettingsSnapshot = { ...initial.settings, activeProfileId: "first", sourceLanguage: "en", targetLanguage: "zh", languageCapabilities: undefined,
    profiles: [
      { id: "first", name: "First model", provider: "alibabaCloud", credentialState: "present" },
      { id: "second", name: "Second model", provider: "openAIRealtime", credentialState: "present" },
    ] };
  const selectProfile = vi.fn(async (activeProfileId: string) => {
    const snapshot = { ...settings, activeProfileId };
    useStore.setState({ settings: snapshot });
    return snapshot;
  });
  useStore.setState({ settings, session, selectProfile });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<OverlayControlWindow />));
    const picker = () => host.querySelector<HTMLButtonElement>(`.overlay-control-picker--profile [role="combobox"]`)!;
    expect(picker().textContent).toBe("First model");
    expect(picker().disabled).toBe(false);
    await act(async () => picker().click());
    const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === "Second model")!;
    await act(async () => option.click());
    expect(selectProfile).toHaveBeenCalledExactlyOnceWith("second");
    expect(picker().textContent).toBe("Second model");
    expect(useStore.getState().session).toBe(session);
    expect(native.hide).not.toHaveBeenCalled();
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
