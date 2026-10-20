// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { OverlayControlMode } from "../../lib/ipc";
import { OverlayControlWindow } from "./OverlayControlWindow";
import { useStore } from "../../lib/store";

const native = vi.hoisted(() => ({
  onMode: undefined as ((mode: OverlayControlMode) => void) | undefined,
  reportWidth: vi.fn(async () => {}),
}));

vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(),
  isTauri: true,
  overlayControlGetState: async () => "hidden",
  overlayControlSetIslandWidth: native.reportWidth,
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
