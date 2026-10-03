// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { shareUnchangedSubtitleHistory } from "../../lib/sessionSnapshot";
import { useStore } from "../../lib/store";
import type { SessionStateEvent, SettingsSnapshot, SubtitleDisplayMode, SubtitleSnapshot } from "../../lib/types";
import { OverlayWindow } from "./OverlayWindow";

// Keep the real projection, stabilizers, sentence blocks and Timeline DOM.
// These unrelated native affordances have no bearing on reading-mode output.
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => null }));

const original = useStore.getState();
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalScrollTo = HTMLElement.prototype.scrollTo;
const clientHeightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
const modes: SubtitleDisplayMode[] = ["original", "translation", "bilingual"];
const empty: SubtitleSnapshot = {
  source: { text: "", isFinal: false }, translation: { text: "", isFinal: false },
  history: [], previewPair: null,
};
const confirmed = { source: "Confirmed original.", translation: "已确认译文。", createdAt: 10 };
let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  HTMLElement.prototype.scrollTo = vi.fn();
  HTMLElement.prototype.getBoundingClientRect = () => ({ top: 0, bottom: 24, height: 24 } as DOMRect);
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(this: HTMLElement) { return Number.parseFloat(this.style.height) || 100; },
  });
  setStoredUiLanguage("en");
  useStore.setState({ ...original,
    session: snapshot(empty),
    settings: { ...original.settings, profiles: [{ id: "atomic", name: "Alibaba Cloud", provider: "alibabaCloud", credentialState: "present" }],
      activeProfileId: "atomic", sourceLanguage: "auto", targetLanguage: "zh", subtitleDisplayMode: "bilingual",
      pulseAnimation: false, subtitleAnimation: false, subtitleBlendsWithBackground: false, isOverlayLocked: false },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

afterEach(async () => {
  await act(() => root.unmount());
  host.remove(); useStore.setState(original, true); setStoredUiLanguage("system");
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  HTMLElement.prototype.scrollTo = originalScrollTo;
  if (clientHeightDescriptor) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeightDescriptor);
  else Reflect.deleteProperty(HTMLElement.prototype, "clientHeight");
  vi.useRealTimers(); vi.unstubAllGlobals();
});

function snapshot(subtitles: SubtitleSnapshot, patch: Partial<SessionStateEvent> = {}): SessionStateEvent {
  return { ...original.session, status: { kind: "listening" }, isActive: true, isPaused: false,
    isTranslationPending: true, detectedLanguage: "ja", subtitles, ...patch };
}

async function mount(subtitles: SubtitleSnapshot, mode: SubtitleDisplayMode = "bilingual", settings: Partial<SettingsSnapshot> = {}) {
  await act(() => useStore.setState(state => ({ session: snapshot(subtitles), settings: { ...state.settings, ...settings, subtitleDisplayMode: mode } })));
  await act(() => root.render(<OverlayWindow />));
}

async function publish(subtitles: SubtitleSnapshot, patch: Partial<SessionStateEvent> = {}) {
  await act(() => useStore.setState(state => ({ session: shareUnchangedSubtitleHistory(state.session, snapshot(subtitles, patch)) })));
}

async function mode(value: SubtitleDisplayMode) {
  await act(() => useStore.setState(state => ({ settings: { ...state.settings, subtitleDisplayMode: value } })));
}

function visibleLanes(): string[] {
  return Array.from(host.querySelectorAll<HTMLElement>("[data-utterance-id] [aria-label]"))
    .filter(lane => !lane.hidden && lane.getAttribute("aria-hidden") !== "true" && lane.style.display !== "none")
    .map(lane => {
      expect(Number.parseFloat(lane.style.height)).toBeGreaterThan(0);
      return lane.firstElementChild?.textContent ?? "";
    });
}

it.each(["zh", "en", "ja"] as const)("uses one status line for a pending or failed control action in %s", async (language) => {
  setStoredUiLanguage(language);
  let reject!: (error: Error) => void;
  const togglePaused = vi.fn().mockImplementationOnce(() => new Promise<void>((_resolve, failed) => { reject = failed; }))
    .mockResolvedValue(undefined);
  useStore.setState({ togglePaused });
  await mount(empty);
  expect(host.querySelector('[data-testid="overlay-latency"]')).not.toBeNull();
  const pause = host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.pause}"]`)!;
  await act(async () => pause.click());
  expect(host.querySelector('[data-testid="overlay-latency"]')).toBeNull();
  expect(host.querySelector(".overlay-action-feedback")?.getAttribute("role")).toBe("status");
  await act(async () => reject(new Error("synthetic-action-failure")));
  expect(host.querySelector('[data-testid="overlay-latency"]')).toBeNull();
  expect(host.querySelector(".overlay-action-feedback")?.textContent).toBe(I18N.overlay.controlActionFailed);
  await act(async () => host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.pause}"]`)!.click());
  expect(togglePaused).toHaveBeenCalledTimes(2);
  expect(host.querySelector(".overlay-action-feedback")).toBeNull();
  expect(host.querySelector('[data-testid="overlay-latency"]')).not.toBeNull();
});

