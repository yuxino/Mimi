// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  committedOverlayMeasurements, DevelopmentTraceQueue, observeOverlayCommitted,
  observeSessionStoreApplied, observeSessionWireReceived, sessionDebugCounts,
  subtitleCharacterCount, type FrontendDebugObservation,
} from "./developmentTrace";
import type { SessionStateEvent } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
const native = vi.hoisted(() => ({ label: "overlay", flush: null as ((event: { payload: { nonce: number } }) => Promise<void>) | null }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ label: native.label }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (_name, handler) => {
  native.flush = handler;
  return () => {};
}) }));

const session: SessionStateEvent = {
  status: { kind: "idle" }, isActive: false, isPaused: false, isOverlayCollapsed: false,
  detectedLanguage: null, isTranslationPending: false, isTranslationTimedOut: false,
  subtitles: { source: { text: "Synthetic 😀 source", isFinal: false },
    translation: { text: "合成译文", isFinal: false }, history: [] },
};
const observation = (snapshotId: number): FrontendDebugObservation => ({
  stage: "wireReceived", window: "overlay", snapshotId,
  sourceCharacters: 10, translationCharacters: 5, historyEntries: 1,
});
const queues: DevelopmentTraceQueue[] = [];
function queue(send: (observations: FrontendDebugObservation[]) => Promise<unknown>) {
  const value = new DevelopmentTraceQueue(send);
  queues.push(value);
  return value;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(invoke).mockReset().mockResolvedValue(undefined);
  native.label = "overlay";
  observeSessionWireReceived(session);
});
afterEach(() => {
  for (const value of queues.splice(0)) value.setEnabled(false);
  observeSessionWireReceived(session);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("bounded frontend observation transport", () => {
  it("stays disabled until enabled, and discards queued data immediately on disable", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const value = queue(send);
    value.record(observation(1));
    await vi.advanceTimersByTimeAsync(100);
    expect(send).not.toHaveBeenCalled();
    value.setEnabled(true);
    value.record(observation(2));
    value.setEnabled(false);
    await vi.advanceTimersByTimeAsync(100);
    expect(send).not.toHaveBeenCalled();
    value.setEnabled(true);
    value.record(observation(3));
    await vi.advanceTimersByTimeAsync(50);
    expect(send).toHaveBeenCalledExactlyOnceWith([observation(3)]);
  });

  it("sends ordered asynchronous batches of at most 16 observations", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const value = queue(send);
    value.setEnabled(true);
    for (let id = 1; id <= 35; id += 1) value.record(observation(id));
    expect(send).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(150);
    expect(send.mock.calls.map(([batch]) => batch.length)).toEqual([16, 16, 3]);
    expect(send.mock.calls.flatMap(([batch]) => batch.map((item: FrontendDebugObservation) => item.snapshotId)))
      .toEqual(Array.from({ length: 35 }, (_, index) => index + 1));
  });

  it("keeps only 128 queued samples and reports missing samples in the next batch", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const value = queue(send);
    value.setEnabled(true);
    for (let id = 1; id <= 200; id += 1) value.record(observation(id));
    await vi.advanceTimersByTimeAsync(400);
    const samples = send.mock.calls.flatMap(([batch]) => batch) as FrontendDebugObservation[];
    expect(samples).toHaveLength(128);
    expect(samples[0]).toEqual({ ...observation(73), clientDropped: 72 });
    expect(samples.at(-1)?.snapshotId).toBe(200);
    expect(samples.slice(1).every(sample => sample.clientDropped === undefined)).toBe(true);
  });

  it("allows only one IPC request in flight while maintaining the bounded backlog", async () => {
    let finish!: () => void;
    const send = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }))
      .mockResolvedValue(undefined);
    const value = queue(send);
    value.setEnabled(true);
    for (let id = 1; id <= 16; id += 1) value.record(observation(id));
    await vi.advanceTimersByTimeAsync(50);
    for (let id = 17; id <= 216; id += 1) value.record(observation(id));
    await vi.advanceTimersByTimeAsync(500);
    expect(send).toHaveBeenCalledOnce();
    finish();
    await vi.advanceTimersByTimeAsync(50);
    expect(send).toHaveBeenCalledTimes(2);
    expect(send.mock.calls[1][0][0]).toEqual({ ...observation(89), clientDropped: 72 });
  });

  it("reports failed delivery without retrying or blocking subsequent observations", async () => {
    const send = vi.fn().mockRejectedValueOnce(new Error("synthetic failure")).mockResolvedValue(undefined);
    const value = queue(send);
    value.setEnabled(true);
    for (let id = 1; id <= 20; id += 1) value.record(observation(id));
    await vi.advanceTimersByTimeAsync(100);
    expect(send).toHaveBeenCalledTimes(2);
    expect(send.mock.calls[1][0][0]).toEqual({ ...observation(17), clientDropped: 16 });
  });

  it("flushes all queued boundary samples immediately without waiting for the batch timer", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const value = queue(send);
    value.setEnabled(true);
    for (let id = 1; id <= 35; id += 1) value.record(observation(id));
    expect(await value.flushNow()).toBe(true);
    expect(send.mock.calls.map(([batch]) => batch.length)).toEqual([16, 16, 3]);
    await vi.advanceTimersByTimeAsync(100);
    expect(send).toHaveBeenCalledTimes(3);
  });

  it("waits for an active delivery before flushing and leaves a failure boundary incomplete", async () => {
    let fail!: (reason: Error) => void;
    const send = vi.fn().mockImplementationOnce(() => new Promise<void>((_resolve, reject) => { fail = reject; }))
      .mockResolvedValue(undefined);
    const value = queue(send);
    value.setEnabled(true);
    value.record(observation(1));
    await vi.advanceTimersByTimeAsync(50);
    value.record(observation(2));
    const flush = value.flushNow();
    fail(new Error("synthetic delivery failure"));
    expect(await flush).toBe(false);
    expect(send.mock.calls[1][0]).toEqual([{ ...observation(2), clientDropped: 1 }]);
  });

  it("keeps a failed final batch incomplete even when no later observation can carry its drop count", async () => {
    const send = vi.fn().mockRejectedValue(new Error("synthetic delivery failure"));
    const value = queue(send);
    value.setEnabled(true);
    value.record(observation(1));
    expect(await value.flushNow()).toBe(false);
    expect(await value.flushNow()).toBe(false);
    expect(send).toHaveBeenCalledOnce();
  });

  it("copies only controlled numeric fields and enum values, never arbitrary content", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const value = queue(send);
    value.setEnabled(true);
    value.record({ ...observation(1), sourceCharacters: -1, visibleCharacters: Number.NaN,
      scrollTop: 10.7, text: "Synthetic text that must not leave the projection" } as FrontendDebugObservation);
    value.record({ ...observation(2), snapshotId: Number.MAX_SAFE_INTEGER + 1 });
    await vi.advanceTimersByTimeAsync(50);
    expect(send).toHaveBeenCalledExactlyOnceWith([{ ...observation(1), sourceCharacters: 0, scrollTop: 11 }]);
  });

  it("exports only finite bounded viewport widths", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const value = queue(send);
    value.setEnabled(true);
    value.record({ ...observation(1), viewportWidth: 500.7 });
    value.record({ ...observation(2), viewportWidth: -10 });
    value.record({ ...observation(3), viewportWidth: Number.MAX_VALUE });
    value.record({ ...observation(4), viewportWidth: Number.POSITIVE_INFINITY });
    value.record({ ...observation(5), viewportWidth: Number.NaN });
    await vi.advanceTimersByTimeAsync(50);
    expect(send).toHaveBeenCalledExactlyOnceWith([
      { ...observation(1), viewportWidth: 501 },
      { ...observation(2), viewportWidth: 0 },
      { ...observation(3), viewportWidth: Number.MAX_SAFE_INTEGER },
      observation(4), observation(5),
    ]);
  });
});

