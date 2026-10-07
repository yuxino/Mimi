// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { audio3ErrorMessage, audio3ErrorSummary } from "../lib/audio3Errors";
import type { OverlayControlMode } from "../lib/ipc";
import { localizedSessionErrorSummary } from "../lib/sessionErrorPresentation";
import { diagnosticCopy } from "../lib/connectionDiagnostics";
import { I18N, setStoredUiLanguage } from "../lib/i18n";
import { useStore } from "../lib/store";
import { OverlayWindow } from "./overlay/OverlayWindow";
import { OverlayControlWindow } from "./overlay-control/OverlayControlWindow";
import { TrayPanel } from "./tray-panel/TrayPanel";
import { SubtitleSessionControls } from "./settings/SubtitleSessionControls";

const native = vi.hoisted(() => ({
  enabled: false,
  mode: "island" as OverlayControlMode,
  listeners: new Set<(mode: OverlayControlMode) => void>(),
}));
vi.mock("../lib/ipc", async original => ({
  ...await original<typeof import("../lib/ipc")>(),
  get isTauri() { return native.enabled; },
  overlayControlGetState: async () => native.mode,
  overlayControlSetIslandWidth: async () => {},
  overlayControlSetPanelHeight: async () => {},
  listenOverlayPointerMotion: async () => () => {},
  listenOverlayControlMode: async (handler: (mode: OverlayControlMode) => void) => {
    native.listeners.add(handler);
    return () => { native.listeners.delete(handler); };
  },
}));
vi.mock("@tauri-apps/api/window", async original => ({
  ...await original<typeof import("@tauri-apps/api/window")>(),
  getCurrentWindow: () => ({ setSize: async () => {} }),
}));
vi.mock("./overlay/PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./overlay/ResizeHandles", () => ({ ResizeHandles: () => null }));
vi.mock("./overlay-control/CaptureStatusRow", () => ({ CaptureStatusRow: () => null }));
vi.mock("../lib/useDesktopShortcuts", () => ({ useDesktopShortcuts: () => ({ nativeShortcuts: true, commands: null }) }));
vi.mock("./overlay/Timeline", () => ({ Timeline: ({ blocks }: { blocks: { source: string | null }[] }) =>
  <div data-testid="retained-subtitles">{blocks.map(block => block.source).join(" ")}</div> }));

const initial = useStore.getState();
const unsupportedLanguage = "audio3_error.setup.unsupported_language.UNSUPPORTED_LANGUAGE";
const timeout = "audio3_error.recognition.timeout.LOCAL_TIMEOUT";
const surfaces = ["overlay", "history", "immersive", "collapsed", "locked", "control", "tray", "settings", "compact-settings"] as const;
type Surface = typeof surfaces[number];
const isOverlaySurface = (surface: Surface) => ["overlay", "history", "immersive", "collapsed", "locked"].includes(surface);
let host: HTMLDivElement, root: Root;
let start: ReturnType<typeof vi.fn>, showSettings: ReturnType<typeof vi.fn>, setOverlayCollapsed: ReturnType<typeof vi.fn>;

beforeEach(() => {
  native.enabled = false; native.mode = "island"; native.listeners.clear();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  window.history.replaceState(null, "", "?mode=panel");
  start = vi.fn().mockResolvedValue(undefined);
  showSettings = vi.fn().mockResolvedValue(undefined);
  setOverlayCollapsed = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, start, showSettings, setOverlayCollapsed,
    settings: { ...initial.settings, pulseAnimation: false, subtitleAnimation: false, subtitleDisplayMode: "original",
      activeProfileId: "custom", targetLanguage: "original", sourceLanguage: "ja",
      profiles: [{ id: "custom", provider: "customDashScopeASR", name: "Speech fixture", credentialState: "present" }] },
    session: { ...initial.session, isActive: false, isPaused: false, status: { kind: "error", message: unsupportedLanguage } },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initial, true); setStoredUiLanguage("system");
  window.history.replaceState(null, "", window.location.pathname);
  vi.unstubAllGlobals();
});