it.each(["Latest corrected English source.", "最新の修正された原文。", "최근에 수정된 원문입니다."])("switches modes using raw %s for original and one complete same-source preview for translated modes", async (source) => {
  const subtitles: SubtitleSnapshot = {
    ...empty, source: { text: source, isFinal: false, utteranceId: "raw-next" },
    translation: { text: "tiny SSE prefix", isFinal: false, utteranceId: "raw-next" },
    previewPair: { source: "Stable paired source.", translation: "完整配对译文。" },
  };
  await mount(subtitles, "original");
  expect(visibleLanes()).toEqual([source]);
  await mode("bilingual");
  expect(visibleLanes()).toEqual(["Stable paired source.", "完整配对译文。"]);
  await mode("translation");
  expect(visibleLanes()).toEqual(["完整配对译文。"]);
  await mode("original");
  expect(visibleLanes()).toEqual([source]);
  expect(host.querySelector("[data-utterance-id]")?.textContent).not.toContain("tiny SSE prefix");
  expect(useStore.getState().session.subtitles.history).toEqual([]);
});

it("keeps the first raw recognition visible in original and bilingual mode while suppressing unpaired SSE prefixes", async () => {
  const subtitles: SubtitleSnapshot = { ...empty,
    source: { text: "First original before MT.", isFinal: false },
    translation: { text: "unpaired tiny prefix", isFinal: false },
  };
  await mount(subtitles);
  expect(visibleLanes()).toEqual(["First original before MT."]);
  await mode("translation");
  expect(visibleLanes()).toEqual([]);
  await mode("original");
  expect(visibleLanes()).toEqual(["First original before MT."]);
  expect(host.querySelector("[data-utterance-id]")?.textContent).not.toContain("unpaired tiny prefix");
});

it.each(["alibabaCloud", "openAIRealtime"] as const)("keeps all three modes working for legacy %s snapshots without an atomic-preview field", async (provider) => {
  const subtitles: SubtitleSnapshot = {
    source: { text: "Legacy current original.", isFinal: false, utteranceId: "same-source" },
    translation: { text: "旧协议当前译文。", isFinal: false, utteranceId: "same-source" }, history: [],
  };
  await mount(subtitles, "bilingual", { profiles: [{ id: "legacy", name: "Legacy", provider, credentialState: "present" }], activeProfileId: "legacy" });
  expect(visibleLanes()).toEqual(["Legacy current original.", "旧协议当前译文。"]);
  await mode("original");
  expect(visibleLanes()).toEqual(["Legacy current original."]);
  await mode("translation");
  expect(visibleLanes()).toEqual(["旧协议当前译文。"]);
});

