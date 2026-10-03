import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SessionStateEvent } from "./types";

export type DebugWindow = "settings" | "overlay" | "tray-panel" | "overlay-control";
export interface FrontendDebugObservation {
  stage: "wireReceived" | "storeApplied" | "overlayCommitted";
  window: DebugWindow;
  snapshotId: number;
  sourceCharacters: number;
  translationCharacters: number;
  historyEntries: number;
  audioTracks?: number;
  visibleCharacters?: number;
  visibleBlocks?: number;
  overflowedBlocks?: number;
  scrollTop?: number;
  scrollHeight?: number;
  viewportWidth?: number;
  viewportHeight?: number;
  projection?: "empty" | "original" | "translation" | "dual" | "collapsed";
  selectedSourceCharacters?: number;
  selectedTranslationCharacters?: number;
  stableSourceCharacters?: number;
  stableTranslationCharacters?: number;
  clientDropped?: number;
  projectionRevision?: number;
}

export const DEBUG_OBSERVATION_QUEUE_LIMIT = 128;
export const DEBUG_OBSERVATION_BATCH_LIMIT = 16;
const FLUSH_DELAY_MS = 50;
const NUMBER_FIELDS = [
  "sourceCharacters", "translationCharacters", "historyEntries", "audioTracks", "visibleCharacters",
  "visibleBlocks", "overflowedBlocks", "scrollTop", "scrollHeight", "viewportWidth", "viewportHeight",
  "selectedSourceCharacters", "selectedTranslationCharacters", "stableSourceCharacters",
  "stableTranslationCharacters", "clientDropped", "projectionRevision",
] as const;

function validSnapshotId(id: number | null | undefined): id is number {
  return typeof id === "number" && Number.isSafeInteger(id) && id >= 0;
}

/** Rebuild the numeric contract so arbitrary objects can never export content. */
function safeObservation(value: FrontendDebugObservation): FrontendDebugObservation | null {
  if (!validSnapshotId(value.snapshotId) ||
    !["wireReceived", "storeApplied", "overlayCommitted"].includes(value.stage) ||
    !["settings", "overlay", "tray-panel", "overlay-control"].includes(value.window)) return null;
  const result: FrontendDebugObservation = {
    stage: value.stage, window: value.window, snapshotId: value.snapshotId,
    sourceCharacters: 0, translationCharacters: 0, historyEntries: 0,
  };
  for (const field of NUMBER_FIELDS) {
    const number = value[field];
    if (typeof number === "number" && Number.isFinite(number)) {
      result[field] = Math.min(Number.MAX_SAFE_INTEGER, Math.max(0, Math.round(number)));
    }
  }
  if (value.projection !== undefined &&
    ["empty", "original", "translation", "dual", "collapsed"].includes(value.projection)) {
    result.projection = value.projection;
  }
  return result;
}

/** One bounded numeric queue per WebView; IPC never blocks subtitle updates. */
export class DevelopmentTraceQueue {
  private observations: FrontendDebugObservation[] = [];
  private dropped = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private delivery: Promise<void> | null = null;
  private draining: Promise<boolean> | null = null;
  private deliveryFailures = 0;
  private enabled = false;
  private generation = 0;
  private send: (observations: FrontendDebugObservation[]) => Promise<unknown>;

  constructor(send: (observations: FrontendDebugObservation[]) => Promise<unknown>) {
    this.send = send;
  }

  setEnabled(enabled: boolean): void {
    if (this.enabled === enabled) return;
    this.enabled = enabled;
    this.generation += 1;
    if (!enabled) {
      this.observations = [];
      this.dropped = 0;
      if (this.timer !== null) clearTimeout(this.timer);
      this.timer = null;
    }
  }

  record(observation: FrontendDebugObservation): void {
    if (!this.enabled) return;
    const safe = safeObservation(observation);
    if (safe === null) return;
    if (this.observations.length === DEBUG_OBSERVATION_QUEUE_LIMIT) {
      this.observations.shift();
      this.dropped += 1;
    }
    this.observations.push(safe);
    this.schedule();
  }