async function mount(surface: Surface) {
  if (["history", "immersive", "collapsed", "locked"].includes(surface)) {
    useStore.setState(state => ({
      session: { ...state.session, isOverlayCollapsed: surface === "collapsed",
        subtitles: { ...state.session.subtitles, source: { text: "Retained synthetic subtitle", isFinal: true } } },
      settings: { ...state.settings, subtitleBlendsWithBackground: surface === "immersive", isOverlayLocked: surface === "locked" },
    }));
  }
  await act(async () => root.render(
    surface === "control" ? <OverlayControlWindow /> : surface === "tray" ? <TrayPanel />
      : surface === "settings" || surface === "compact-settings"
        ? <SubtitleSessionControls compact={surface === "compact-settings"} onConfigure={target => void showSettings(target)} />
        : <OverlayWindow />,
  ));
}

const feedback = () => host.querySelector<HTMLElement>(".session-error-feedback")!;
const configure = () => feedback().querySelector<HTMLButtonElement>("button")!;

it.each(["zh", "en", "ja"] as const)("keeps localized unsupported-language causes and configuration actions visible on every surface in %s", async language => {
  setStoredUiLanguage(language);
  for (const surface of surfaces) {
    await act(async () => root.render(null));
    await mount(surface);
    expect(feedback().getAttribute("role")).toBe("alert");
    expect(feedback().querySelector("p")?.textContent).toBe(isOverlaySurface(surface) ? audio3ErrorSummary(unsupportedLanguage) : audio3ErrorMessage(unsupportedLanguage));
    expect(feedback().closest('[aria-hidden="true"], [role="tooltip"]')).toBeNull();
    expect(host.textContent).not.toContain(unsupportedLanguage);
    expect(configure().textContent).toBe(I18N.settings.openSpeechSettings);
    expect(feedback().querySelectorAll("button")).toHaveLength(1);
    expect([...host.querySelectorAll("button")].some(button => button.textContent === I18N.settings.sessionRetry)).toBe(false);
    if (["history", "immersive", "collapsed", "locked"].includes(surface)) {
      expect(host.querySelector('[data-testid="retained-subtitles"]')?.textContent).toContain("Retained synthetic subtitle");
      expect(host.querySelector(".overlay-swap-collapsed, [data-presentation='background-blend']")).toBeNull();
      expect(host.querySelector<HTMLButtonElement>('[data-testid="drag-handle"]')?.disabled).toBe(false);
    }
    await act(async () => configure().focus());
    expect(document.activeElement).toBe(configure());
    await act(async () => configure().click());
    expect(showSettings).toHaveBeenLastCalledWith("service");
    expect(start).not.toHaveBeenCalled();
    expect(setOverlayCollapsed).not.toHaveBeenCalled();
  }
});

it.each(surfaces)("clears only the error presentation when %s recovers", async surface => {
  setStoredUiLanguage("en");
  await mount(surface);
  const settings = useStore.getState().settings;
  const subtitles = useStore.getState().session.subtitles;
  await act(async () => useStore.setState(state => ({ session: { ...state.session, status: { kind: "idle" } } })));
  expect(host.querySelector(".session-error-feedback")).toBeNull();
  expect(useStore.getState().settings).toBe(settings);
  expect(useStore.getState().session.subtitles).toBe(subtitles);
  if (surface === "collapsed") expect(host.querySelector(".overlay-swap-collapsed")).not.toBeNull();
  if (surface === "immersive") expect(host.querySelector('[data-presentation="background-blend"]')).not.toBeNull();
});