it("does not use an unrelated atomic-preview field for another provider's identified live streams", async () => {
  await mount({ ...empty, source: { text: "Identified provider original.", isFinal: false, utteranceId: "identified" },
    translation: { text: "相同句子身份的译文。", isFinal: false, utteranceId: "identified" },
    previewPair: { source: "Unrelated stale Alibaba original.", translation: "Unrelated stale Alibaba translation." } }, "bilingual", {
    profiles: [{ id: "identified", name: "OpenAI", provider: "openAIRealtime", credentialState: "present" }], activeProfileId: "identified",
  });
  expect(visibleLanes()).toEqual(["Identified provider original.", "相同句子身份的译文。"]);
  await mode("original");
  expect(visibleLanes()).toEqual(["Identified provider original."]);
  await mode("translation");
  expect(visibleLanes()).toEqual(["相同句子身份的译文。"]);
});

it("keeps a completed preview pair together during raw ASR corrections and replaces it atomically", async () => {
  const subtitles: SubtitleSnapshot = { ...empty,
    source: { text: "ASR revision 0", isFinal: false },
    translation: { text: "raw partial 0", isFinal: false },
    previewPair: { source: "Stable first source.", translation: "稳定的第一份译文。" },
  };
  await mount(subtitles);
  for (let revision = 1; revision <= 8; revision += 1) {
    await publish({ ...subtitles, source: { text: `ASR revision ${revision}`, isFinal: false }, translation: { text: `raw partial ${revision}`, isFinal: false } });
    expect(visibleLanes()).toEqual(["Stable first source.", "稳定的第一份译文。"]);
  }
  await mode("original");
  expect(visibleLanes()).toEqual(["ASR revision 8"]);
  await mode("bilingual");
  await publish({ ...subtitles, previewPair: { source: "Stable corrected source.", translation: "修正后的完整译文。" } });
  expect(visibleLanes()).toEqual(["Stable corrected source.", "修正后的完整译文。"]);
  expect(useStore.getState().session.subtitles.history).toEqual([]);
});

it("shows identical atomic bilingual lanes once while preserving repeated utterances and translation mode", async () => {
  const text = "Synthetic repeated line.";
  const first = { source: text, translation: text, createdAt: 10 };
  const second = { ...first, createdAt: 11 };
  const subtitles: SubtitleSnapshot = { ...empty,
    source: { text: text, isFinal: false, utteranceId: "synthetic-owner-B" },
    previewPair: { source: text, translation: text, utteranceId: "synthetic-owner-B" }, history: [first] };
  await mount(subtitles);
  expect(visibleLanes()).toEqual([text, text]);
  expect(host.querySelectorAll("[data-utterance-id]")).toHaveLength(2);
  expect(host.querySelector('[data-utterance-id="live-utterance-synthetic-owner-B"]')).not.toBeNull();
  await mode("translation");
  expect(visibleLanes()).toEqual([text, text]);
  await mode("bilingual");
  await publish({ ...empty, source: { text, isFinal: true }, translation: { text, isFinal: true },
    history: [first, second] }, { isTranslationPending: false });
  expect(visibleLanes()).toEqual([text, text]);
  expect(host.querySelector('[data-utterance-id^="live"]')).toBeNull();
  expect(host.querySelectorAll('[data-utterance-id^="history"]')).toHaveLength(2);
  expect(useStore.getState().session.subtitles.history).toEqual([first, second]);
});

it.each(modes)("retains the actual projected %s owner when an earlier final arrives while raw ASR is ahead", async displayMode => {
  const subtitles: SubtitleSnapshot = { ...empty,
    source: { text: "Latest synthetic raw source C.", isFinal: false, utteranceId: "synthetic-owner-C" },
    previewPair: { source: "Complete synthetic source B.", translation: "完整合成译文 B。", utteranceId: "synthetic-owner-B" } };
  await mount(subtitles, displayMode);
  const liveRow = host.querySelector<HTMLElement>('[data-utterance-id^="live"]')!;
  expect(liveRow.dataset.utteranceId).toBe(`live-utterance-synthetic-owner-${displayMode === "original" ? "C" : "B"}`);
  const text = liveRow.textContent;
  await publish({ ...subtitles, history: [confirmed] });
  expect(host.querySelector('[data-utterance-id^="live"]')).toBe(liveRow);
  expect(liveRow.textContent).toBe(text);
});