describe("snapshot gate and content-free projections", () => {
  it("uses Unicode scalar counts and totals independent tracks without double counting the top level", () => {
    expect(subtitleCharacterCount("😀中a")).toBe(3);
    expect(subtitleCharacterCount(null)).toBe(0);
    const tracks = (["system", "microphone"] as const).map(audioSource => ({
      ...session.subtitles, audioSource, detectedLanguage: null,
      isTranslationPending: false, isTranslationTimedOut: false,
      source: { text: "😀a", isFinal: false }, translation: { text: "译", isFinal: false },
    }));
    expect(sessionDebugCounts({ ...session, subtitles: { ...session.subtitles, tracks } }))
      .toEqual({ sourceCharacters: 4, translationCharacters: 2, historyEntries: 0, audioTracks: 2 });
  });

  it("makes no IPC calls for snapshots without a debug ID or for a plain browser preview", async () => {
    observeSessionWireReceived(session);
    observeSessionStoreApplied({ ...session, debugSnapshotId: null });
    observeOverlayCommitted(session, { projection: "empty" });
    observeSessionWireReceived({ ...session, debugSnapshotId: 1 });
    await vi.advanceTimersByTimeAsync(100);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("correlates wire, store and DOM commits while suppressing identical repeated commit effects", async () => {
    vi.stubGlobal("__TAURI_INTERNALS__", {});
    const snapshot = { ...session, debugSnapshotId: 5 };
    observeSessionWireReceived(snapshot);
    observeSessionStoreApplied(snapshot);
    observeOverlayCommitted(snapshot, { projection: "translation", visibleBlocks: 1, projectionRevision: 1 });
    observeOverlayCommitted(snapshot, { projection: "translation", visibleBlocks: 1, projectionRevision: 1 });
    // A same-length stable-text replacement still has its own committed revision.
    observeOverlayCommitted(snapshot, { projection: "translation", visibleBlocks: 1, projectionRevision: 2 });
    await vi.advanceTimersByTimeAsync(50);
    expect(invoke).toHaveBeenCalledOnce();
    const [command, args] = vi.mocked(invoke).mock.calls[0];
    expect(command).toBe("development_debug_observe");
    const samples = (args as { observations: FrontendDebugObservation[] }).observations;
    expect(samples.map(sample => sample.stage)).toEqual(["wireReceived", "storeApplied", "overlayCommitted", "overlayCommitted"]);
    expect(samples.every(sample => sample.snapshotId === 5 && sample.window === "overlay")).toBe(true);
    expect(JSON.stringify(args)).not.toContain(session.subtitles.source.text);
  });

  it("acknowledges a native flush only after delivery and identifies the current allowed window", async () => {
    vi.stubGlobal("__TAURI_INTERNALS__", {});
    native.label = "settings";
    observeSessionWireReceived({ ...session, debugSnapshotId: 8 });
    await Promise.resolve();
    expect(native.flush).not.toBeNull();
    await native.flush!({ payload: { nonce: 42 } });
    expect(vi.mocked(invoke).mock.calls.map(([name]) => name))
      .toEqual(["development_debug_observe", "development_debug_flush_ack"]);
    expect(invoke).toHaveBeenLastCalledWith("development_debug_flush_ack", { nonce: 42, window: "settings" });
  });

  it("does not acknowledge failed flush delivery or accept an unknown native window", async () => {
    vi.stubGlobal("__TAURI_INTERNALS__", {});
    observeSessionWireReceived({ ...session, debugSnapshotId: 9 });
    await Promise.resolve();
    vi.mocked(invoke).mockRejectedValueOnce(new Error("synthetic delivery failure"));
    await native.flush!({ payload: { nonce: 43 } });
    expect(invoke).toHaveBeenCalledOnce();
    native.label = "uncontrolled-window";
    await native.flush!({ payload: { nonce: 44 } });
    expect(invoke).toHaveBeenCalledOnce();
  });

  it("measures submitted display text instead of the full accessible label and reports clipping", () => {
    const root = document.createElement("div");
    root.innerHTML = '<div class="overlay-timeline"><div data-utterance-id="synthetic"><div data-debug-lane="translation" aria-label="Synthetic full repeated sentence"><span>显示 ×40</span></div></div></div>';
    const timeline = root.firstElementChild as HTMLElement;
    const block = timeline.firstElementChild as HTMLElement;
    const lane = block.firstElementChild as HTMLElement;
    const text = lane.firstElementChild as HTMLElement;
    timeline.getBoundingClientRect = () => new DOMRect(0, 0, 500, 100);
    block.getBoundingClientRect = lane.getBoundingClientRect = () => new DOMRect(0, 70, 500, 40);
    text.getBoundingClientRect = () => new DOMRect(0, 30, 500, 80);
    Object.defineProperty(timeline, "clientHeight", { value: 100 });
    Object.defineProperty(timeline, "clientWidth", { value: 500 });
    Object.defineProperty(timeline, "scrollHeight", { value: 200 });
    timeline.scrollTop = 25;
    expect(committedOverlayMeasurements(root)).toEqual({
      visibleCharacters: subtitleCharacterCount("显示 ×40"), visibleBlocks: 1, overflowedBlocks: 1,
      scrollTop: 25, scrollHeight: 200, viewportWidth: 500, viewportHeight: 100,
    });
    block.getBoundingClientRect = () => new DOMRect(0, -50, 500, 40);
    expect(committedOverlayMeasurements(root).visibleBlocks).toBe(0);
    expect(committedOverlayMeasurements(document.createElement("div"))).toEqual({
      visibleCharacters: 0, visibleBlocks: 0, overflowedBlocks: 0,
      scrollTop: 0, scrollHeight: 0, viewportWidth: 0, viewportHeight: 0,
    });
  });
});
