// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { OverlayControlMode } from "../ipc";
import { useOverlayControlMode } from "./useOverlayControlMode";

const native = vi.hoisted(() => ({
  listen: vi.fn(), read: vi.fn(), unlisten: vi.fn(),
  onMode: undefined as ((mode: OverlayControlMode) => void) | undefined,
}));
vi.mock("../ipc", () => ({
  isTauri: true,
  listenOverlayControlMode: native.listen,
  overlayControlGetState: native.read,
}));
let host: HTMLDivElement, root: Root | null;
function Probe() { const [mode] = useOverlayControlMode(); return <output>{mode}</output>; }
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  native.listen.mockReset().mockImplementation(async (listener: (mode: OverlayControlMode) => void) => {
    native.onMode = listener; return native.unlisten;
  });
  native.read.mockReset().mockResolvedValue("island"); native.unlisten.mockReset();
  native.onMode = undefined;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root?.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("registers before reading and preserves a newer panel event over a delayed initial state", async () => {
  let finish!: (mode: OverlayControlMode) => void;
  native.read.mockImplementation(() => new Promise<OverlayControlMode>(resolve => { finish = resolve; }));
  await act(async () => root!.render(<Probe />));
  expect(native.listen.mock.invocationCallOrder[0]).toBeLessThan(native.read.mock.invocationCallOrder[0]);
  await act(async () => native.onMode?.("panel"));
  await act(async () => finish("island"));
  expect(host.textContent).toBe("panel");
  await act(async () => native.onMode?.("island"));
  expect(host.textContent).toBe("island");
});

it("uses the current panel state when no event overtakes the initial read", async () => {
  native.read.mockResolvedValue("panel");
  await act(async () => root!.render(<Probe />));
  expect(host.textContent).toBe("panel");
});

it("keeps canvas recovery visible if the mode read fails", async () => {
  native.read.mockRejectedValue(new Error("mode unavailable"));
  await act(async () => root!.render(<Probe />));
  expect(host.textContent).not.toBe("panel");
});

it("releases registration that finishes after teardown and ignores late events", async () => {
  let registered!: (unlisten: () => void) => void;
  native.listen.mockImplementation((listener: (mode: OverlayControlMode) => void) => {
    native.onMode = listener;
    return new Promise<() => void>(resolve => { registered = resolve; });
  });
  await act(async () => root!.render(<Probe />));
  await act(async () => root!.unmount()); root = null;
  await act(async () => registered(native.unlisten));
  await act(async () => native.onMode?.("panel"));
  expect(native.unlisten).toHaveBeenCalledOnce();
  expect(native.read).not.toHaveBeenCalled();
  expect(host.textContent).toBe("");
});
