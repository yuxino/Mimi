import { applicationAudioCopy } from "./applicationAudio";
import { expect, it, vi } from "vitest";
import { capturePresentation, captureStatusForSource, type CaptureLifecycle, type CaptureStatus } from "./captureStatus";

const listening: CaptureLifecycle = { status: { kind: "listening" }, isActive: true, isPaused: false };
const system: CaptureStatus = { kind: "windows_output", strategy: "follow_system", actualDeviceName: "Synthetic headphones", observation: { pcmDataRecent: false, soundRecent: false } };

it.each(["en", "zh", "ja"] as const)("keeps source labels short and device/troubleshooting details in help in %s", language => {
  const view = capturePresentation(system, listening, language);
  expect(view.source).toBe({ en: "System audio", zh: "系统声音", ja: "システム音声" }[language]);
  expect(view.observation).toBe({ en: "No audio data", zh: "未收到音频", ja: "データなし" }[language]);
  expect(view.help).toContain("Synthetic headphones");
  expect(view.help).toContain({ en: "Play sound", zh: "请播放声音", ja: "音を再生" }[language]);
  const microphone = capturePresentation({ ...system, kind: "microphone", strategy: "default_input", systemOutputDeviceName: "unrelated output" }, listening, language, "microphone");
  expect(microphone.help).toContain({ en: "default microphone", zh: "默认麦克风", ja: "既定のマイク" }[language]);
  expect(microphone.help).not.toContain("unrelated output");
  expect(microphone.help).not.toContain({ en: "Play sound", zh: "请播放声音", ja: "音を再生" }[language]);
});

it.each(["en", "zh", "ja"] as const)("distinguishes absent observations, missing PCM, silence and sound in %s", language => {
  const states = [null, { pcmDataRecent: false, soundRecent: false }, { pcmDataRecent: true, soundRecent: false }, { pcmDataRecent: true, soundRecent: true }];
  expect(new Set(states.map(observation => capturePresentation({ ...system, observation }, listening, language).observation)).size).toBe(4);
  expect(capturePresentation({ ...system, observation: { pcmDataRecent: false, soundRecent: true } }, listening, language).observation)
    .toBe(capturePresentation(system, listening, language).observation);
});

it.each(["en", "zh", "ja"] as const)("lifecycle states override a stale sound sample in %s", language => {
  const value = { ...system, observation: { pcmDataRecent: true, soundRecent: true } };
  const lifecycle: CaptureLifecycle[] = [
    { ...listening, status: { kind: "idle" }, isActive: false },
    { ...listening, isPaused: true, isActive: false },
    { ...listening, status: { kind: "connecting" } },
    { ...listening, status: { kind: "stopping" } },
    { ...listening, status: { kind: "error", message: "synthetic-private-error" }, isActive: false },
  ];
  const labels = lifecycle.map(state => capturePresentation(value, state, language).observation);
  expect(new Set(labels).size).toBe(5);
  expect(labels).not.toContain(capturePresentation(value, listening, language).observation);
  expect(labels.join()).not.toContain("synthetic-private-error");
});

it("never projects aggregate both-source observations onto individual sources", () => {
  const aggregate: CaptureStatus = { ...system, kind: "both", observation: { pcmDataRecent: true, soundRecent: true } };
  expect(captureStatusForSource(aggregate, "both", "system")).toBeNull();
  expect(captureStatusForSource(aggregate, "both", "microphone")).toBeNull();
  expect(captureStatusForSource(system, "both", "system")).toBeNull();
  const value: CaptureStatus = { ...aggregate, sources: [
    { ...system, audioSource: "system", observation: { pcmDataRecent: true, soundRecent: true } },
    { ...system, audioSource: "microphone", kind: "microphone", observation: { pcmDataRecent: false, soundRecent: false } },
  ] };
  expect(captureStatusForSource(value, "both", "system")?.observation?.soundRecent).toBe(true);
  expect(captureStatusForSource(value, "both", "microphone")?.observation?.pcmDataRecent).toBe(false);
  expect(captureStatusForSource({ ...value, sources: [] }, "system", "system")).toBeNull();
});

it.each(["en", "zh", "ja"] as const)("marks disabled inputs off without stale observations, devices or troubleshooting in %s", language => {
  const value: CaptureStatus = { ...system, kind: "microphone", actualDeviceName: "Synthetic old microphone", observation: { pcmDataRecent: true, soundRecent: true } };
  const view = capturePresentation(value, listening, language, "microphone", false);
  expect(view.observation).toBe({ en: "Off", zh: "已关闭", ja: "オフ" }[language]);
  expect(view.help).not.toContain("Synthetic old microphone");
  expect(view.help).not.toContain({ en: "input level", zh: "输入音量", ja: "入力音量" }[language]);
  expect(capturePresentation(value, { ...listening, status: { kind: "error", message: "synthetic-error" } }, language, "microphone", false).observation).toBe(view.observation);
});

it.each(["en", "zh", "zh-TW", "ja", "de", "fr", "ko", "th", "ru"] as const)("presents a selected Linux stream on the system lane in %s", language => {
  vi.stubGlobal("navigator", { userAgent: "Linux" });
  const stream: CaptureStatus = { ...system, kind: "application", strategy: "selected_application",
    actualDeviceName: "Player · Media #7", systemOutputDeviceName: null };
  const view = capturePresentation(stream, listening, language);
  const text = applicationAudioCopy(language);
  expect(view.help).toContain(text.help);
  expect(view.deviceDescription).toBe(text.application + ": Player · Media #7");
  expect(captureStatusForSource(stream, "system", "system")?.kind).toBe("application");
  expect(captureStatusForSource(stream, "system", "microphone")).toBeNull();
  vi.unstubAllGlobals();
});
