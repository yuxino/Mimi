// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import type { OverlayControlMode } from "../../lib/ipc";
import { OverlayControlWindow } from "./OverlayControlWindow";
import { overlayControlPanelModel } from "./overlayControlModel";
import { SettingsToastRegion } from "../settings/SettingsToast";
import { useSettingsToast } from "../settings/useSettingsToast";
import { useStore } from "../../lib/store";

const native = vi.hoisted(() => ({
  onMode: undefined as ((mode: OverlayControlMode) => void) | undefined,
  reportWidth: vi.fn(async () => {}),
  toggle: vi.fn(async () => {}),
  nativeShortcuts: true,
  isTauri: true,
}));

vi.mock("../../lib/useDesktopShortcuts", () => ({
  useDesktopShortcuts: () => ({ nativeShortcuts: native.nativeShortcuts }),
}));

vi.mock("./overlayControlModel", async original => {
  const actual = await original<typeof import("./overlayControlModel")>();
  return { ...actual, overlayControlPanelModel: vi.fn(actual.overlayControlPanelModel) };
});

vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  get isTauri() { return native.isTauri; },
  overlayControlGetState: async () => "hidden",
  overlayControlSetIslandWidth: native.reportWidth,
  overlayPopoverToggle: native.toggle,
  listenOverlayControlMode: async (callback: (mode: OverlayControlMode) => void) => {
    native.onMode = callback;
    return () => { native.onMode = undefined; };
  },
}));

// Mounting the actual panel starts its independent capture-status reads.
// The hidden measurement capsule must never mount that surface.
vi.mock("./OverlayControlPanel", () => ({
  OverlayControlPanel: () => <section data-testid="panel" />,
}));

it("keeps the collapsed capsule measured across hidden/panel modes without exposing it or mounting a hidden panel", async () => {
  const previousSession = useStore.getState().session;
  useStore.setState({ session: { ...previousSession, isActive: true, isPaused: false, status: { kind: "listening" } } });
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  const callbacks: (() => void)[] = [];
  const disconnect = vi.fn();
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { callbacks.push(callback); }
    observe() {}
    disconnect = disconnect;
  });
  let width = 130.2;
  const bounds = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockImplementation(() => ({ width } as DOMRect));
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<OverlayControlWindow />));
    const capsule = host.querySelector("button")!;
    const indicator = capsule.querySelector<HTMLElement>(".sound-light")!;
    const originalTrack = indicator.querySelector(".sound-light__wave");
    expect(indicator.dataset.clock).toBe("paused");
    expect(native.reportWidth).toHaveBeenCalledExactlyOnceWith(131);
    expect(capsule.parentElement?.className).toBe("overlay-control-island-measure");
    expect(capsule.parentElement?.getAttribute("aria-hidden")).toBe("true");
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();

    await act(async () => native.onMode?.("panel"));
    expect(host.querySelector('[data-testid="panel"]')).not.toBeNull();
    expect(capsule.parentElement?.getAttribute("aria-hidden")).toBe("true");
    expect(indicator.dataset.clock).toBe("paused");
    width = 156.1;
    callbacks[0]();
    expect(native.reportWidth).toHaveBeenLastCalledWith(157);

    await act(async () => native.onMode?.("island"));
    expect(host.querySelector("button")).toBe(capsule);
    expect(capsule.parentElement?.getAttribute("aria-hidden")).toBeNull();
    expect(indicator.dataset.clock).toBe("running");
    expect(indicator.querySelector(".sound-light__wave")).toBe(originalTrack);
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
    expect(callbacks).toHaveLength(1);
    expect(native.reportWidth).toHaveBeenCalledTimes(2);

    await act(async () => native.onMode?.("hidden"));
    expect(capsule.parentElement?.getAttribute("aria-hidden")).toBe("true");
    expect(indicator.dataset.clock).toBe("paused");
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
  } finally {
    await act(async () => root.unmount());
    useStore.setState({ session: previousSession });
    host.remove();
    bounds.mockRestore();
    native.reportWidth.mockClear();
    vi.unstubAllGlobals();
  }
  expect(disconnect).toHaveBeenCalledOnce();
});