it.each(["overlay", "control", "tray", "settings", "compact-settings"] as const)("retains retry for a transient failure in %s", async surface => {
  setStoredUiLanguage("en");
  useStore.setState(state => ({ session: { ...state.session, status: { kind: "error", message: timeout } } }));
  await mount(surface);
  const retry = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.sessionRetry)!;
  expect(retry).toBeDefined();
  expect(feedback().querySelector("p")?.textContent).toBe(isOverlaySurface(surface) ? audio3ErrorSummary(timeout) : audio3ErrorMessage(timeout));
  await act(async () => retry.click());
  expect(start).toHaveBeenCalledOnce();
  expect(showSettings).not.toHaveBeenCalled();
});

it.each(["overlay", "control", "tray"] as const)("guards configuration actions and sanitizes their failures in %s", async surface => {
  let reject!: (error: Error) => void;
  showSettings.mockImplementationOnce(() => new Promise<void>((_resolve, fail) => { reject = fail; }));
  await mount(surface);
  await act(async () => { configure().click(); configure().click(); });
  expect(showSettings).toHaveBeenCalledOnce();
  expect(configure().disabled).toBe(true);
  await act(async () => reject(new Error("private rejected settings action")));
  expect(configure().disabled).toBe(false);
  expect(host.textContent).not.toContain("private rejected settings action");
  expect(feedback().querySelector("p")?.textContent).toBe(isOverlaySurface(surface) ? audio3ErrorSummary(unsupportedLanguage) : audio3ErrorMessage(unsupportedLanguage));
  await act(async () => configure().click());
  expect(showSettings).toHaveBeenCalledTimes(2);
});

it("routes the settings start switch to configuration while a configuration error remains", async () => {
  await mount("settings");
  const toggle = host.querySelector<HTMLButtonElement>('[role="switch"]')!;
  await act(async () => toggle.click());
  expect(showSettings).toHaveBeenCalledExactlyOnceWith("service");
  expect(start).not.toHaveBeenCalled();
});

it.each(["overlay", "control", "tray", "settings", "compact-settings"] as const)("never exposes an unrecognized provider message in %s", async surface => {
  useStore.setState(state => ({ session: { ...state.session, status: { kind: "error", message: "provider-private-text synthetic-key" } } }));
  await mount(surface);
  expect(feedback().querySelector("p")?.textContent).toBe(I18N.settings.sessionError);
  expect(host.textContent).not.toMatch(/provider-private-text|synthetic-key/);
});

it.each(["zh", "en", "ja"] as const)("shows an Audio3 startup transport cause and usable retry across surfaces in %s", async language => {
  setStoredUiLanguage(language);
  const error = "The speech recognition transport failed.";
  useStore.setState(state => ({ session: { ...state.session, status: { kind: "error", message: error } } }));
  for (const surface of surfaces) {
    await act(async () => root.render(null));
    start.mockClear();
    await mount(surface);
    expect(feedback().querySelector("p")?.textContent).toBe(isOverlaySurface(surface) ? localizedSessionErrorSummary(diagnosticCopy().speechUnreachable) : diagnosticCopy().speechUnreachable);
    expect(feedback().closest('[aria-hidden="true"], [role="tooltip"]')).toBeNull();
    expect(host.textContent).not.toContain(error);
    expect(feedback().textContent).not.toContain(I18N.settings.sessionError);
    expect(configure().textContent).toBe(isOverlaySurface(surface) ? I18N.settings.sessionRetry : I18N.settings.openSpeechSettings);
    const retry = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.sessionRetry)!;
    expect(retry).toBeDefined();
    await act(async () => retry.click());
    expect(start).toHaveBeenCalledOnce();
    expect(showSettings).not.toHaveBeenCalled();
  }
});

