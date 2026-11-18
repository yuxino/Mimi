// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { subtitleStreamKey, useStableText } from "./animation";

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

interface PairProps {
  id?: string;
  committedAt?: number;
  source?: string;
  translation?: string;
  final?: boolean;
}
function Pair({ id, committedAt = 1, source = "Old source", translation = "旧译文", final = false }: PairProps) {
  const sourceText = useStableText(source, final ? 0 : 180, 750, subtitleStreamKey("bilingual", "source", id, committedAt));
  const translationText = useStableText(translation, final ? 0 : 400, 1500, subtitleStreamKey("bilingual", "translation", id, committedAt));
  return <><span data-lane="source">{sourceText}</span><span data-lane="translation">{translationText}</span></>;
}
async function render(props: PairProps) {
  await act(async () => root.render(<Pair {...props} />));
}
function texts() {
  return Array.from(host.children).map(lane => lane.textContent);
}

it("resets both lanes together when a coalesced snapshot replaces their utterance stamps", async () => {
  await render({ id: "old" });
  await render({ id: "new", source: "New source", translation: "新译文" });
  expect(texts()).toEqual(["New source", "新译文"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(200); });
  // The shorter source timer must not leave a new original beside the old
  // translation until the independent 400ms translation timer completes.
  expect(texts()).toEqual(["New source", "新译文"]);
});

it("seeds a new utterance once and stabilizes its following draft revisions immediately", async () => {
  await render({ id: "old" });
  await render({ id: "new", source: "New source", translation: "新译文" });
  await act(async () => { await vi.advanceTimersByTimeAsync(40); });
  await render({ id: "new", source: "New source grows", translation: "新译文增加" });
  expect(texts()).toEqual(["New source", "新译文"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(180); });
  expect(texts()).toEqual(["New source grows", "新译文"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(220); });
  expect(texts()).toEqual(["New source grows", "新译文增加"]);
});

it("starts a fresh maximum-wait window after replacing an utterance with pending timers", async () => {
  await render({ id: "old" });
  await render({ id: "old", source: "Old pending source", translation: "旧句处理中" });
  await act(async () => { await vi.advanceTimersByTimeAsync(100); });
  await render({ id: "new", source: "New seed", translation: "新句起点" });
  for (let revision = 1; revision <= 6; revision += 1) {
    await act(async () => { await vi.advanceTimersByTimeAsync(100); });
    await render({ id: "new", source: `New revision ${revision}`, translation: `新句修订 ${revision}` });
  }
  // The old source's force timer would fire at 750 ms, ahead of this
  // utterance's own 950 ms deadline. It cannot publish the new raw draft.
  await act(async () => { await vi.advanceTimersByTimeAsync(50); });
  expect(texts()).toEqual(["New seed", "新句起点"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(130); });
  expect(texts()).toEqual(["New revision 6", "新句起点"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(220); });
  expect(texts()).toEqual(["New revision 6", "新句修订 6"]);
});

it("resets unstamped previews when the previous sentence is confirmed in the same snapshot", async () => {
  await render({ committedAt: 1 });
  await render({ committedAt: 2, source: "Next original", translation: "下一句译文" });
  expect(texts()).toEqual(["Next original", "下一句译文"]);
});

it("keeps the independent settling delays for updates within one stamped utterance", async () => {
  await render({ id: "same" });
  await render({ id: "same", committedAt: 2, source: "Same source grows", translation: "同句译文增加" });
  expect(texts()).toEqual(["Old source", "旧译文"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(180); });
  expect(texts()).toEqual(["Same source grows", "旧译文"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(220); });
  expect(texts()).toEqual(["Same source grows", "同句译文增加"]);
});

it("still clears removed previews and accepts final text immediately", async () => {
  await render({ id: "same" });
  await render({ id: "same", source: "Confirmed source", translation: "最终译文", final: true });
  expect(texts()).toEqual(["Confirmed source", "最终译文"]);
  await render({ id: "same", source: "", translation: "" });
  expect(texts()).toEqual(["", ""]);
});

it("never flashes the cached pre-final draft when another update follows confirmation", async () => {
  await render({ id: "same" });
  await render({ id: "same", source: "Confirmed source", translation: "最终译文", final: true });
  await render({ id: "same", source: "Subsequent source", translation: "后续译文" });
  expect(texts()).toEqual(["Confirmed source", "最终译文"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(400); });
  expect(texts()).toEqual(["Subsequent source", "后续译文"]);
});

it("does not resurrect a cleared draft while the next preview settles", async () => {
  await render({ id: "same" });
  await render({ id: "same", source: "", translation: "" });
  await render({ id: "same", source: "Resumed source", translation: "新的译文" });
  expect(texts()).toEqual(["", ""]);
  await act(async () => { await vi.advanceTimersByTimeAsync(400); });
  expect(texts()).toEqual(["Resumed source", "新的译文"]);
});

it("keeps bounded maximum-wait updates under continuous drafts without losing their text", async () => {
  await render({ id: "continuous", source: "Source seed", translation: "译文起点" });
  await render({ id: "continuous", source: "Source revision 0", translation: "译文修订 0" });
  for (let revision = 1; revision <= 14; revision += 1) {
    await act(async () => { await vi.advanceTimersByTimeAsync(100); });
    await render({ id: "continuous", source: `Source revision ${revision}`, translation: `译文修订 ${revision}` });
  }
  expect(texts()).toEqual(["Source revision 7", "译文起点"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(100); });
  expect(texts()).toEqual(["Source revision 7", "译文修订 14"]);
  await act(async () => { await vi.advanceTimersByTimeAsync(80); });
  expect(texts()).toEqual(["Source revision 14", "译文修订 14"]);
});

it("cancels pending settle and maximum-wait work on unmount", async () => {
  await render({ id: "same" });
  await render({ id: "same", source: "Pending source", translation: "等待中的译文" });
  expect(vi.getTimerCount()).toBeGreaterThan(0);
  await act(() => root.unmount());
  expect(vi.getTimerCount()).toBe(0);
});
