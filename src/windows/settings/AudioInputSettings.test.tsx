// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { audioInputErrorMessage } from "../../lib/audioInput";
import { applicationAudioCopy } from "../../lib/applicationAudio";
import { captureSwitchCopy } from "../../lib/captureStatus";
import type { SessionStateEvent } from "../../lib/types";
import { AudioInputSettings } from "./AudioInputSettings";

vi.mock("./WindowsAudioSource", () => ({ WindowsAudioSource: () => <span data-output-selector /> }));
const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
let switchInput: ReturnType<typeof vi.fn>;
const save = vi.fn();
const start = vi.fn();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  switchInput = vi.fn(initial.switchAudioInput); save.mockReset(); start.mockReset();
  useStore.setState({ ...initial, initializationStatus: "ready", saveSettings: save, switchAudioInput: switchInput, start,
    settings: { ...initial.settings, microphoneInputAvailable: true, audioInput: "system", recordSessionAudio: true },
    session: { ...initial.session, status: { kind: "idle" }, isActive: false, isPaused: false },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(() => root.unmount()); host.remove();
  useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});
async function render() { await act(async () => root.render(<AudioInputSettings />)); }
function toggle(source: "system" | "microphone") { return host.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${source === "system" ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone}"]`)!; }
async function selectMicrophone() { await act(async () => toggle("microphone").click()); }

it.each(["en", "zh", "ja"] as const)("saves the microphone explicitly in %s without starting capture, and clears the prior recording opt-in", async language => {
  setStoredUiLanguage(language); await render();
  expect(toggle("system").getAttribute("aria-checked")).toBe("true");
  expect(host.querySelector('[data-output-selector]')).not.toBeNull();
  await selectMicrophone();
  expect(switchInput).toHaveBeenCalledExactlyOnceWith("both");
  expect(save).not.toHaveBeenCalled();
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("true");
  expect(toggle("system").getAttribute("aria-checked")).toBe("true");
  expect(host.querySelector('[data-output-selector]')).not.toBeNull();
  expect(useStore.getState().settings.recordSessionAudio).toBe(false);
  expect(start).not.toHaveBeenCalled();
  expect(host.querySelector('.settings-row__description')).toBeNull();
  expect(host.querySelector('.settings-help-control__description')?.textContent).toContain(I18N.settings.audioInputHelp);
});

it.each([
  { status: { kind: "connecting" }, isActive: false, isPaused: false },
  { status: { kind: "stopping" }, isActive: false, isPaused: false },
] satisfies Pick<SessionStateEvent, "status" | "isActive" | "isPaused">[])("locks the source in session state %j", async state => {
  useStore.setState({ session: { ...initial.session, ...state } });
  await render();
  expect(toggle("microphone").disabled).toBe(true);
  expect(toggle("system").disabled).toBe(true);
  await selectMicrophone();
  expect(switchInput).not.toHaveBeenCalled();
});

it("prevents duplicate switches and exposes a safe, normal-sized actionable error", async () => {
  let fail!: (reason: Error) => void;
  switchInput.mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  await render();
  await act(() => { toggle("microphone").click(); toggle("microphone").click(); });
  expect(toggle("microphone").disabled).toBe(true);
  await act(() => toggle("microphone").click());
  expect(switchInput).toHaveBeenCalledOnce();
  await act(async () => fail(new Error("synthetic-private-device-details")));
  expect(toggle("microphone").disabled).toBe(false);
  expect(toggle("system").getAttribute("aria-checked")).toBe("true");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(captureSwitchCopy().switchFailed);
  expect(host.textContent).not.toContain("synthetic-private-device-details");
});

it("does not offer input changes until settings have loaded", async () => {
  useStore.setState({ initializationStatus: "loading" }); await render();
  expect(toggle("microphone").disabled).toBe(true);
  expect(switchInput).not.toHaveBeenCalled();
});

it("allows either source alone or both, but never no source", async () => {
  await render();
  expect(toggle("system").disabled).toBe(true);
  await act(async () => toggle("system").click());
  expect(switchInput).not.toHaveBeenCalled();
  await selectMicrophone();
  expect(useStore.getState().settings.audioInput).toBe("both");
  expect(toggle("system").disabled).toBe(false);
  await act(async () => toggle("system").click());
  expect(useStore.getState().settings.audioInput).toBe("microphone");
  expect(toggle("microphone").disabled).toBe(true);
  expect(host.querySelector('[data-output-selector]')).toBeNull();
  await act(async () => toggle("system").click());
  expect(useStore.getState().settings.audioInput).toBe("both");
  await selectMicrophone();
  expect(useStore.getState().settings.audioInput).toBe("system");
  expect(start).not.toHaveBeenCalled();
});

it.each([false, true])("reconfigures while paused=%s and preserves the session and confirmed subtitles", async isPaused => {
  const session = { ...initial.session, status: { kind: "listening" } as const, isActive: !isPaused, isPaused,
    subtitles: { ...initial.session.subtitles, history: [{ source: "Synthetic original", translation: "Synthetic translation", createdAt: 1 }] },
  };
  useStore.setState({ session });
  await render();
  expect(toggle("microphone").disabled).toBe(false);
  await selectMicrophone();
  expect(switchInput).toHaveBeenCalledExactlyOnceWith("both");
  expect(save).not.toHaveBeenCalled();
  expect(useStore.getState().session).toBe(session);
  expect(useStore.getState().settings.recordSessionAudio).toBe(false);
  expect(start).not.toHaveBeenCalled();
  expect(host.textContent).toContain(captureSwitchCopy().switchHelp);
});

it.each([new Error("audio_input_switch_save_failed"), "Microphone capture permission was denied."])("explains a known switching error safely and allows retry: %s", async error => {
  switchInput.mockRejectedValueOnce(error);
  await render(); await selectMicrophone();
  const message = error instanceof Error ? error.message : error;
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(audioInputErrorMessage(message));
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("false");
  expect(toggle("microphone").disabled).toBe(false);
  await selectMicrophone();
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("true");
});

it("explains an unavailable selected application when changing inputs", async () => {
  switchInput.mockRejectedValueOnce("application_audio_unavailable");
  await render(); await selectMicrophone();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().unavailable);
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("false");
});


it.each([false, undefined])("hides the microphone and input switches when availability is %s", async microphoneInputAvailable => {
  useStore.setState({ settings: { ...initial.settings, microphoneInputAvailable } });
  await render();
  expect(host.querySelector('[role="switch"]')).toBeNull();
  expect(host.textContent).not.toContain(I18N.settings.audioInputMicrophone);
  expect(host.textContent).toContain(I18N.settings.audioInputSystem);
  expect(host.querySelector('[data-output-selector]')).not.toBeNull();
  expect(switchInput).not.toHaveBeenCalled();
});