it.each([false, true])("keeps a quiet hover unlock entry with safe retry and normal error recovery (embedded=%s)", async embedded => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const initial = useStore.getState();
  let rejectUnlock!: (error: Error) => void;
  const setOverlayLocked = vi.fn()
    .mockImplementationOnce(() => new Promise<void>((_resolve, reject) => { rejectUnlock = reject; }))
    .mockImplementation(async (locked: boolean) => {
      useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: locked } }));
      native.onMode?.("island");
    });
  const showSettings = vi.fn();
  useStore.setState({
    settings: { ...initial.settings, isOverlayLocked: true, subtitleBlendsWithBackground: false },
    session: { ...initial.session, isActive: true, status: { kind: "listening" } },
    setOverlayLocked,
    showSettings,
  });
  native.toggle.mockImplementation(async () => { native.onMode?.("panel"); });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(() => root.render(<OverlayControlWindow embedded={embedded} />));
    let capsule = host.querySelector<HTMLButtonElement>(".overlay-control-island")!;
    const surface = capsule.closest<HTMLElement>(".overlay-control-embedded, .overlay-control-detached")!;
    expect(capsule.closest('[aria-hidden="true"]')).not.toBeNull();
    await act(() => capsule.click());
    expect(native.toggle).not.toHaveBeenCalled();
    expect(setOverlayLocked).not.toHaveBeenCalled();
    // A delayed native panel event must still leave only the unlock capsule.
    await act(() => native.onMode?.("panel"));
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
    expect(capsule.closest('[aria-hidden="true"], [inert]')).toBeNull();
    if (embedded) expect(surface.getAttribute("data-mode")).toBe("island");
    await act(() => native.onMode?.("island"));
    expect(surface.hasAttribute("data-position-locked")).toBe(false);
    expect(surface.hasAttribute("inert")).toBe(false);
    expect(surface.hasAttribute("data-quiet-island")).toBe(true);
    expect(capsule.getAttribute("aria-label")).toBe(I18N.overlay.unlockPosition);
    expect(capsule.title).toContain("Shift+L");
    expect(capsule.querySelector("[data-pulse-style]")).toBeNull();
    expect(capsule.hasAttribute("aria-haspopup")).toBe(false);
    for (const audioRecording of ["recording", "error"] as const) {
      await act(() => useStore.setState(state => ({ session: { ...state.session, audioRecording } })));
      expect(capsule.querySelector("[data-recording-status]")?.getAttribute("data-recording-status")).toBe(audioRecording);
      expect(surface.hasAttribute("data-quiet-island")).toBe(false);
    }
    await act(() => useStore.setState(state => ({ session: { ...state.session, audioRecording: "off" } })));
    expect(capsule.querySelector("[data-recording-status]")).toBeNull();
    expect(surface.hasAttribute("data-quiet-island")).toBe(true);
    await act(() => { capsule.click(); capsule.click(); });
    expect(setOverlayLocked).toHaveBeenCalledExactlyOnceWith(false);
    expect(capsule.disabled).toBe(true);
    expect(capsule.getAttribute("aria-busy")).toBe("true");
    expect(native.toggle).not.toHaveBeenCalled();
    expect(showSettings).not.toHaveBeenCalled();
    await act(() => rejectUnlock(new Error("private-unlock-failure")));
    expect(capsule.disabled).toBe(false);
    expect(capsule.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
    expect(host.textContent).not.toContain("private-unlock-failure");
    await act(() => window.dispatchEvent(new Event("blur")));
    expect(capsule.querySelector('[role="alert"]')).toBeNull();

    await act(() => capsule.click());
    expect(setOverlayLocked).toHaveBeenCalledTimes(2);
    expect(useStore.getState().settings.isOverlayLocked).toBe(false);
    capsule = host.querySelector<HTMLButtonElement>(".overlay-control-island")!;
    expect(surface.hasAttribute("data-position-locked")).toBe(false);
    expect(surface.hasAttribute("inert")).toBe(false);
    expect(capsule.closest('[aria-hidden="true"]')).toBeNull();
    expect(capsule.getAttribute("aria-label")?.endsWith(I18N.overlay.openControls)).toBe(true);
    expect(capsule.getAttribute("aria-haspopup")).toBe("dialog");
    await act(() => capsule.click());
    expect(native.toggle).toHaveBeenCalledOnce();
    expect(host.querySelector('[data-testid="panel"]')).not.toBeNull();

    await act(() => useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: true } })));
    capsule = host.querySelector<HTMLButtonElement>(".overlay-control-island")!;
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
    expect(surface.hasAttribute("inert")).toBe(false);
    expect(surface.hasAttribute("data-quiet-island")).toBe(true);

    await act(() => {
      useStore.setState(state => ({
        settings: { ...state.settings, isOverlayLocked: true },
        session: { ...state.session, status: { kind: "error", message: "unavailable" } },
      }));
      native.onMode?.("island");
    });
    capsule = host.querySelector<HTMLButtonElement>(".overlay-control-island")!;
    expect(surface.hasAttribute("data-position-locked")).toBe(false);
    expect(surface.hasAttribute("inert")).toBe(false);
    expect(surface.hasAttribute("data-quiet-island")).toBe(false);
    expect(capsule.getAttribute("aria-label")?.endsWith(I18N.overlay.openControls)).toBe(true);
    await act(() => capsule.click());
    expect(native.toggle).toHaveBeenCalledTimes(2);
    expect(showSettings).not.toHaveBeenCalled();
  } finally {
    await act(() => root.unmount());
    host.remove();
    useStore.setState(initial, true);
    native.toggle.mockReset();
    native.reportWidth.mockClear();
    vi.unstubAllGlobals();
  }
});

