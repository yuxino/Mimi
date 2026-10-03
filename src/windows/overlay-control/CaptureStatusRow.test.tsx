// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { captureSwitchCopy, type CaptureStatus } from "../../lib/captureStatus";
import { applicationAudioCopy } from "../../lib/applicationAudio";
import { mergeSettingsSnapshot } from "../../lib/settingsState";
import type { AudioInput, AudioSource, SessionStateEvent } from "../../lib/types";
import { CaptureStatusRow } from "./CaptureStatusRow";
import type { SystemAudioTarget } from "../../lib/types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), switchAudioInput: vi.fn(), switchSystemAudioTarget: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../lib/ipc", () => ({ isTauri: true, setOverlayPointerCursor: vi.fn() }));
let host: HTMLDivElement;
let root: Root;
const initial = useStore.getState();
const system: CaptureStatus = { kind: "macos_system_mix", strategy: "platform_capture", actualDeviceName: null, observation: { pcmDataRecent: true, soundRecent: true }, systemOutputDeviceName: "Synthetic long headphones name" };
const microphone: CaptureStatus = { kind: "microphone", strategy: "default_input", actualDeviceName: "Synthetic microphone", systemOutputDeviceName: "Unrelated output", observation: { pcmDataRecent: false, soundRecent: false } };
const dual: CaptureStatus = { ...system, kind: "both", sources: [
  { ...system, audioSource: "system" }, { ...microphone, audioSource: "microphone" },
] };

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  setStoredUiLanguage("en");
  useStore.setState({ ...initial, initializationStatus: "ready", switchAudioInput: mocks.switchAudioInput, switchSystemAudioTarget: mocks.switchSystemAudioTarget, session: { ...initial.session, status: { kind: "listening" }, isActive: true, isPaused: false } }, true);
  mocks.invoke.mockResolvedValue(system);
  mocks.switchAudioInput.mockImplementation(async (audioInput: AudioInput) => {
    useStore.setState(state => ({ settings: { ...state.settings, audioInput, recordSessionAudio: false } }));
  });
  mocks.switchSystemAudioTarget.mockImplementation(async (target: SystemAudioTarget) => {
    useStore.setState(state => ({ settings: mergeSettingsSnapshot(state.settings, { systemAudioTarget: target }) }));
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); useStore.setState(initial, true);
  setStoredUiLanguage("system"); vi.restoreAllMocks(); vi.clearAllMocks(); vi.unstubAllGlobals();
});
async function mount(disabled = false) { await act(async () => root.render(<CaptureStatusRow disabled={disabled} />)); }
function row(source: AudioSource) { return host.querySelector<HTMLElement>(`[data-audio-source="${source}"]`)!; }
function toggle(source: AudioSource) { return row(source).querySelector<HTMLButtonElement>('[role="switch"]')!; }
function help(source: AudioSource) { return row(source).querySelector<HTMLButtonElement>('.settings-help-control__button')!; }
function description(source: AudioSource) { return document.getElementById(help(source).getAttribute("aria-describedby")!)?.textContent ?? ""; }
function status(source: AudioSource) { return description(source).split("\n")[0]; }
async function session(patch: Partial<SessionStateEvent>) { await act(async () => useStore.setState(state => ({ session: { ...state.session, ...patch } }))); }

it("always shows both switches, defaulting to system on and microphone off, with details only in help", async () => {
  await mount();
  expect(host.querySelectorAll('[role="switch"]')).toHaveLength(2);
  expect(toggle("system").getAttribute("aria-checked")).toBe("true");
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("false");
  expect(toggle("system").disabled).toBe(true);
  expect(toggle("microphone").disabled).toBe(false);
  expect(row("system").querySelector('.overlay-control-capture__source')?.textContent).toBe("System audio");
  expect(status("system")).toBe("Receiving sound");
  expect(status("microphone")).toBe("Off");
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect(host.querySelector('.overlay-control-capture__settings')).toBeNull();
  expect(description("system")).toContain("System output: Synthetic long headphones name");
  expect(description("system")).toContain("Keep at least one input on");
  expect(description("microphone")).not.toContain("Waiting for audio");
  const button = help("system");
  const matches = button.matches.bind(button);
  vi.spyOn(button, "matches").mockImplementation(selector => selector === ":focus-visible" || matches(selector));
  await act(() => button.focus());
  expect(document.querySelector('[role="tooltip"]')?.textContent).toContain("Synthetic long headphones name");
});

