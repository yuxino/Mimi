// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../../lib/store";
import type { SettingsSnapshot, SubtitleSnapshot } from "../../lib/types";
import { formatGeminiTranscriptForDisplay } from "./geminiTranscriptDisplay";
import { useSubtitleTail } from "./useSubtitleTail";

vi.mock("./geminiTranscriptDisplay", async original => {
  const actual = await original<typeof import("./geminiTranscriptDisplay")>();
  return { ...actual, formatGeminiTranscriptForDisplay: vi.fn(actual.formatGeminiTranscriptForDisplay) };
});

const settings: SettingsSnapshot = { ...useStore.getState().settings, sourceLanguage: "en", targetLanguage: "zh", subtitleDisplayMode: "bilingual" };
const initial: SubtitleSnapshot = {
  source: { text: "Synthetic original sentence. Another fragment", isFinal: false, utteranceId: "same" },
  translation: { text: `${"合成".repeat(12)}。下一句`, isFinal: false, utteranceId: "same" },
  history: [],
};
let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(formatGeminiTranscriptForDisplay).mockClear();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(() => root.unmount()); host.remove();
  vi.useRealTimers(); vi.unstubAllGlobals();
  vi.mocked(formatGeminiTranscriptForDisplay).mockClear();
});

function Tail({ subtitles, streamingProvider = true, running = true, appearance = settings }: {
  subtitles: SubtitleSnapshot; streamingProvider?: boolean; running?: boolean; appearance?: SettingsSnapshot;
}) {
  const tail = useSubtitleTail(subtitles, appearance, { detectedLanguage: "en", isTranslationPending: false, isTranslationTimedOut: false },
    running, false, "primary", streamingProvider);
  return <><span data-lane="source">{tail.source}</span><span data-lane="translation">{tail.translation}</span></>;
}
async function render(subtitles = initial, options: { streamingProvider?: boolean; running?: boolean; appearance?: SettingsSnapshot } = {}) {
  await act(() => root.render(<Tail subtitles={subtitles} {...options} />));
}
function text(lane: "source" | "translation") { return host.querySelector(`[data-lane="${lane}"]`)!.textContent; }

it("reuses formatting across fresh snapshots and unrelated status/appearance renders", async () => {
  await render();
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledTimes(2);
  for (let index = 0; index < 20; index++) {
    const snapshot = { ...initial, source: { ...initial.source }, translation: { ...initial.translation }, history: [] };
    await render(snapshot, { running: index % 2 === 0, appearance: { ...settings, fontSize: 16 + index } });
  }
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledTimes(2);
  expect(text("source")).toBe("Synthetic original sentence.\nAnother fragment");
  expect(text("translation")).toBe(`${"合成".repeat(12)}。\n下一句`);
});

it("formats only the changed lane and preserves its existing streaming deadlines", async () => {
  await render();
  vi.mocked(formatGeminiTranscriptForDisplay).mockClear();
  const updatedSource = { ...initial, source: { ...initial.source, text: "Updated synthetic original sentence. Another fragment" } };
  await render(updatedSource);
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledExactlyOnceWith(updatedSource.source.text);
  expect(text("source")).toBe("Synthetic original sentence.\nAnother fragment");
  await act(async () => { await vi.advanceTimersByTimeAsync(180); });
  expect(text("source")).toBe("Updated synthetic original sentence.\nAnother fragment");
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledTimes(1);

  vi.mocked(formatGeminiTranscriptForDisplay).mockClear();
  const updatedTranslation = { ...updatedSource, translation: { ...initial.translation, text: `${"新的".repeat(12)}。下一句` } };
  await render(updatedTranslation);
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledExactlyOnceWith(updatedTranslation.translation.text);
  expect(text("translation")).toBe(`${"合成".repeat(12)}。\n下一句`);
  await act(async () => { await vi.advanceTimersByTimeAsync(100); });
  expect(text("translation")).toBe(`${"新的".repeat(12)}。\n下一句`);
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledTimes(1);
});

it("updates provider formatting immediately without formatting nonstreaming text", async () => {
  await render(initial, { streamingProvider: false });
  expect(formatGeminiTranscriptForDisplay).not.toHaveBeenCalled();
  expect(text("source")).toBe(initial.source.text);
  await render();
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledTimes(2);
  expect(text("source")).toBe("Synthetic original sentence.\nAnother fragment");
  await render(initial, { streamingProvider: false });
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledTimes(2);
  expect(text("source")).toBe(initial.source.text);
  expect(text("translation")).toBe(initial.translation.text);
});

it("still confirms and clears a lane immediately when its formatted text is cached", async () => {
  await render();
  const revised = { ...initial, source: { ...initial.source, text: "A revised synthetic sentence. Final fragment" } };
  await render(revised);
  expect(text("source")).toBe("Synthetic original sentence.\nAnother fragment");
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledTimes(3);
  await render({ ...revised, source: { ...revised.source, isFinal: true }, translation: { ...initial.translation, isFinal: true } });
  expect(formatGeminiTranscriptForDisplay).toHaveBeenCalledTimes(3);
  expect(text("source")).toBe("A revised synthetic sentence.\nFinal fragment");
  await render({ ...initial, source: { text: "", isFinal: false }, translation: { text: "", isFinal: false } });
  expect(text("source")).toBe("");
  expect(text("translation")).toBe("");
});
