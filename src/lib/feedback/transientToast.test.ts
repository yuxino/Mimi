import { afterEach, describe, expect, it, vi } from "vitest";
import { TransientToast } from "./transientToast";

afterEach(() => vi.useRealTimers());
describe("transient feedback lifecycle", () => {
  it("dismisses success after three seconds", () => {
    vi.useFakeTimers(); const publish = vi.fn(); const toast = new TransientToast(publish);
    toast.show("copied", false); vi.advanceTimersByTime(2999); expect(publish).toHaveBeenLastCalledWith("copied");
    vi.advanceTimersByTime(1); expect(publish).toHaveBeenLastCalledWith(null);
  });
  it("replaces repeated feedback and restarts one timer", () => {
    vi.useFakeTimers(); const publish = vi.fn(); const toast = new TransientToast(publish);
    toast.show("copied", false); vi.advanceTimersByTime(2000); toast.show("copied", false);
    expect(vi.getTimerCount()).toBe(1); vi.advanceTimersByTime(1000); expect(publish).toHaveBeenLastCalledWith("copied");
    vi.advanceTimersByTime(2000); expect(publish).toHaveBeenLastCalledWith(null);
  });
  it("keeps failure readable longer, then expires", () => {
    vi.useFakeTimers(); const publish = vi.fn(); const toast = new TransientToast(publish);
    toast.show("failed", true); vi.advanceTimersByTime(7999); expect(publish).toHaveBeenLastCalledWith("failed");
    vi.advanceTimersByTime(1); expect(publish).toHaveBeenLastCalledWith(null);
  });
  it("clears on navigation/manual close and cancels all work on unmount", () => {
    vi.useFakeTimers(); const publish = vi.fn(); const toast = new TransientToast(publish);
    toast.show("failed", true); toast.clear(); expect(vi.getTimerCount()).toBe(0); expect(publish).toHaveBeenLastCalledWith(null);
    toast.show("copied", false); toast.dispose(); publish.mockClear(); vi.runAllTimers(); expect(publish).not.toHaveBeenCalled();
  });
});