it("switches independently while listening, keeping at least one input selected", async () => {
  await mount();
  await act(async () => toggle("system").click());
  expect(mocks.switchAudioInput).not.toHaveBeenCalled();
  await act(async () => toggle("microphone").click());
  expect(mocks.switchAudioInput).toHaveBeenLastCalledWith("both");
  expect(toggle("system").disabled).toBe(false);
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("true");
  await act(async () => toggle("system").click());
  expect(mocks.switchAudioInput).toHaveBeenLastCalledWith("microphone");
  expect(toggle("system").getAttribute("aria-checked")).toBe("false");
  expect(toggle("microphone").disabled).toBe(true);
  await act(async () => toggle("system").click());
  await act(async () => toggle("microphone").click());
  expect(mocks.switchAudioInput).toHaveBeenLastCalledWith("system");
  expect(toggle("system").disabled).toBe(true);
});

it("locks both switches while a change is pending and ignores repeated clicks", async () => {
  let resolve!: () => void;
  mocks.switchAudioInput.mockImplementationOnce(() => new Promise<void>(done => { resolve = done; }));
  await mount();
  await act(async () => { toggle("microphone").click(); toggle("microphone").click(); });
  expect(mocks.switchAudioInput).toHaveBeenCalledExactlyOnceWith("both");
  expect(toggle("system").disabled).toBe(true);
  expect(toggle("microphone").disabled).toBe(true);
  expect(host.querySelector('[aria-busy="true"]')).not.toBeNull();
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("false");
  await act(async () => resolve());
  expect(toggle("microphone").disabled).toBe(false);
});

it.each(["connecting", "stopping"] as const)("disables switching while %s", async kind => {
  useStore.setState({ settings: { ...initial.settings, audioInput: "both" } });
  await session({ status: { kind } });
  await mount();
  expect(toggle("system").disabled).toBe(true);
  expect(toggle("microphone").disabled).toBe(true);
  await act(async () => toggle("microphone").click());
  expect(mocks.switchAudioInput).not.toHaveBeenCalled();
});

it("waits for settings initialization and honors the panel action lock", async () => {
  useStore.setState({ initializationStatus: "loading" });
  await mount();
  expect(toggle("microphone").disabled).toBe(true);
  await act(async () => useStore.setState({ initializationStatus: "ready" }));
  expect(toggle("microphone").disabled).toBe(false);
  await mount(true);
  expect(toggle("microphone").disabled).toBe(true);
});

it("allows switching while paused without asking to resume the session", async () => {
  await session({ isPaused: true, isActive: false });
  await mount();
  expect(toggle("microphone").disabled).toBe(false);
  await act(async () => toggle("microphone").click());
  expect(mocks.switchAudioInput).toHaveBeenCalledExactlyOnceWith("both");
  expect(useStore.getState().session.isPaused).toBe(true);
  expect(status("microphone")).toBe("Paused");
  expect(description("microphone")).toContain("Paused subtitles stay paused");
});