it.each(modes)("keeps the original and final history available without a duplicate live row after confirmation in %s mode", async (displayMode) => {
  await mount({ ...empty, source: { text: confirmed.source, isFinal: false },
    previewPair: { source: confirmed.source, translation: confirmed.translation } }, displayMode);
  await publish({ ...empty, source: { text: confirmed.source, isFinal: true },
    translation: { text: confirmed.translation, isFinal: true }, history: [confirmed] }, { isTranslationPending: false });
  const expected = displayMode === "original" ? [confirmed.source] : displayMode === "translation" ? [confirmed.translation] : [confirmed.source, confirmed.translation];
  expect(visibleLanes()).toEqual(expected);
  expect(host.querySelector('[data-utterance-id^="live"]')).toBeNull();
  expect(host.querySelectorAll("[data-utterance-id]")).toHaveLength(1);
  await mode("original");
  expect(visibleLanes()).toEqual([confirmed.source]);
  await mode("bilingual");
  expect(visibleLanes()).toEqual([confirmed.source, confirmed.translation]);
});

it("retains the previous confirmed original as well as its translation when the next sentence arrives", async () => {
  await mount({ ...empty, source: { text: "Next raw original.", isFinal: false },
    previewPair: { source: "Next paired original.", translation: "下一句完整译文。" }, history: [confirmed] });
  expect(visibleLanes()).toEqual([confirmed.source, confirmed.translation, "Next paired original.", "下一句完整译文。"]);
  await mode("original");
  expect(visibleLanes()).toEqual([confirmed.source, "Next raw original."]);
  await mode("translation");
  expect(visibleLanes()).toEqual([confirmed.translation, "下一句完整译文。"]);
});

it.each(["original", "bilingual"] as const)("keeps a confirmed source out of the live %s tail when the next MT request starts", async displayMode => {
  const subtitles = { ...empty, source: { text: confirmed.source, isFinal: true },
    translation: { text: confirmed.translation, isFinal: true }, history: [confirmed] };
  await mount(subtitles, displayMode);
  await publish(subtitles, { isTranslationPending: false });
  const historyRow = host.querySelector('[data-utterance-id="history-10"]');
  await publish(subtitles, { isTranslationPending: true });
  expect(host.querySelector('[data-utterance-id="history-10"]')).toBe(historyRow);
  expect(host.querySelectorAll("[data-utterance-id]")).toHaveLength(1);
  expect(host.querySelector('[data-utterance-id^="live"]')).toBeNull();

  // An actual new recognition cycle may repeat the exact same lyric. Its
  // replaceable source draft remains visible instead of being text-deduped.
  await publish({ ...subtitles, source: { text: confirmed.source, isFinal: false } });
  await act(async () => { await vi.advanceTimersByTimeAsync(180); });
  expect(host.querySelectorAll("[data-utterance-id]")).toHaveLength(2);
  expect(host.querySelector('[data-utterance-id^="live"]')?.textContent).toBe(confirmed.source);
});

it.each([
  { sourceLanguage: "zh", targetLanguage: "zh", detectedLanguage: "zh" },
  { sourceLanguage: "auto", targetLanguage: "zh", detectedLanguage: "zh" },
  { sourceLanguage: "ja", targetLanguage: "original", detectedLanguage: "ja" },
] as const)("uses raw recognition as the reading lane in every mode for $sourceLanguage → $targetLanguage", async ({ sourceLanguage, targetLanguage, detectedLanguage }) => {
  await mount({ ...empty, source: { text: "Same-language current recognition.", isFinal: false },
    previewPair: { source: "Unrelated prior original.", translation: "Unrelated prior translation." } }, "original", { sourceLanguage, targetLanguage });
  await act(() => useStore.setState(state => ({ session: { ...state.session, detectedLanguage } })));
  for (const displayMode of modes) {
    await mode(displayMode);
    expect(visibleLanes()).toEqual(["Same-language current recognition."]);
  }
});

