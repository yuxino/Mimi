// @vitest-environment jsdom
import { act, useEffect } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { getWindowsLiveCaptionsSupport, openWindowsLiveCaptions } from "../../lib/ipc";
import type { WindowsLiveCaptionsSupport } from "../../lib/types";
import { useWindowsLiveCaptionsSupport } from "./useWindowsLiveCaptionsSupport";

vi.mock("../../lib/ipc", () => ({ getWindowsLiveCaptionsSupport: vi.fn(), openWindowsLiveCaptions: vi.fn() }));
let root: Root, host: HTMLDivElement, current: ReturnType<typeof useWindowsLiveCaptionsSupport>;
function Harness({ visible }: { visible: boolean }) {
  const state = useWindowsLiveCaptionsSupport(visible);
  useEffect(() => { current = state; }, [state]);
  return null;
}
function pending() {
  let resolve!: (support: WindowsLiveCaptionsSupport) => void;
  let reject!: () => void;
  const promise = new Promise<WindowsLiveCaptionsSupport>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.clearAllMocks();
  vi.mocked(getWindowsLiveCaptionsSupport).mockResolvedValue({ available: true, status: "closed" });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
async function render(visible = true) { await act(async () => root.render(<Harness visible={visible} />)); }
it("queries only visible settings and never opens Windows captions", async () => {
  await render(false); expect(getWindowsLiveCaptionsSupport).not.toHaveBeenCalled();
  await render(); expect(getWindowsLiveCaptionsSupport).toHaveBeenCalledOnce();
  expect(current.support?.status).toBe("closed"); expect(current.loading).toBe(false);
  expect(openWindowsLiveCaptions).not.toHaveBeenCalled();
});
it("ignores stale failures and snapshots after a newer explicit refresh", async () => {
  const old = pending(), recent = pending();
  vi.mocked(getWindowsLiveCaptionsSupport).mockReturnValueOnce(old.promise).mockReturnValueOnce(recent.promise);
  await render();
  let refreshing!: Promise<void>;
  await act(async () => { refreshing = current.refresh(); });
  await act(async () => recent.resolve({ available: true, status: "ready" })); await refreshing;
  await act(async () => old.reject());
  expect(current.support?.status).toBe("ready"); expect(current.failed).toBe(false); expect(current.loading).toBe(false);
});
it("clears unknown availability on failure and recovers through an explicit retry", async () => {
  vi.mocked(getWindowsLiveCaptionsSupport).mockRejectedValueOnce(new Error("synthetic-native-error"));
  await render(); expect(current.support).toBeNull(); expect(current.failed).toBe(true);
  await act(async () => current.refresh());
  expect(current.support?.status).toBe("closed"); expect(current.failed).toBe(false);
});