it("does not rederive normal panel controls when a transient notification changes", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const initial = useStore.getState();
  useStore.setState({ settings: { ...initial.settings, isOverlayLocked: false } });
  let notify!: (message: string) => void;
  function Notification() {
    const { beginToast } = useSettingsToast();
    notify = message => beginToast()(message);
    return <SettingsToastRegion />;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(() => root.render(<><OverlayControlWindow /><Notification /></>));
    await act(() => native.onMode?.("panel"));
    const panel = host.querySelector('[data-testid="panel"]');
    vi.mocked(overlayControlPanelModel).mockClear();
    await act(() => notify("Synthetic feedback"));
    expect(host.querySelector('[role="status"]')?.textContent).toBe("Synthetic feedback");
    expect(host.querySelector('[data-testid="panel"]')).toBe(panel);
    expect(overlayControlPanelModel).not.toHaveBeenCalled();
  } finally {
    await act(() => root.unmount());
    host.remove();
    useStore.setState(initial, true);
    native.reportWidth.mockClear();
    vi.unstubAllGlobals();
  }
});

it.each([false, true])("keeps a locked panel folded when settings unlock before the native mode update (preview=%s)", async preview => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const initial = useStore.getState();
  const previousUrl = window.location.href;
  native.isTauri = !preview;
  if (preview) window.history.replaceState(null, "", "?mode=panel");
  useStore.setState({ settings: { ...initial.settings, isOverlayLocked: false } });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(() => root.render(<OverlayControlWindow />));
    if (!preview) await act(() => native.onMode?.("panel"));
    expect(host.querySelector('[data-testid="panel"]')).not.toBeNull();
    await act(() => useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: true } })));
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
    await act(() => useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: false } })));
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
    const capsule = host.querySelector<HTMLButtonElement>(".overlay-control-island")!;
    expect(capsule.closest('[aria-hidden="true"]')).toBeNull();
    if (!preview) await act(() => native.onMode?.("island"));
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
    expect(native.toggle).not.toHaveBeenCalled();
  } finally {
    await act(() => root.unmount());
    host.remove();
    useStore.setState(initial, true);
    native.isTauri = true;
    native.reportWidth.mockClear();
    window.history.replaceState(null, "", previousUrl);
    vi.unstubAllGlobals();
  }
});

it.each([false, true])("shows the Mac unlock shortcut only with native shortcut support=%s", async supported => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const userAgent = vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Macintosh");
  const initial = useStore.getState();
  native.nativeShortcuts = supported;
  useStore.setState({ settings: { ...initial.settings, isOverlayLocked: true } });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(() => root.render(<OverlayControlWindow />));
    await act(() => native.onMode?.("island"));
    const capsule = host.querySelector<HTMLButtonElement>(".overlay-control-island")!;
    expect(capsule.title).toBe(`${I18N.overlay.unlockPosition}${supported ? " · ⌘⇧L" : ""}`);
    expect(capsule.getAttribute("aria-label")).toBe(I18N.overlay.unlockPosition);
  } finally {
    await act(() => root.unmount());
    host.remove();
    useStore.setState(initial, true);
    native.nativeShortcuts = true;
    native.reportWidth.mockClear();
    userAgent.mockRestore();
    vi.unstubAllGlobals();
  }
});