// Error presentation is interactive even when the saved reading mode is locked.
it.each(["overlay", "history", "immersive", "collapsed", "locked"] as const)("keeps movement and top actions visible through %s error presentation", async surface => {
  await mount(surface);
  const drag = host.querySelector<HTMLButtonElement>('[data-testid="drag-handle"]')!;
  expect(drag.disabled).toBe(false);
  expect(drag.getAttribute("aria-label")).toBe(I18N.overlay.moveSubtitle);
  const topActions = [...host.querySelectorAll<HTMLButtonElement>(".overlay-control-button")];
  expect(topActions).toHaveLength(7);
  const settingsAction = topActions.find(button => button.getAttribute("aria-label") === I18N.overlay.openSettings)!;
  expect(settingsAction.disabled).toBe(false);
  for (const id of ["collapse-subtitles", "toggle-immersive-mode", "toggle-overlay-lock"]) {
    expect(host.querySelector<HTMLButtonElement>(`[data-testid="${id}"]`)?.disabled).toBe(true);
  }
  await act(async () => settingsAction.click());
  expect(showSettings).toHaveBeenCalledOnce();
  expect(setOverlayCollapsed).not.toHaveBeenCalled();
  const settings = useStore.getState().settings;
  await act(async () => useStore.setState(state => ({ session: { ...state.session, isActive: true, status: { kind: "listening" } } })));
  expect(useStore.getState().settings).toBe(settings);
  expect(host.querySelector(".session-error-feedback")).toBeNull();
  if (surface === "locked" || surface === "immersive") {
    expect(host.querySelector(".overlay-control-button")).toBeNull();
  } else if (surface === "collapsed") {
    expect(host.querySelector<HTMLButtonElement>('[data-testid="expand-subtitles"]')?.disabled).toBe(false);
  } else {
    expect(host.querySelector<HTMLButtonElement>('[data-testid="collapse-subtitles"]')?.disabled).toBe(false);
  }
});

it.each(["zh", "en", "ja"] as const)("assigns detailed recovery to the open panel while keeping the %s tray independent", async language => {
  setStoredUiLanguage(language);
  native.enabled = true;
  useStore.setState(state => ({ session: { ...state.session, status: { kind: "error", message: timeout } } }));
  await mount("overlay");
  const summary = audio3ErrorSummary(timeout);
  const fullMessage = audio3ErrorMessage(timeout);
  expect(feedback().querySelector("p")?.textContent).toBe(summary);
  expect(feedback().querySelectorAll("button")).toHaveLength(1);
  expect(feedback().querySelector("button")?.textContent).toBe(I18N.settings.sessionRetry);
  expect(feedback().textContent).not.toContain(fullMessage);

  native.mode = "panel";
  await act(async () => { for (const listener of native.listeners) listener(native.mode); });
  expect(feedback().querySelector("p")?.textContent).toBe(summary);
  expect(feedback().querySelector("button")).toBeNull();
  expect(host.querySelectorAll(".overlay-control-button")).toHaveLength(7);
  expect(host.querySelector<HTMLButtonElement>('[data-testid="drag-handle"]')?.disabled).toBe(false);

  await mount("control");
  expect(feedback().querySelector("p")?.textContent).toBe(fullMessage);
  expect(feedback().querySelectorAll("button")).toHaveLength(2);
  await mount("tray");
  expect(feedback().querySelector("p")?.textContent).toBe(fullMessage);
  expect(feedback().querySelectorAll("button")).toHaveLength(2);

  await mount("overlay");
  expect(feedback().querySelector("button")).toBeNull();
  native.mode = "island";
  await act(async () => { for (const listener of native.listeners) listener(native.mode); });
  const retry = feedback().querySelector<HTMLButtonElement>("button")!;
  expect(retry.textContent).toBe(I18N.settings.sessionRetry);
  await act(async () => retry.click());
  expect(start).toHaveBeenCalledOnce();
  expect(showSettings).not.toHaveBeenCalled();
});


