// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import type { OverlayControlMode } from "../../lib/ipc";
import { OverlayControlWindow } from "./OverlayControlWindow";
import { useStore } from "../../lib/store";

const native = vi.hoisted(() => ({
  onMode: undefined as ((mode: OverlayControlMode) => void) | undefined,
  reportWidth: vi.fn(async () => {}),
  toggle: vi.fn(async () => {}),
}));

vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  isTauri: true,
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

it.each([false, true])("hides locked controls until a settings update unlocks them or an error needs recovery (embedded=%s)", async embedded => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const initial = useStore.getState();
  useStore.setState({
    settings: { ...initial.settings, isOverlayLocked: true },
    session: { ...initial.session, isActive: true, status: { kind: "listening" } },
  });
  native.toggle.mockImplementation(async () => { native.onMode?.("panel"); });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(() => root.render(<OverlayControlWindow embedded={embedded} />));
    const capsule = host.querySelector<HTMLButtonElement>(".overlay-control-island")!;
    const surface = capsule.closest<HTMLElement>(".overlay-control-embedded, .overlay-control-detached")!;
    expect(capsule.closest('[aria-hidden="true"]')).not.toBeNull();
    await act(() => capsule.click());
    expect(native.toggle).not.toHaveBeenCalled();
    // A delayed native island/panel event must not expose the saved lock.
    await act(() => native.onMode?.("panel"));
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
    await act(() => native.onMode?.("island"));
    expect(surface.hasAttribute("data-position-locked")).toBe(true);
    expect(surface.hasAttribute("inert")).toBe(true);
    expect(capsule.closest('[aria-hidden="true"]')).not.toBeNull();
    expect(capsule.querySelector<HTMLElement>("[data-pulse-style]")?.dataset.clock).toBe("paused");
    await act(() => capsule.click());
    expect(native.toggle).not.toHaveBeenCalled();

    await act(() => useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: false } })));
    expect(surface.hasAttribute("data-position-locked")).toBe(false);
    expect(surface.hasAttribute("inert")).toBe(false);
    expect(capsule.closest('[aria-hidden="true"]')).toBeNull();
    expect(capsule.getAttribute("aria-label")?.endsWith(I18N.overlay.openControls)).toBe(true);
    expect(capsule.getAttribute("aria-haspopup")).toBe("dialog");
    await act(() => capsule.click());
    expect(native.toggle).toHaveBeenCalledOnce();
    expect(host.querySelector('[data-testid="panel"]')).not.toBeNull();

    await act(() => useStore.setState(state => ({ settings: { ...state.settings, isOverlayLocked: true } })));
    expect(host.querySelector('[data-testid="panel"]')).toBeNull();
    expect(surface.hasAttribute("inert")).toBe(true);

    await act(() => {
      useStore.setState(state => ({
        settings: { ...state.settings, isOverlayLocked: true },
        session: { ...state.session, status: { kind: "error", message: "unavailable" } },
      }));
      native.onMode?.("island");
    });
    expect(surface.hasAttribute("data-position-locked")).toBe(false);
    expect(surface.hasAttribute("inert")).toBe(false);
    expect(capsule.getAttribute("aria-label")?.endsWith(I18N.overlay.openControls)).toBe(true);
    await act(() => capsule.click());
    expect(native.toggle).toHaveBeenCalledTimes(2);
  } finally {
    await act(() => root.unmount());
    host.remove();
    useStore.setState(initial, true);
    native.toggle.mockReset();
    native.reportWidth.mockClear();
    vi.unstubAllGlobals();
  }
});
