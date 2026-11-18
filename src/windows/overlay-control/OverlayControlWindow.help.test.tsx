// @vitest-environment jsdom
import { invoke } from "@tauri-apps/api/core";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import { applicationAudioCopy } from "../../lib/audio/applicationAudio";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { TARGET_LANGUAGE_DISPLAY_NAMES, type SettingsSnapshot, type TargetLanguage } from "../../lib/types";
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

it("honors the saved divider preference across native immersive changes", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  disposeStoreSnapshotStreams();
  const initial = useStore.getState();
  let savedSettings: SettingsSnapshot = { ...initial.settings, uiLanguage: "en",
    showSubtitleDividers: false, subtitleBlendsWithBackground: false };
  const session = { ...initial.session, status: { kind: "listening" as const }, isActive: true };
  const save = vi.fn();
  vi.mocked(invoke).mockImplementation(async command => {
    if (command === "settings_get") return savedSettings;
    if (command === "session_get_state") return session;
    if (command === "settings_save") return save();
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
    expect(subtitleMarkup()).not.toContain("subtitle-separator");
    // A preference broadcast from Settings still reaches the timeline.
    await act(async () => {
      savedSettings = { ...savedSettings, showSubtitleDividers: true };
      native.onSettings!(savedSettings);
    });
    expect(useStore.getState().settings.showSubtitleDividers).toBe(true);
    expect(subtitleMarkup()).toContain("subtitle-separator");
    for (const immersive of [true, false]) {
      await act(async () => {
        savedSettings = { ...savedSettings, subtitleBlendsWithBackground: immersive };
        native.onSettings!(savedSettings);
      });
      expect(useStore.getState().settings.showSubtitleDividers).toBe(true);
      expect(subtitleMarkup().includes("subtitle-separator")).toBe(!immersive);
    }
    expect(save).not.toHaveBeenCalled();
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

it("retains Original across panel reopen and sends the manually chosen translation target", async () => {
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
    const target = () => host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.settings.translateTo}"]`)!;
    const choose = async (language: TargetLanguage) => {
      await act(async () => target().click());
      const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')]
        .find(choice => choice.textContent === TARGET_LANGUAGE_DISPLAY_NAMES[language])!;
      await act(async () => option.click());
    };
    await choose("original");
    expect(switchTargetLanguage).toHaveBeenCalledExactlyOnceWith("original");
    expect(target().textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.original);
    await act(async () => native.onMode!("island"));
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    await act(async () => native.onMode!("panel"));
    expect(target().textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.original);
    await choose("ja");
    expect(switchTargetLanguage).toHaveBeenLastCalledWith("ja");
    expect(target().textContent).toBe(TARGET_LANGUAGE_DISPLAY_NAMES.ja);
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


it("keeps the floating panel open when Escape belongs to composition or a child control", async () => {
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
    const escape = (options: KeyboardEventInit = {}) =>
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true, ...options });
    for (const composition of [{ isComposing: true }, { keyCode: 229 }]) {
      await act(() => { input.dispatchEvent(escape(composition)); });
      expect(document.querySelector('.mimi-select__search')).toBe(input);
      expect(host.querySelector('[role="dialog"]')).not.toBeNull();
      expect(native.hide).not.toHaveBeenCalled();
    }
    await act(() => { input.dispatchEvent(escape()); });
    expect(document.querySelector('.mimi-select__search')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    expect(native.hide).not.toHaveBeenCalled();
    trigger.addEventListener("keydown", (event) => event.preventDefault(), { once: true });
    await act(() => { trigger.dispatchEvent(escape()); });
    expect(native.hide).not.toHaveBeenCalled();
    await act(() => { trigger.dispatchEvent(escape()); });
    expect(native.hide).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount()); host.remove();
    useStore.setState(initial, true); native.hide.mockClear(); vi.mocked(invoke).mockImplementation(async () => null); vi.unstubAllGlobals();
  }
});
