// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import type { ProfileSwitchFeedback } from "../../lib/ipc";
import { useProfileSwitchFeedback } from "./useProfileSwitchFeedback";

const native = vi.hoisted(() => ({ callback: undefined as ((event: ProfileSwitchFeedback) => void) | undefined, remove: vi.fn() }));
vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(), isTauri: true,
  listenProfileSwitchFeedback: async (callback: (event: ProfileSwitchFeedback) => void) => { native.callback = callback; return native.remove; },
}));
it("shows a rejected switch independently of the popover, filters old replies and clears on the next selection", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  function Canvas() { const feedback = useProfileSwitchFeedback(); return feedback.pending ? <div role="status">pending</div> : feedback.failed ? <div role="alert">{feedback.failureMessage}</div> : null; }
  try {
    await act(async () => root.render(<Canvas />));
    await act(async () => native.callback?.({ requestId: 1, pending: false, error: "apple_speech_status_failed" }));
    expect(host.textContent).toBe(I18N.settings.appleSpeechLoadFailed);
    await act(async () => native.callback?.({ requestId: 2, pending: true, error: null }));
    expect(host.querySelector('[role="status"]')?.textContent).toBe("pending");
    await act(async () => native.callback?.({ requestId: 1, pending: false, error: "apple_speech_assets_missing" }));
    expect(host.querySelector('[role="alert"]')).toBeNull();
    await act(async () => native.callback?.({ requestId: 2, pending: false, error: "private-provider-body" }));
    expect(host.textContent).toBe(I18N.settings.profileActionFailed);
    expect(host.textContent).not.toContain("private-provider-body");
    await act(async () => native.callback?.({ requestId: 3, pending: false, error: null }));
    expect(host.querySelector('[role="alert"]')).toBeNull();
  } finally {
    await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();
  }
  expect(native.remove).toHaveBeenCalledOnce();
  await act(async () => native.callback?.({ requestId: 4, pending: false, error: "apple_speech_status_failed" }));
});