it.each([
  { sourceLanguage: "zh", targetLanguage: "zh", detectedLanguage: "zh" },
  { sourceLanguage: "auto", targetLanguage: "zh", detectedLanguage: "zh" },
  { sourceLanguage: "ja", targetLanguage: "original", detectedLanguage: "ja" },
] as const)("does not show a lagging translation mirror over newer $sourceLanguage → $targetLanguage recognition", async ({ sourceLanguage, targetLanguage, detectedLanguage }) => {
  await mount({ ...empty, source: { text: "Latest corrected recognition.", isFinal: false },
    translation: { text: "Older mirrored recognition.", isFinal: false } }, "original", { sourceLanguage, targetLanguage });
  await act(() => useStore.setState(state => ({ session: { ...state.session, detectedLanguage } })));
  for (const displayMode of modes) {
    await mode(displayMode);
    expect(visibleLanes()).toEqual(["Latest corrected recognition."]);
  }
});

it("does not duplicate committed same-language recognition when a prior translation mirror remains", async () => {
  const sameLanguagePair = { source: "Confirmed same-language text.", translation: "Confirmed same-language text.", createdAt: 20 };
  await mount({ ...empty, source: { text: sameLanguagePair.source, isFinal: true },
    translation: { text: "An older unconfirmed mirror.", isFinal: false }, history: [sameLanguagePair] }, "original", { sourceLanguage: "zh", targetLanguage: "zh" });
  await act(() => useStore.setState(state => ({ session: { ...state.session, detectedLanguage: "zh", isTranslationPending: false } })));
  for (const displayMode of modes) {
    await mode(displayMode);
    expect(visibleLanes()).toEqual([sameLanguagePair.source]);
    expect(host.querySelector('[data-utterance-id^="live"]')).toBeNull();
  }
});

it("keeps clear reachable while only a complete preview pair remains", async () => {
  await mount({ ...empty, previewPair: { source: "Complete source without raw tail.", translation: "原始尾句为空时的完整译文。" } });
  expect(visibleLanes()).toEqual(["Complete source without raw tail.", "原始尾句为空时的完整译文。"]);
  const clear = host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.clearSubtitles}"]`);
  expect(clear).not.toBeNull();
  await act(async () => clear!.click());
  expect(visibleLanes()).toEqual([]);
});

it("keeps paused text readable across modes and resumes with a new complete pair", async () => {
  await mount({ ...empty, source: { text: "Current raw original.", isFinal: false },
    previewPair: { source: "Frozen paired original.", translation: "暂停前完整译文。" } });
  await act(async () => useStore.getState().togglePaused());
  expect(useStore.getState().session.isPaused).toBe(true);
  expect(visibleLanes()).toEqual(["Frozen paired original.", "暂停前完整译文。"]);
  await mode("original");
  expect(visibleLanes()).toEqual(["Current raw original."]);
  await mode("translation");
  expect(visibleLanes()).toEqual(["暂停前完整译文。"]);
  await act(async () => useStore.getState().togglePaused());
  await publish({ ...empty, source: { text: "Resumed raw original.", isFinal: false },
    previewPair: { source: "Resumed paired original.", translation: "继续后的完整译文。" } });
  await mode("bilingual");
  expect(visibleLanes()).toEqual(["Resumed paired original.", "继续后的完整译文。"]);
});