  private schedule(): void {
    if (this.enabled && this.delivery === null && this.draining === null && this.timer === null && this.observations.length > 0) {
      this.timer = setTimeout(() => {
        this.timer = null;
        void this.flushBatch();
      }, FLUSH_DELAY_MS);
    }
  }

  /** Drain the at-most-128 samples queued at this boundary. Later samples stay
   * bounded and are handled normally; an active stream cannot prolong stop. */
  flushNow(): Promise<boolean> {
    if (this.draining !== null) return this.draining;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    const queuedAtBoundary = this.observations.length;
    const failuresAtBoundary = this.deliveryFailures;
    const generation = this.generation;
    const drain = Promise.resolve().then(async () => {
      if (this.delivery !== null) await this.delivery;
      let remaining = queuedAtBoundary;
      while (this.enabled && generation === this.generation && remaining > 0 && this.observations.length > 0) {
        const size = Math.min(DEBUG_OBSERVATION_BATCH_LIMIT, remaining, this.observations.length);
        await this.flushBatch(size);
        remaining -= size;
      }
      return failuresAtBoundary === this.deliveryFailures && generation === this.generation && this.dropped === 0;
    }).finally(() => {
      if (this.draining === drain) this.draining = null;
      this.schedule();
    });
    this.draining = drain;
    return drain;
  }

  private flushBatch(limit = DEBUG_OBSERVATION_BATCH_LIMIT): Promise<void> {
    if (this.delivery !== null) return this.delivery;
    if (!this.enabled || this.observations.length === 0) return Promise.resolve();
    const generation = this.generation;
    const batch = this.observations.splice(0, limit);
    const reportedDropped = this.dropped;
    this.dropped = 0;
    if (reportedDropped > 0) batch[0].clientDropped = reportedDropped;
    const delivery = Promise.resolve().then(() => this.send(batch)).then(() => {}, () => {
      // No retries can delay or grow the live pipeline. Report missing samples
      // with the next successful batch, including an undelivered drop report.
      if (this.enabled && generation === this.generation) {
        this.dropped += batch.length + reportedDropped;
        this.deliveryFailures += 1;
      }
    }).finally(() => {
      if (this.delivery === delivery) this.delivery = null;
      this.schedule();
    });
    this.delivery = delivery;
    return delivery;
  }
}

const traceQueue = new DevelopmentTraceQueue(observations =>
  invoke("development_debug_observe", { observations }));
let lastOverlayObservation = "";
let flushListener: Promise<UnlistenFn> | null = null;

function ensureFlushListener(): void {
  if (flushListener !== null) return;
  const registration = listen<{ nonce: number }>("development-debug-flush", async event => {
    const nonce = event.payload?.nonce;
    const window = nativeWindowLabel();
    if (!validSnapshotId(nonce) || window === null) return;
    try {
      // A failed transport is deliberately left unacknowledged, so the native
      // stop report can state that the observation boundary is incomplete.
      if (await traceQueue.flushNow()) {
        await invoke("development_debug_flush_ack", { nonce, window });
      }
    } catch { /* Observability must never affect the subtitle pipeline. */ }
  }).catch(() => {
    if (flushListener === registration) flushListener = null;
    return () => {};
  });
  flushListener = registration;
}

/** Unicode scalar count, matching Rust's chars().count(), without a text copy. */
export function subtitleCharacterCount(text: string | null | undefined): number {
  let characters = 0;
  if (text) for (let index = 0; index < text.length; index += 1) {
    if (text.codePointAt(index)! > 0xffff) index += 1;
    characters += 1;
  }
  return characters;
}

function nativeWindowLabel(): DebugWindow | null {
  if (typeof window === "undefined" ||
    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ === undefined) return null;
  try {
    const label = getCurrentWindow().label;
    return label === "settings" || label === "overlay" || label === "tray-panel" || label === "overlay-control"
      ? label : null;
  } catch { return null; }
}

