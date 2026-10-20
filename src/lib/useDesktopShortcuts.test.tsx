// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DesktopShortcutCommands } from "./ipc";

const native = vi.hoisted(() => ({ isTauri: true, read: vi.fn() }));
vi.mock("./ipc", () => ({
  get isTauri() { return native.isTauri; },
  appDesktopShortcutCommands: native.read,
}));

const wayland: DesktopShortcutCommands = {
  toggleSession: "'/opt/Mimi.AppImage' --toggle-session",
  toggleImmersive: "'/opt/Mimi.AppImage' --toggle-immersive",
  cycleSubtitleDisplay: "'/opt/Mimi.AppImage' --cycle-subtitle-display",
};
let useDesktopShortcuts: typeof import("./useDesktopShortcuts")["useDesktopShortcuts"];
let host: HTMLDivElement, root: Root;

function Probe({ id }: { id: string }) {
  const { commands, nativeShortcuts } = useDesktopShortcuts();
  return <output data-id={id} data-native={nativeShortcuts}>
    {commands === undefined ? "unknown" : commands === null ? "native" : commands.toggleSession}
  </output>;
}

function deferred() {
  let resolve!: (value: DesktopShortcutCommands | null) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<DesktopShortcutCommands | null>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.resetModules();
  native.isTauri = true;
  native.read.mockReset().mockResolvedValue(null);
  ({ useDesktopShortcuts } = await import("./useDesktopShortcuts"));
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
async function render(ids = ["a"]) { await act(async () => root.render(<>{ids.map(id => <Probe key={id} id={id} />)}</>)); }

it("shares one pending native read across three consumers without canceling the remaining consumers on unmount", async () => {
  const pending = deferred(); native.read.mockReturnValue(pending.promise);
  await render(["a", "b", "c"]);
  expect(native.read).toHaveBeenCalledOnce();
  expect([...host.querySelectorAll("output")].map(node => node.textContent)).toEqual(["unknown", "unknown", "unknown"]);
  await render(["b", "c"]);
  await act(async () => pending.resolve(null));
  expect(native.read).toHaveBeenCalledOnce();
  expect([...host.querySelectorAll("output")].map(node => node.textContent)).toEqual(["native", "native"]);
});

it.each([null, wayland])("reuses a successful native result after all consumers unmount: %j", async commands => {
  native.read.mockResolvedValue(commands);
  await render();
  const expected = commands?.toggleSession ?? "native";
  expect(host.textContent).toBe(expected);
  await render([]);
  await render(["replacement"]);
  expect(host.textContent).toBe(expected);
  expect(host.querySelector("output")?.getAttribute("data-native")).toBe(String(commands === null));
  expect(native.read).toHaveBeenCalledOnce();
});

it("leaves native shortcut support unknown on failure and allows a later consumer to retry", async () => {
  native.read.mockRejectedValueOnce(new Error("native bridge unavailable")).mockResolvedValue(wayland);
  await render();
  expect(host.textContent).toBe("unknown");
  expect(host.querySelector("output")?.getAttribute("data-native")).toBe("false");
  await render([]);
  await render(["retry"]);
  expect(native.read).toHaveBeenCalledTimes(2);
  expect(host.textContent).toBe(wayland.toggleSession);
});

it("keeps a pending read reusable after full unmount and accepts its late reply in a replacement consumer", async () => {
  const pending = deferred(); native.read.mockReturnValue(pending.promise);
  await render();
  await render([]);
  await render(["replacement"]);
  expect(native.read).toHaveBeenCalledOnce();
  await act(async () => pending.resolve(wayland));
  expect(host.textContent).toBe(wayland.toggleSession);
  expect(native.read).toHaveBeenCalledOnce();
});

it("does not cache a read that rejects after all of its consumers unmount", async () => {
  const pending = deferred(); native.read.mockReturnValueOnce(pending.promise).mockResolvedValue(null);
  await render();
  await render([]);
  await act(async () => pending.reject(new Error("late native failure")));
  await render(["retry"]);
  expect(native.read).toHaveBeenCalledTimes(2);
  expect(host.textContent).toBe("native");
});

it("preserves the browser preview fallback without making a native request", async () => {
  native.isTauri = false;
  await render(["a", "b", "c"]);
  expect(native.read).not.toHaveBeenCalled();
  expect([...host.querySelectorAll("output")].map(node => node.textContent)).toEqual(["native", "native", "native"]);
});
