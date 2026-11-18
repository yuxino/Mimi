// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useSessionAction } from "./useSessionAction";

it("acknowledges a click, prevents duplicate commands, and allows retry after failure", async () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div");
  const root = createRoot(host);
  let action!: ReturnType<typeof useSessionAction>;
  function Harness() {
    action = useSessionAction();
    return <output>{action.pending ? "pending" : action.failed ? "failed" : "ready"}</output>;
  }
  let reject!: (reason?: unknown) => void;
  const operation = vi.fn(() => new Promise<void>((_, fail) => { reject = fail; }));
  try {
    await act(async () => root.render(<Harness />));
    let request!: Promise<void>;
    await act(async () => { request = action.run(operation); });
    expect(host.textContent).toBe("pending");
    await act(async () => { await action.run(operation); });
    expect(operation).toHaveBeenCalledTimes(1);
    await act(async () => { reject(new Error("synthetic failure")); await request; });
    expect(host.textContent).toBe("failed");
    expect(host.textContent).not.toContain("synthetic failure");
    await act(async () => { action.clearFailure(); });
    expect(host.textContent).toBe("ready");
    await act(async () => { await action.run(async () => {}); });
    expect(host.textContent).toBe("ready");
  } finally {
    await act(async () => root.unmount());
  }
});