it("shows a safe actionable failure and lets the user retry without optimistic switch changes", async () => {
  mocks.switchAudioInput.mockRejectedValueOnce(new Error("synthetic-private-server-error"));
  await mount();
  await act(async () => toggle("microphone").click());
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("false");
  expect(toggle("microphone").disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(captureSwitchCopy().switchFailed);
  expect(host.textContent).not.toContain("synthetic-private-server-error");
  await act(async () => toggle("microphone").click());
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(toggle("microphone").getAttribute("aria-checked")).toBe("true");
});

it("explains a known microphone permission failure without exposing raw errors", async () => {
  mocks.switchAudioInput.mockRejectedValueOnce("Microphone capture permission was denied.");
  await mount();
  await act(async () => toggle("microphone").click());
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("system privacy settings");
});

it.each([false, true])("selects a real application from the floating panel while paused=%s", async isPaused => {
  vi.stubGlobal("navigator", { userAgent: "Macintosh" });
  Element.prototype.scrollIntoView = vi.fn();
  mocks.invoke.mockImplementation(async command => command === "audio_applications"
    ? { supported: true, applications: [{ id: "example.player", name: "Player" }] } : system);
  await session({ isPaused, isActive: !isPaused });
  await mount();
  expect(mocks.invoke).not.toHaveBeenCalledWith("audio_applications");
  const target = row("system").querySelector<HTMLButtonElement>('button[role="combobox"]')!;
  expect(target.disabled).toBe(false);
  await act(async () => target.click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === "Player")!;
  await act(async () => option.click());
  expect(mocks.switchSystemAudioTarget).toHaveBeenCalledExactlyOnceWith({ kind: "application", id: "example.player", name: "Player" });
  expect(useStore.getState().session.isPaused).toBe(isPaused);
  expect(target.textContent).toContain("Player");
  expect(mocks.switchAudioInput).not.toHaveBeenCalled();
});

it("locks the input switches while an application change is pending and maps known target failures", async () => {
  vi.stubGlobal("navigator", { userAgent: "Macintosh" });
  Element.prototype.scrollIntoView = vi.fn();
  mocks.invoke.mockImplementation(async command => command === "audio_applications"
    ? { supported: true, applications: [{ id: "example.player", name: "Player" }] } : system);
  let reject!: (reason: unknown) => void;
  mocks.switchSystemAudioTarget.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  await mount();
  const target = row("system").querySelector<HTMLButtonElement>('button[role="combobox"]')!;
  await act(async () => target.click());
  const option = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === "Player")!;
  await act(async () => option.click());
  expect(toggle("microphone").disabled).toBe(true);
  await act(async () => toggle("microphone").click());
  expect(mocks.switchAudioInput).not.toHaveBeenCalled();
  await act(async () => reject("application_audio_unavailable"));
  expect(toggle("microphone").disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().unavailable);
  expect(target.textContent).toContain(applicationAudioCopy().all);
});

it("explains unavailable application failures from an input toggle", async () => {
  mocks.switchAudioInput.mockRejectedValueOnce("application_audio_unavailable");
  await mount();
  await act(async () => toggle("microphone").click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().unavailable);
});

it.each([
  ["zh", "收到声音", "未收到音频"], ["en", "Receiving sound", "No audio data"], ["ja", "音声を受信中", "データなし"],
] as const)("keeps truthful independent observations in each source tooltip in %s", async (language, sound, noData) => {
  setStoredUiLanguage(language);
  useStore.setState({ settings: { ...initial.settings, audioInput: "both" } });
  mocks.invoke.mockResolvedValue(dual);
  await mount();
  expect(status("system")).toBe(sound);
  expect(status("microphone")).toBe(noData);
  expect(description("microphone")).toContain("Synthetic microphone");
  expect(description("microphone")).not.toContain("Unrelated output");
  expect(description("microphone")).toContain({ zh: "默认麦克风", en: "default microphone", ja: "既定のマイク" }[language]);
  expect(description("microphone")).toContain({ zh: "用量分别计算", en: "separate recognition and usage", ja: "利用量も個別" }[language]);
});