it("clears all three modes through the real clear action without resurrecting a pending draft", async () => {
  await mount({ ...empty, source: { text: "Visible original before clear.", isFinal: false },
    previewPair: { source: "Visible paired original before clear.", translation: "清空前的译文。" }, history: [confirmed] });
  const clear = host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.clearSubtitles}"]`)!;
  expect(clear).not.toBeNull();
  await act(async () => clear.click());
  for (const displayMode of modes) {
    await mode(displayMode);
    expect(visibleLanes()).toEqual([]);
  }
  await act(async () => { await vi.advanceTimersByTimeAsync(2_000); });
  expect(visibleLanes()).toEqual([]);
  expect(useStore.getState().session.subtitles.history).toEqual([]);
});

it("drops the previous generation's live pair on reconnect while preserving confirmed history in all modes", async () => {
  await mount({ ...empty, source: { text: "Old raw original.", isFinal: false },
    previewPair: { source: "Old live paired original.", translation: "旧连接尚未确认的译文。" }, history: [confirmed] });
  await publish({ ...empty, history: [confirmed] }, { status: { kind: "connecting" }, detectedLanguage: null, isTranslationPending: false });
  for (const displayMode of modes) {
    await mode(displayMode);
    expect(visibleLanes()).toEqual(displayMode === "original" ? [confirmed.source] : displayMode === "translation" ? [confirmed.translation] : [confirmed.source, confirmed.translation]);
  }
  await publish({ ...empty, history: [confirmed], source: { text: "New generation original.", isFinal: false, utteranceId: "new-generation" },
    translation: { text: "new unpaired prefix", isFinal: false, utteranceId: "new-generation" } });
  expect(visibleLanes()).toEqual([confirmed.source, confirmed.translation, "New generation original."]);
  await mode("translation");
  expect(visibleLanes()).toEqual([confirmed.translation]);
  await mode("original");
  expect(visibleLanes()).toEqual([confirmed.source, "New generation original."]);
  await act(async () => { await vi.advanceTimersByTimeAsync(2_000); });
  expect(visibleLanes()).toEqual([confirmed.source, "New generation original."]);
});

function dualSnapshot(): SubtitleSnapshot {
  const track = (audioSource: "system" | "microphone", text: string, translation: string) => ({
    ...empty, audioSource, detectedLanguage: "en", isTranslationPending: true, isTranslationTimedOut: false,
    source: { text, isFinal: false, utteranceId: "shared-provider-id" },
    previewPair: { source: text, translation, utteranceId: "shared-provider-id" },
  });
  return { ...empty, tracks: [track("system", "Synthetic system phrase.", "系统声音合成译文。"),
    track("microphone", "Synthetic microphone phrase.", "麦克风合成译文。")], history: [] };
}

it.each(modes)("keeps both source tails and labels independent in %s mode, including identical provider ids", async displayMode => {
  const dual = dualSnapshot();
  await mount(dual, displayMode, { audioInput: "both" });
  expect([...host.querySelectorAll(".subtitle-audio-source")].map(label => label.textContent))
    .toEqual([I18N.settings.audioInputSystem, I18N.settings.audioInputMicrophone]);
  const ids = [...host.querySelectorAll("[data-utterance-id]")].map(row => row.getAttribute("data-utterance-id"));
  expect(new Set(ids).size).toBe(2);
  const expected = displayMode === "original" ? ["Synthetic system phrase.", "Synthetic microphone phrase."]
    : displayMode === "translation" ? ["系统声音合成译文。", "麦克风合成译文。"]
    : ["Synthetic system phrase.", "系统声音合成译文。", "Synthetic microphone phrase.", "麦克风合成译文。"];
  expect(visibleLanes()).toEqual(expected);
  const microphoneRow = host.querySelector('[data-utterance-id^="microphone:"]');
  const revised = { ...dual, tracks: dual.tracks!.map(track => track.audioSource === "system" ? {
    ...track, source: { text: "Revised synthetic system.", isFinal: false, utteranceId: "next" },
    previewPair: { source: "Revised synthetic system.", translation: "修改后的系统声音。", utteranceId: "next" },
  } : track) };
  await publish(revised); await act(async () => vi.advanceTimersByTime(800));
  expect(host.querySelector('[data-utterance-id^="microphone:"]')).toBe(microphoneRow);
  expect(visibleLanes()).toContain(displayMode === "original" ? "Synthetic microphone phrase." : "麦克风合成译文。");
});

