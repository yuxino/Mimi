// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../../lib/store";
import { shareUnchangedSubtitleHistory } from "../../lib/session/sessionSnapshot";
import type { SessionStateEvent } from "../../lib/types";
import { OverlayWindow } from "./OverlayWindow";

const timelineRender = vi.hoisted(() => vi.fn());
vi.mock("./Timeline", async () => {
  const { memo } = await import("react");
  return { Timeline: memo((props: { blocks: { source: string | null; translation: string | null }[] }) => {
    timelineRender();
    return <div data-testid="timeline">{props.blocks.map((block) => `${block.source ?? ""} ${block.translation ?? ""}`).join("\n")}</div>;
  }) };
});
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => null }));

const original = useStore.getState();
const session: SessionStateEvent = {
  ...original.session, status: { kind: "listening" }, isActive: true,
  detectedLanguage: "en", isTranslationPending: true,
  subtitles: {
    source: { text: "Synthetic live source", isFinal: false },
    translation: { text: "Synthetic settled translation", isFinal: false },
    history: [
      { source: "Synthetic confirmed first", translation: "Synthetic first translation", createdAt: 1 },
      { source: "Synthetic confirmed second", translation: "Synthetic second translation", createdAt: 2 },
    ],
  },
};
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.useFakeTimers();
  timelineRender.mockClear();
  useStore.setState({ ...original, session, settings: { ...original.settings, sourceLanguage: "en", targetLanguage: "zh", subtitleDisplayMode: "translation", pulseAnimation: false, subtitleAnimation: false } }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); useStore.setState(original, true);
  vi.useRealTimers(); vi.unstubAllGlobals();
});

async function publish(snapshot: SessionStateEvent) {
  await act(async () => useStore.setState((state) => ({ session: shareUnchangedSubtitleHistory(state.session, snapshot) })));
}

it("keeps Timeline unchanged during draft churn and updates for settled text and real history edits", async () => {
  await act(async () => root.render(<OverlayWindow />));
  expect(timelineRender).toHaveBeenCalledOnce();
  for (let index = 1; index <= 50; index++) {
    const incoming = JSON.parse(JSON.stringify(session)) as SessionStateEvent;
    incoming.subtitles.translation.text = `Synthetic translation draft ${index}`;
    await publish(incoming);
  }
  expect(timelineRender).toHaveBeenCalledOnce();
  expect(host.textContent).not.toContain("Synthetic translation draft 50");
  await act(async () => vi.advanceTimersByTime(400));
  expect(timelineRender).toHaveBeenCalledTimes(2);
  expect(host.textContent).toContain("Synthetic translation draft 50");

  const revised = JSON.parse(JSON.stringify(useStore.getState().session)) as SessionStateEvent;
  revised.subtitles.history[0].translation = "Synthetic corrected first translation";
  await publish(revised);
  expect(timelineRender).toHaveBeenCalledTimes(3);
  expect(host.textContent).toContain("Synthetic corrected first translation");

  const trimmed = { ...revised, subtitles: { ...revised.subtitles, history: revised.subtitles.history.slice(1) } };
  await publish(trimmed);
  expect(timelineRender).toHaveBeenCalledTimes(4);
  expect(host.textContent).not.toContain("Synthetic corrected first translation");

  await publish({ ...trimmed, subtitles: { ...trimmed.subtitles, history: [] } });
  expect(timelineRender).toHaveBeenCalledTimes(5);
  expect(host.textContent).not.toContain("Synthetic second translation");
});

it("stabilizes both input drafts without rebuilding their shared history on every IPC event", async () => {
  const tracks = (["system", "microphone"] as const).map(audioSource => ({
    audioSource, source: { text: `Synthetic ${audioSource}`, isFinal: false },
    translation: { text: `Synthetic ${audioSource} settled`, isFinal: false },
    history: session.subtitles.history.map(pair => ({ ...pair, audioSource })),
    detectedLanguage: "en", isTranslationPending: true, isTranslationTimedOut: false,
  }));
  const dual = { ...session, subtitles: { ...session.subtitles, tracks } };
  await publish(dual);
  await act(async () => root.render(<OverlayWindow />));
  expect(timelineRender).toHaveBeenCalledOnce();
  for (let index = 1; index <= 50; index++) {
    const incoming = JSON.parse(JSON.stringify(dual)) as SessionStateEvent;
    incoming.subtitles.tracks![index % 2].translation.text = `Synthetic updated draft ${index}`;
    await publish(incoming);
  }
  expect(timelineRender).toHaveBeenCalledOnce();
  await act(async () => vi.advanceTimersByTime(400));
  expect(timelineRender).toHaveBeenCalledTimes(2);
  expect(host.textContent).toContain("Synthetic updated draft 50");
});
