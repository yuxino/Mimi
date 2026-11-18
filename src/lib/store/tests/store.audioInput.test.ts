import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../../store";

const native = vi.hoisted(() => ({ switchInput: vi.fn(), save: vi.fn() }));
vi.mock("../../ipc", async original => ({
  ...await original<typeof import("../../ipc")>(), isTauri: true,
  sessionSwitchAudioInput: native.switchInput, settingsSave: native.save,
}));
const initial = useStore.getState();
beforeEach(() => { useStore.setState(initial, true); native.switchInput.mockReset(); native.save.mockReset(); });
afterEach(() => useStore.setState(initial, true));

it.each([false, true])("uses the native live-input command while paused=%s without faking a successful choice", async isPaused => {
  let reject!: (error: Error) => void;
  native.switchInput.mockImplementation(() => new Promise<void>((_resolve, fail) => { reject = fail; }));
  useStore.setState({ session: { ...initial.session, isActive: true, isPaused, status: { kind: "listening" } } });
  const session = useStore.getState().session;
  const operation = useStore.getState().switchAudioInput("both");
  expect(native.switchInput).toHaveBeenCalledExactlyOnceWith("both");
  expect(native.save).not.toHaveBeenCalled();
  expect(useStore.getState().settings.audioInput).toBe("system");
  expect(useStore.getState().session).toBe(session);
  const failure = expect(operation).rejects.toThrow("audio_input_switch_save_failed");
  reject(new Error("audio_input_switch_save_failed")); await failure;
  expect(useStore.getState().settings.audioInput).toBe("system");
});

it.each(["connecting", "stopping"] as const)("does not queue a source change during %s", async kind => {
  useStore.setState({ session: { ...initial.session, status: { kind } } });
  await expect(useStore.getState().switchAudioInput("both")).rejects.toThrow("audio_input_switch_busy");
  expect(native.switchInput).not.toHaveBeenCalled();
});