it("preserves source identity for simultaneous finals and a lagging other-source preview", async () => {
  const dual = dualSnapshot();
  const sameTime = [
    { audioSource: "system" as const, source: "System confirmed.", translation: "系统已确认。", createdAt: 40 },
    { audioSource: "microphone" as const, source: "Microphone confirmed.", translation: "麦克风已确认。", createdAt: 40 },
  ];
  await mount({ ...dual, history: sameTime }, "bilingual", { audioInput: "both" });
  const rows = [...host.querySelectorAll('[data-utterance-id]')];
  expect(rows).toHaveLength(4);
  expect(new Set(rows.map(row => row.getAttribute("data-utterance-id"))).size).toBe(4);
  expect(rows[0].textContent).toContain(I18N.settings.audioInputSystem);
  expect(rows[1].textContent).toContain(I18N.settings.audioInputMicrophone);
  await publish({ ...empty, history: sameTime, tracks: dual.tracks!.map(track => ({ ...track,
    source: { text: "", isFinal: false }, previewPair: null, isTranslationPending: false })) }, { status: { kind: "connecting" } });
  expect(host.querySelectorAll('[data-utterance-id]')).toHaveLength(2);
  expect(visibleLanes()).not.toContain("麦克风合成译文。");
});

it("uses each source's detected language when the other source does not need translation", async () => {
  const dual = dualSnapshot();
  dual.tracks![0].detectedLanguage = "zh";
  await mount(dual, "translation", { audioInput: "both" });
  expect(visibleLanes()).toEqual(["Synthetic system phrase.", "麦克风合成译文。"]);
});

it.each(["system", "microphone"] as const)("keeps both history labels but only the %s live tail after disabling the other input", async enabled => {
  const dual = dualSnapshot();
  const history = [
    { audioSource: "system" as const, source: "System confirmed.", translation: "系统已确认。", createdAt: 40 },
    { audioSource: "microphone" as const, source: "Microphone confirmed.", translation: "麦克风已确认。", createdAt: 41 },
  ];
  const remaining = dual.tracks!.find(track => track.audioSource === enabled)!;
  await mount({ ...remaining, history, tracks: [remaining] }, "bilingual", { audioInput: enabled });
  const rows = [...host.querySelectorAll('[data-utterance-id]')];
  expect(rows).toHaveLength(3);
  expect(rows[0].textContent).toContain(I18N.settings.audioInputSystem);
  expect(rows[1].textContent).toContain(I18N.settings.audioInputMicrophone);
  expect(rows[2].getAttribute("data-utterance-id")).toMatch(new RegExp(`^${enabled}:`));
  expect(visibleLanes()).not.toContain(enabled === "system" ? "麦克风合成译文。" : "系统声音合成译文。");
});

it("applies background opacity to expanded and collapsed cards and restores it after immersive mode", async () => {
  await mount(empty, "translation", { subtitleBackgroundOpacity: 35 });
  const backgrounds = () => [...host.querySelectorAll<HTMLElement>("[style]")].map(node => node.style.background);
  expect(backgrounds()).toContain("rgba(0, 0, 0, 0.35)");
  await act(() => useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: true } })));
  expect(backgrounds()).toContain("rgba(0, 0, 0, 0.35)");
  await act(() => useStore.setState(state => ({ session: { ...state.session, isOverlayCollapsed: false }, settings: { ...state.settings, subtitleBlendsWithBackground: true } })));
  expect(backgrounds()).not.toContain("rgba(0, 0, 0, 0.35)");
  await act(() => useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: false } })));
  expect(backgrounds()).toContain("rgba(0, 0, 0, 0.35)");
});