export function sessionDebugCounts(session: SessionStateEvent): Pick<FrontendDebugObservation,
  "sourceCharacters" | "translationCharacters" | "historyEntries" | "audioTracks"> {
  const subtitles = session.subtitles;
  const inputs = subtitles.tracks?.length ? subtitles.tracks : [subtitles];
  return {
    sourceCharacters: inputs.reduce((count, track) => count + subtitleCharacterCount(track.source.text), 0),
    translationCharacters: inputs.reduce((count, track) => count + subtitleCharacterCount(track.translation.text), 0),
    historyEntries: subtitles.history.length,
    audioTracks: inputs.length,
  };
}

function observeSession(stage: FrontendDebugObservation["stage"], session: SessionStateEvent,
  details: Partial<FrontendDebugObservation> = {}): void {
  if (!validSnapshotId(session.debugSnapshotId)) {
    traceQueue.setEnabled(false);
    lastOverlayObservation = "";
    return;
  }
  const window = nativeWindowLabel();
  if (window === null) return;
  ensureFlushListener();
  const observation = safeObservation({ ...details, ...sessionDebugCounts(session),
    stage, window, snapshotId: session.debugSnapshotId });
  if (observation === null) return;
  if (stage === "overlayCommitted") {
    const key = JSON.stringify(observation);
    if (key === lastOverlayObservation) return;
    lastOverlayObservation = key;
  }
  traceQueue.setEnabled(true);
  traceQueue.record(observation);
}

export function observeSessionWireReceived(session: SessionStateEvent): void {
  observeSession("wireReceived", session);
}

export function observeSessionStoreApplied(session: SessionStateEvent): void {
  observeSession("storeApplied", session);
}

export function observeOverlayCommitted(session: SessionStateEvent,
  details: Partial<FrontendDebugObservation>): void {
  observeSession("overlayCommitted", session, details);
}

/** DOM commit/layout evidence, not proof that an OS window was seen on screen.
 * Character counts describe submitted display text in intersecting lanes;
 * overflow separately reports partial clipping instead of guessing glyphs. */
export function committedOverlayMeasurements(root: HTMLElement): Pick<FrontendDebugObservation,
  "visibleCharacters" | "visibleBlocks" | "overflowedBlocks" | "scrollTop" | "scrollHeight" | "viewportWidth" | "viewportHeight"> {
  const timeline = root.querySelector<HTMLElement>(".overlay-timeline");
  if (timeline === null) return { visibleCharacters: 0, visibleBlocks: 0, overflowedBlocks: 0,
    scrollTop: 0, scrollHeight: 0, viewportWidth: 0, viewportHeight: 0 };
  const viewport = timeline.getBoundingClientRect();
  const intersects = (rect: DOMRect) => rect.height > 0 && rect.width > 0 &&
    rect.bottom > viewport.top && rect.top < viewport.bottom &&
    rect.right > viewport.left && rect.left < viewport.right;
  let visibleCharacters = 0;
  let visibleBlocks = 0;
  let overflowedBlocks = 0;
  for (const block of timeline.querySelectorAll<HTMLElement>("[data-utterance-id]")) {
    const rect = block.getBoundingClientRect();
    if (!intersects(rect)) continue;
    visibleBlocks += 1;
    let overflowed = rect.top < viewport.top - 2 || rect.bottom > viewport.bottom + 2;
    for (const lane of block.querySelectorAll<HTMLElement>("[data-debug-lane]")) {
      const laneRect = lane.getBoundingClientRect();
      if (!intersects(laneRect)) continue;
      const text = lane.firstElementChild instanceof HTMLElement ? lane.firstElementChild : lane;
      visibleCharacters += subtitleCharacterCount(text.textContent);
      if (text.getBoundingClientRect().height > laneRect.height + 2) overflowed = true;
    }
    if (overflowed) overflowedBlocks += 1;
  }
  return { visibleCharacters, visibleBlocks, overflowedBlocks,
    scrollTop: timeline.scrollTop, scrollHeight: timeline.scrollHeight,
    viewportWidth: timeline.clientWidth, viewportHeight: timeline.clientHeight };
}