it("keeps switching available when capture status is unavailable", async () => {
  mocks.invoke.mockRejectedValue(new Error("synthetic-status-failure"));
  await mount();
  expect(status("system")).toBe("Waiting for audio");
  expect(status("microphone")).toBe("Off");
  await act(async () => toggle("microphone").click());
  expect(mocks.switchAudioInput).toHaveBeenCalledExactlyOnceWith("both");
});

it("does not invent per-source sound for a legacy aggregate both response", async () => {
  useStore.setState({ settings: { ...initial.settings, audioInput: "both" } });
  mocks.invoke.mockResolvedValue({ ...dual, sources: undefined });
  await mount();
  expect(status("system")).toBe("Waiting for audio");
  expect(status("microphone")).toBe("Waiting for audio");
});

it.each([
  [{ status: { kind: "connecting" } }, "Connecting"],
  [{ status: { kind: "stopping" } }, "Stopping"],
  [{ isPaused: true, isActive: false }, "Paused"],
  [{ status: { kind: "error", message: "synthetic-error" }, isActive: false }, "Error"],
  [{ status: { kind: "idle" }, isActive: false }, "Not capturing"],
] satisfies [Partial<SessionStateEvent>, string][])("gives lifecycle %j priority over old sound", async (patch, label) => {
  useStore.setState({ settings: { ...initial.settings, audioInput: "both" } });
  mocks.invoke.mockResolvedValue(dual);
  await mount();
  mocks.invoke.mockImplementationOnce(() => new Promise(() => {}));
  await session(patch);
  expect(status("system")).toBe(label);
  expect(status("microphone")).toBe(label);
  expect(description("system")).not.toContain("Synthetic long headphones name");
});

it("ignores pending reads from old source and lifecycle stamps, including returning to listening", async () => {
  let finishOld!: (value: CaptureStatus) => void;
  mocks.invoke.mockImplementationOnce(() => new Promise(resolve => { finishOld = resolve; }));
  await mount();
  mocks.invoke.mockResolvedValueOnce(microphone);
  await act(async () => useStore.setState({ settings: { ...initial.settings, audioInput: "microphone" } }));
  await act(async () => finishOld(system));
  expect(status("microphone")).toBe("No audio data");
  expect(status("system")).toBe("Off");
  expect(description("microphone")).not.toContain("Synthetic long headphones name");
  mocks.invoke.mockImplementationOnce(() => new Promise(() => {}));
  await session({ status: { kind: "connecting" } });
  let finishNew!: (value: CaptureStatus) => void;
  mocks.invoke.mockImplementationOnce(() => new Promise(resolve => { finishNew = resolve; }));
  await session({ status: { kind: "listening" } });
  expect(status("microphone")).toBe("Waiting for audio");
  await act(async () => finishNew({ ...microphone, observation: { pcmDataRecent: true, soundRecent: false } }));
  expect(status("microphone")).toBe("No sound");
});


it("keeps long device names bounded in help and readable in full through the source title", async () => {
  const device = "Synthetic headphones " + "耳機".repeat(40);
  mocks.invoke.mockResolvedValueOnce({ ...system, systemOutputDeviceName: device });
  await mount();
  expect(description("system")).not.toContain(device);
  expect(description("system")).toContain(`${Array.from(device).slice(0, 39).join("")}…`);
  expect(row("system").querySelector("label")?.title).toBe(`System output: ${device}`);
});

it("keeps restrictions in the relevant source help without repeating dual-input advice", async () => {
  await mount();
  expect(description("system")).toContain("Keep at least one input on");
  expect(description("system")).not.toContain("separate recognition and usage");
  expect(description("microphone")).toContain("separate recognition and usage");
  expect(description("microphone")).not.toContain("Keep at least one input on");
  await act(async () => useStore.setState({ settings: { ...initial.settings, audioInput: "microphone" } }));
  expect(description("microphone")).toContain("Keep at least one input on");
  expect(description("microphone")).not.toContain("separate recognition and usage");
  expect(description("system")).not.toContain("Keep at least one input on");
});
