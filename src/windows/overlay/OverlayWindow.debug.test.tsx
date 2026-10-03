// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../../lib/store";
import type { SessionStateEvent } from "../../lib/types";
import { OverlayWindow } from "./OverlayWindow";

const observations = vi.hoisted(() => ({ committed: vi.fn(), timeline: vi.fn() }));
vi.mock("../../lib/developmentTrace", async original => ({
  ...await original<typeof import("../../lib/developmentTrace")>(),
  observeOverlayCommitted: observations.committed,
}));
vi.mock("./Timeline", async () => {
  const { memo, useLayoutEffect, useRef } = await import("react");
  return { Timeline: memo((props: { blocks: { source: string | null; translation: string | null }[] }) => {
    const timeline = useRef<HTMLDivElement>(null);
    useLayoutEffect(() => { timeline.current!.scrollTop = 25; });
    observations.timeline();
    return <div ref={timeline} className="overlay-timeline">{props.blocks.map(block => block.translation ?? block.source).join("\n")}</div>;
  }) };
});
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => null }));

const original = useStore.getState();
const session: SessionStateEvent = {
  ...original.session, debugSnapshotId: 10, status: { kind: "listening" },
  isActive: true, detectedLanguage: "en",
  subtitles: {
    source: { text: "Synthetic original", isFinal: true },
    translation: { text: "First text", isFinal: false }, history: [],
  },
};
let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.useFakeTimers();
  observations.committed.mockClear();
  observations.timeline.mockClear();
  useStore.setState({ ...original, session, settings: { ...original.settings,
    sourceLanguage: "en", targetLanguage: "zh", subtitleDisplayMode: "translation",
    pulseAnimation: false, subtitleAnimation: false,
  } }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  useStore.setState(original, true);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("observes raw draft commits without repainting Timeline and identifies a later same-length stable replacement", async () => {
  await act(async () => root.render(<OverlayWindow />));
  expect(observations.timeline).toHaveBeenCalledOnce();
  expect(observations.committed.mock.lastCall?.[1]).toMatchObject({
    projectionRevision: 1, selectedTranslationCharacters: 10, stableTranslationCharacters: 10,
  });
  const updated = { ...session, debugSnapshotId: 11, subtitles: { ...session.subtitles,
    translation: { text: "Other text", isFinal: false },
  } };
  await act(async () => useStore.setState({ session: updated }));
  expect(observations.timeline).toHaveBeenCalledOnce();
  expect(host.textContent).toContain("First text");
  expect(observations.committed.mock.lastCall?.[0].debugSnapshotId).toBe(11);
  expect(observations.committed.mock.lastCall?.[1]).toMatchObject({ projectionRevision: 1 });
  await act(async () => vi.advanceTimersByTimeAsync(400));
  expect(observations.timeline).toHaveBeenCalledTimes(2);
  expect(host.textContent).toContain("Other text");
  expect(observations.committed.mock.lastCall?.[1]).toMatchObject({
    projectionRevision: 2, selectedTranslationCharacters: 10, stableTranslationCharacters: 10,
  });
});

it("measures the committed viewport after Timeline has applied its layout scroll", async () => {
  await act(async () => root.render(<OverlayWindow />));
  expect(observations.committed.mock.lastCall?.[1]).toMatchObject({ scrollTop: 25 });
});

it("records a collapsed DOM projection without changing its selected or stable text, then stops observing untagged snapshots", async () => {
  await act(async () => root.render(<OverlayWindow />));
  await act(async () => useStore.setState({ session: { ...session, debugSnapshotId: 12, isOverlayCollapsed: true } }));
  expect(observations.committed.mock.lastCall?.[1]).toMatchObject({
    projection: "collapsed", visibleCharacters: 0, visibleBlocks: 0,
    selectedTranslationCharacters: 10, stableTranslationCharacters: 10,
  });
  observations.committed.mockClear();
  await act(async () => useStore.setState({ session: { ...session, debugSnapshotId: null } }));
  await act(async () => vi.advanceTimersByTimeAsync(500));
  expect(observations.committed).not.toHaveBeenCalled();
});