it.each(["zh", "en", "ja"] as const)("opens Apple resources instead of retrying missing resources across %s surfaces", async language => {
  setStoredUiLanguage(language);
  useStore.setState(state => ({
    settings: { ...state.settings, activeProfileId: "apple", profiles: [{ id: "apple", provider: "appleSpeech", name: "Apple Speech", credentialState: "missing" }] },
    session: { ...state.session, status: { kind: "error", message: "apple_speech_assets_missing" } },
  }));
  for (const surface of surfaces) {
    await act(async () => root.render(null));
    showSettings.mockClear();
    await mount(surface);
    expect(feedback().querySelector("p")?.textContent).toBe(isOverlaySurface(surface) ? localizedSessionErrorSummary(I18N.settings.appleSpeechAssetsMissing) : I18N.settings.appleSpeechAssetsMissing);
    expect(configure().textContent).toBe(I18N.settings.appleSpeechOpenResources);
    expect(feedback().querySelectorAll("button")).toHaveLength(1);
    expect([...host.querySelectorAll("button")].some(button => button.textContent === I18N.settings.sessionRetry)).toBe(false);
    expect(host.textContent).not.toContain("apple_speech_assets_missing");
    await act(async () => configure().click());
    expect(showSettings).toHaveBeenCalledExactlyOnceWith("appleSpeechResources");
    expect(start).not.toHaveBeenCalled();
  }
});

it.each(["apple_speech_preparing", "apple_speech_prepare_failed"])("opens the resource state for %s without starting subtitles", async message => {
  useStore.setState(state => ({ session: { ...state.session, status: { kind: "error", message } } }));
  await mount("settings");
  expect(configure().textContent).toBe(I18N.settings.appleSpeechOpenResources);
  await act(async () => host.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
  expect(showSettings).toHaveBeenCalledExactlyOnceWith("appleSpeechResources");
  expect(start).not.toHaveBeenCalled();
});

it.each(["control", "tray"] as const)("keeps Apple resources discoverable with zero or one ready language in %s", async surface => {
  const stop = vi.fn().mockResolvedValue(undefined);
  const switchSourceLanguage = vi.fn().mockResolvedValue(undefined);
  const saveSettings = vi.fn().mockResolvedValue(undefined);
  useStore.setState(state => ({ stop, switchSourceLanguage, saveSettings,
    settings: { ...state.settings, sourceLanguage: "en", targetLanguage: "original", activeProfileId: "apple",
      profiles: [{ id: "apple", provider: "appleSpeech", name: "Apple Speech", credentialState: "missing" }] },
    session: { ...state.session, status: { kind: "listening" }, isActive: true, isPaused: true },
  }));
  for (const sourceLanguages of [[], ["en"]] as const) {
    await act(async () => root.render(null));
    useStore.setState(state => ({ settings: { ...state.settings,
      languageCapabilities: { profileId: "apple", provider: "appleSpeech", textTranslation: "followService", targetLanguage: "original", sourceLanguages, targetLanguages: ["original"] },
    } }));
    showSettings.mockClear();
    await mount(surface);
    const button = host.querySelector<HTMLButtonElement>(".speech-resources-link")!;
    expect(button.textContent).toBe(I18N.settings.appleSpeechResources);
    expect(button.disabled).toBe(false);
    if (surface === "control") expect(host.querySelector<HTMLButtonElement>(`[role="combobox"][aria-label="${I18N.overlay.sourceLanguage}"]`)?.disabled).toBe(true);
    await act(async () => button.click());
    expect(showSettings).toHaveBeenCalledExactlyOnceWith("appleSpeechResources");
    expect(start).not.toHaveBeenCalled();
    expect(stop).not.toHaveBeenCalled();
    expect(switchSourceLanguage).not.toHaveBeenCalled();
    expect(saveSettings).not.toHaveBeenCalled();
  }
});

it.each(["control", "tray"] as const)("shows the compact resource entry only for Apple and handles a failed open in %s", async surface => {
  await mount(surface);
  expect(host.querySelector(".speech-resources-link")).toBeNull();
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    activeProfileId: "apple", profiles: [{ id: "apple", provider: "appleSpeech", name: "Apple Speech", credentialState: "missing" }],
  } })));
  let reject!: (error: Error) => void;
  showSettings.mockImplementationOnce(() => new Promise<void>((_resolve, fail) => { reject = fail; }));
  const button = host.querySelector<HTMLButtonElement>(".speech-resources-link")!;
  await act(async () => { button.click(); button.click(); });
  expect(showSettings).toHaveBeenCalledExactlyOnceWith("appleSpeechResources");
  expect(button.disabled).toBe(true);
  await act(async () => reject(new Error("private-resource-navigation-failure")));
  expect(button.disabled).toBe(false);
  expect(host.textContent).not.toContain("private-resource-navigation-failure");
  expect(host.querySelectorAll('[role="alert"]').length).toBeGreaterThan(1);
});


it.each(["en", "zh", "ja"] as const)("opens service settings for fixed authentication and configuration failures across %s surfaces", async language => {
  setStoredUiLanguage(language);
  const labels = diagnosticCopy();
  for (const [error, message] of [
    ["credential_authentication_failed", labels.auth],
    ["custom_speech_authentication_failed", labels.auth],
    ["custom_speech_credentials_missing", labels.missing],
    ["custom_speech_endpoint_invalid", I18N.settings.customSpeechEndpointInvalid],
    ["custom_speech_model_invalid", I18N.settings.customSpeechModelInvalid],
    ["text_translation_credentials_missing", labels.missing],
    ["Add a Google Gemini API key in Settings.", labels.missing],
    ["Enter a valid Azure OpenAI resource endpoint in Settings.", labels.reasons.invalidConfiguration],
    ["Baidu realtime translation requires an explicit supported source language.", I18N.settings.languageSwitchUnsupported],
  ]) {
    for (const surface of surfaces) {
      await act(async () => root.render(null));
      useStore.setState(state => ({ session: { ...state.session, status: { kind: "error", message: error } } }));
      showSettings.mockClear();
      await mount(surface);
      expect(feedback().querySelector("p")?.textContent).toBe(isOverlaySurface(surface) ? localizedSessionErrorSummary(message) : message);
      expect(configure().textContent).toBe(I18N.settings.openSpeechSettings);
      expect(feedback().querySelectorAll("button")).toHaveLength(1);
      expect([...host.querySelectorAll("button")].some(button => button.textContent === I18N.settings.sessionRetry)).toBe(false);
      await act(async () => configure().click());
      expect(showSettings).toHaveBeenCalledExactlyOnceWith("service");
      expect(start).not.toHaveBeenCalled();
      if (surface === "settings" || surface === "compact-settings") {
        showSettings.mockClear();
        await act(async () => host.querySelector<HTMLButtonElement>('[role="switch"]')!.click());
        expect(showSettings).toHaveBeenCalledExactlyOnceWith("service");
        expect(start).not.toHaveBeenCalled();
      }
    }
  }
});

it.each(["en", "zh", "ja"] as const)("retains retry for built-in timeouts and unclassified rejections across %s surfaces", async language => {
  setStoredUiLanguage(language);
  for (const error of [
    "Gemini Live Translation did not confirm the session configuration in time.",
    "Baidu realtime translation rejected the session configuration.",
    "Baidu realtime translation rejected the session configuration (code 31003).",
  ]) {
    for (const surface of ["overlay", "control", "tray", "settings", "compact-settings"] as const) {
      await act(async () => root.render(null));
      useStore.setState(state => ({ session: { ...state.session, status: { kind: "error", message: error } } }));
      start.mockClear();
      await mount(surface);
      const retry = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === I18N.settings.sessionRetry)!;
      expect(retry).toBeDefined();
      await act(async () => retry.click());
      expect(start).toHaveBeenCalledOnce();
      expect(showSettings).not.toHaveBeenCalled();
    }
  }
});
