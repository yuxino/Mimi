import type { SettingsDraft, SettingsSnapshot } from "./types";
import { DEFAULT_NETWORK_PROXY, validateNetworkProxy } from "./networkProxy";

type Unlisten = () => void;

interface SnapshotStreamSources<Settings, Session> {
  listenSettings: (
    handler: (settings: Settings) => void,
  ) => Promise<Unlisten>;
  listenSession: (handler: (session: Session) => void) => Promise<Unlisten>;
  getSettings: () => Promise<Settings>;
  getSession: () => Promise<Session>;
}

interface SnapshotStreamConsumers<Settings, Session> {
  applySettings: (settings: Settings) => void;
  applySession: (session: Session) => void;
  onReady?: () => void;
}

export const SNAPSHOT_STEP_TIMEOUT_MS = 12_000;

export class SnapshotBootstrapTimeoutError extends Error {
  constructor() { super("snapshot-step-timeout"); }
}

function withSnapshotDeadline<Value>(operation: Promise<Value>, signal: AbortSignal): Promise<Value> {
  return new Promise((resolve, reject) => {
    const cleanup = () => { clearTimeout(timer); signal.removeEventListener("abort", abort); };
    const abort = () => { cleanup(); reject(new Error("snapshot-stream-disposed")); };
    const timer = setTimeout(() => { cleanup(); reject(new SnapshotBootstrapTimeoutError()); }, SNAPSHOT_STEP_TIMEOUT_MS);
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) abort();
    operation.then(
      (value) => { cleanup(); resolve(value); },
      (error: unknown) => { cleanup(); reject(error); },
    );
  });
}

/**
 * One stream owns its listener and pending native read for the WebView's
 * lifetime. A UI deadline cannot cancel a Keychain call and must not remove
 * healthy listeners. Explicit retries reuse unfinished native operations.
 */
class SnapshotStream<Snapshot> {
  private lifetime = new AbortController();
  private listener: Promise<void> | null = null;
  private unlisten: Unlisten | null = null;
  private read: Promise<void> | null = null;
  private readFailed = false;
  private revision = 0;
  private hasSnapshot = false;
  private listen: (handler: (snapshot: Snapshot) => void) => Promise<Unlisten>;
  private getSnapshot: () => Promise<Snapshot>;
  private apply: (snapshot: Snapshot) => void;
  private progressed: () => void;

  constructor(
    listen: (handler: (snapshot: Snapshot) => void) => Promise<Unlisten>,
    getSnapshot: () => Promise<Snapshot>,
    apply: (snapshot: Snapshot) => void,
    progressed: () => void,
  ) {
    this.listen = listen;
    this.getSnapshot = getSnapshot;
    this.apply = apply;
    this.progressed = progressed;
  }

  get ready(): boolean { return this.hasSnapshot && this.unlisten !== null; }

  private accept(snapshot: Snapshot): void {
    if (this.lifetime.signal.aborted) return;
    this.hasSnapshot = true;
    this.apply(snapshot);
    this.progressed();
  }

  private ensureListener(): Promise<void> {
    if (this.unlisten !== null) return Promise.resolve();
    if (this.listener !== null) return this.listener;
    const listener = Promise.resolve().then(() => {
      if (this.lifetime.signal.aborted) throw new Error("snapshot-stream-disposed");
      return this.listen((snapshot) => {
        if (this.lifetime.signal.aborted) return;
        this.revision += 1;
        // Events are already complete snapshots. Publish promptly, including
        // while credential hydration is pending or its UI deadline has expired.
        this.accept(snapshot);
      });
    }).then((unlisten) => {
      if (this.lifetime.signal.aborted) safelyUnlisten(unlisten);
      else {
        this.unlisten = unlisten;
        this.progressed();
        // A listener acknowledgement may arrive after initialize's deadline.
        // Start its first read once, so hidden windows recover without a retry
        // button. This is not a re-read or a background retry after failure.
        if (!this.hasSnapshot) void this.ensureRead().catch(() => {});
      }
    }, () => { throw new Error("snapshot-listener-unavailable"); })
      .finally(() => { if (this.listener === listener) this.listener = null; });
    this.listener = listener;
    return listener;
  }

  private ensureRead(): Promise<void> {
    if (this.read !== null) return this.read;
    const revision = this.revision;
    const read = Promise.resolve().then(() => {
      if (this.lifetime.signal.aborted) throw new Error("snapshot-stream-disposed");
      return this.getSnapshot();
    }).then((snapshot) => {
      // A newer event wins even when the old read finishes hours after timeout.
      if (revision === this.revision) this.accept(snapshot);
    }, () => {
      this.readFailed = true;
      throw new Error("boot-snapshot-unavailable");
    });
    this.read = read;
    return read;
  }

  async initialize(): Promise<void> {
    if (this.lifetime.signal.aborted || this.ready) return;
    // Only a new explicit attempt may replace an actually failed read. Keep
    // even a settled first read while its listener acknowledgement reconciles,
    // so an immediate rejection cannot cause an automatic second native read.
    if (this.readFailed) { this.read = null; this.readFailed = false; }
    await withSnapshotDeadline(this.ensureListener(), this.lifetime.signal);
    if (this.lifetime.signal.aborted || this.ready) return;
    await withSnapshotDeadline(this.ensureRead(), this.lifetime.signal);
  }

  dispose(): void {
    this.lifetime.abort();
    if (this.unlisten !== null) safelyUnlisten(this.unlisten);
    this.unlisten = null;
  }
}

function safelyUnlisten(unlisten: Unlisten): void {
  try { unlisten(); } catch { /* Other stream cleanup must still run. */ }
}

/** Independent, persistent streams; retries never recreate healthy subscriptions. */
export class SnapshotStreamBootstrap<Settings, Session> {
  private settings: SnapshotStream<Settings>;
  private session: SnapshotStream<Session>;
  private disposed = false;
  private reportedReady = false;

  constructor(sources: SnapshotStreamSources<Settings, Session>, consumers: SnapshotStreamConsumers<Settings, Session>) {
    const progressed = () => {
      if (!this.disposed && this.ready && !this.reportedReady) {
        this.reportedReady = true;
        consumers.onReady?.();
      }
    };
    this.settings = new SnapshotStream(sources.listenSettings, sources.getSettings, consumers.applySettings, progressed);
    this.session = new SnapshotStream(sources.listenSession, sources.getSession, consumers.applySession, progressed);
  }

  get ready(): boolean { return !this.disposed && this.settings.ready && this.session.ready; }

  async initialize(): Promise<void> {
    if (this.disposed) return;
    const results = await Promise.allSettled([this.settings.initialize(), this.session.initialize()]);
    if (this.disposed || this.ready) return;
    for (const result of results) if (result.status === "rejected") throw result.reason;
  }

  dispose(): void {
    this.disposed = true;
    this.settings.dispose();
    this.session.dispose();
  }
}

export function mergeSettingsSnapshot(
  current: SettingsSnapshot,
  draft: SettingsDraft,
): SettingsSnapshot {
  const networkProxy = draft.networkProxy === undefined ? null
    : validateNetworkProxy(draft.networkProxy.mode, draft.networkProxy.url);
  const recordingInputChanged =
    (draft.systemAudioTarget !== undefined && JSON.stringify(draft.systemAudioTarget) !== JSON.stringify(current.systemAudioTarget ?? { kind: "system" }))
    || (draft.audioInput !== undefined && draft.audioInput !== (current.audioInput ?? "system"))
    || (draft.windowsAudioSource !== undefined && draft.windowsAudioSource !== (current.windowsAudioSource ?? ""));
  return {
    ...current,
    sourceLanguage: draft.sourceLanguage ?? current.sourceLanguage,
    targetLanguage: draft.targetLanguage ?? current.targetLanguage,
    translationMode: draft.translationMode ?? current.translationMode,
    fontSize: draft.fontSize ?? current.fontSize,
    subtitleFontFamily: draft.subtitleFontFamily ?? current.subtitleFontFamily,
    subtitleBackgroundOpacity: draft.subtitleBackgroundOpacity ?? current.subtitleBackgroundOpacity ?? 80,
    subtitleColor: draft.subtitleColor ?? current.subtitleColor,
    microphoneSubtitleColor: draft.microphoneSubtitleColor ?? current.microphoneSubtitleColor ?? "yellow",
    subtitleAlignment: draft.subtitleAlignment ?? current.subtitleAlignment,
    subtitleDisplayMode: draft.subtitleDisplayMode ?? current.subtitleDisplayMode,
    showIntermediateSubtitles: draft.showIntermediateSubtitles ?? current.showIntermediateSubtitles ?? true,
    showSubtitleDividers: draft.showSubtitleDividers ?? current.showSubtitleDividers,
    showSubtitleTimestamps: draft.showSubtitleTimestamps ?? current.showSubtitleTimestamps ?? false,
    pulseAnimation: draft.pulseAnimation ?? current.pulseAnimation,
    pulseStyle: draft.pulseStyle ?? current.pulseStyle,
    subtitleAnimation: draft.subtitleAnimation ?? current.subtitleAnimation,
    audioInput: draft.audioInput ?? current.audioInput ?? "system",
    systemAudioTarget: draft.systemAudioTarget ?? current.systemAudioTarget ?? { kind: "system" },
    windowsAudioSource: draft.windowsAudioSource ?? current.windowsAudioSource,
    showInDock: draft.showInDock ?? current.showInDock,
    // Do not put an invalid or credential-bearing URL into the global UI
    // snapshot while native validation is still pending.
    networkProxy: networkProxy && "config" in networkProxy ? networkProxy.config : current.networkProxy ?? DEFAULT_NETWORK_PROXY,
    subtitleBlendsWithBackground:
      draft.subtitleBlendsWithBackground ??
      current.subtitleBlendsWithBackground,
    isOverlayLocked: draft.isOverlayLocked ?? current.isOverlayLocked,
    uiLanguage: draft.uiLanguage ?? current.uiLanguage,
    retainSessionHistory:
      draft.retainSessionHistory ?? current.retainSessionHistory,
    // A prior recording opt-in must never silently cover a different input.
    recordSessionAudio: recordingInputChanged
      ? false : draft.recordSessionAudio ?? current.recordSessionAudio,
  };
}

/**
 * Applies settings drafts immediately and keeps the last confirmed snapshot
 * for rollback. Consecutive slider edits share the latest queued write; other
 * drafts remain ordered barriers. External snapshots invalidate pending UI
 * responses without allowing their older results to overwrite the store.
 */
export class SettingsSaveCoordinator {
  private generation = 0;
  private epoch = 0;
  private persistenceQueue: Promise<void> = Promise.resolve();
  private queuedSlider: SettingsSaveOperation | null = null;
  private confirmedSnapshot: SettingsSnapshot | null = null;
  private activeSaves = 0;
  private sliderPreviews = new Map<SliderField, { value: number; generation: number }>();
  private submittedSlider: { draft: SettingsDraft; generations: Partial<Record<SliderField, number>> } | null = null;
  private sliderRevisions: Record<SliderField, number> = { fontSize: 0, subtitleBackgroundOpacity: 0 };
  private receivedSnapshot: SettingsSnapshot | null = null;
  private displayedSnapshot: SettingsSnapshot | null = null;

  invalidate(): void {
    this.sliderPreviews.clear();
    this.receivedSnapshot = null;
    this.displayedSnapshot = null;
    for (const field of SLIDER_FIELDS) this.sliderRevisions[field] += 1;
    this.invalidateResponses();
  }

  private invalidateResponses(): void {
    this.generation += 1;
    this.epoch += 1;
    this.confirmedSnapshot = null;
  }

  /** Native events can acknowledge an earlier slider save while a newer
   * local value is still queued. Other fields always belong to the event. */
  receiveSnapshot(snapshot: SettingsSnapshot): SettingsSnapshot {
    for (const field of SLIDER_FIELDS) {
      const value = sliderValue(snapshot, field);
      const unchanged = this.receivedSnapshot !== null && value === sliderValue(this.receivedSnapshot, field);
      const submitted = value === this.submittedSlider?.draft[field];
      if (this.receivedSnapshot !== null && !unchanged && !submitted) this.sliderRevisions[field] += 1;
      const preview = this.sliderPreviews.get(field);
      if (!preview) continue;
      // A different native value supersedes the local preview. A matching
      // value confirms only the latest operation: the user may drag back to
      // an earlier value while an intermediate value is still queued.
      const acknowledged = value === preview.value && this.submittedSlider?.generations[field] === preview.generation;
      if (acknowledged || (value !== preview.value && !unchanged && !submitted)) this.sliderPreviews.delete(field);
    }
    this.invalidateResponses();
    this.receivedSnapshot = snapshot;
    return this.preview(snapshot);
  }

  private preview(snapshot: SettingsSnapshot): SettingsSnapshot {
    const draft: SettingsDraft = {};
    for (const [field, preview] of this.sliderPreviews) draft[field] = preview.value;
    this.displayedSnapshot = this.sliderPreviews.size ? mergeSettingsSnapshot(snapshot, draft) : snapshot;
    return this.displayedSnapshot;
  }

  async save(
    previous: SettingsSnapshot,
    draft: SettingsDraft,
    persist: (draft: SettingsDraft) => Promise<SettingsSnapshot>,
    apply: (settings: SettingsSnapshot) => void,
  ): Promise<void> {
    const generation = ++this.generation;
    const epoch = this.epoch;
    const sliderFields = sliderDraftFields(draft);
    this.receivedSnapshot ??= previous;
    // A mixed draft keeps the ordinary transaction and opt-out semantics.
    // Its explicit fields supersede a prior standalone slider preview.
    if (!sliderFields.length) {
      if (draft.fontSize !== undefined) this.sliderPreviews.delete("fontSize");
      if (draft.subtitleBackgroundOpacity !== undefined) this.sliderPreviews.delete("subtitleBackgroundOpacity");
    }
    for (const field of sliderFields) this.sliderPreviews.set(field, { value: draft[field]!, generation });
    if (this.confirmedSnapshot === null) {
      this.confirmedSnapshot = previous;
    }
    const generations = Object.fromEntries(sliderFields.map(field => [field, generation]));
    let queued = sliderFields.length ? this.queuedSlider : null;
    if (queued) {
      // Keep only the latest value for each field, and let every caller observe
      // the shared write's completion or failure. Never merge an active write.
      Object.assign(queued.draft, draft);
      Object.assign(queued.sliderGenerations, generations);
      Object.assign(queued, { generation, epoch, persist, apply });
    } else {
      queued = { previous, draft: { ...draft }, generation, epoch, persist, apply,
        sliderGenerations: generations, completion: Promise.resolve() };
      const pending = queued;
      // Reserve the first write immediately, before its microtask starts. All
      // later slider edits can share one queued successor until a barrier.
      this.queuedSlider = sliderFields.length && this.activeSaves > 0 ? pending : null;
      this.activeSaves += 1;
      const operation = this.persistenceQueue.then(() => this.persist(pending));
      pending.completion = operation;
      // A failed write must reject its callers without poisoning later saves.
      this.persistenceQueue = operation.catch(() => {});
    }
    apply(this.preview(mergeSettingsSnapshot(previous, draft)));
    return queued.completion;
  }

  private async persist(operation: SettingsSaveOperation): Promise<void> {
    if (this.queuedSlider === operation) this.queuedSlider = null;
    const { previous, draft, generation, epoch, persist, apply, sliderGenerations } = operation;
    const sliderFields = sliderDraftFields(draft);
    const publish = (snapshot: SettingsSnapshot) => apply(this.preview(snapshot));
    this.submittedSlider = sliderFields.length ? { draft, generations: sliderGenerations } : null;
    const revisions = { ...this.sliderRevisions };
    try {
      const snapshot = await persist(draft);
      if (epoch === this.epoch) {
        this.confirmedSnapshot = snapshot;
        this.receivedSnapshot = snapshot;
      }
      if (sliderFields.length) {
        const acknowledged: SettingsDraft = {};
        const confirmed: SettingsDraft = {};
        for (const field of sliderFields) {
          const preview = this.sliderPreviews.get(field);
          if (!preview) continue;
          // A late acknowledgement from before an external edit cannot
          // become the rollback baseline for a subsequent local intent,
          // even when external edits returned to the same numeric value.
          if (revisions[field] !== this.sliderRevisions[field]) continue;
          confirmed[field] = sliderValue(snapshot, field);
          if (preview.generation === sliderGenerations[field]) {
            this.sliderPreviews.delete(field);
            acknowledged[field] = confirmed[field];
          }
        }
        if (Object.keys(confirmed).length) {
          this.receivedSnapshot = mergeSettingsSnapshot(this.receivedSnapshot ?? previous, confirmed);
          if (this.confirmedSnapshot !== null) this.confirmedSnapshot = mergeSettingsSnapshot(this.confirmedSnapshot, confirmed);
        }
        if (Object.keys(acknowledged).length) {
          const base = epoch === this.epoch && generation === this.generation ? snapshot : this.displayedSnapshot ?? previous;
          publish(mergeSettingsSnapshot(base, acknowledged));
        }
      } else if (epoch === this.epoch && generation === this.generation) {
        publish(snapshot);
      }
    } catch (error) {
      if (sliderFields.length) {
        const rollback: SettingsDraft = {};
        for (const field of sliderFields) {
          if (this.sliderPreviews.get(field)?.generation !== sliderGenerations[field]) continue;
          this.sliderPreviews.delete(field);
          rollback[field] = sliderValue(this.receivedSnapshot ?? previous, field);
        }
        if (Object.keys(rollback).length) {
          if (this.confirmedSnapshot !== null) this.confirmedSnapshot = mergeSettingsSnapshot(this.confirmedSnapshot, rollback);
          const base = epoch === this.epoch && generation === this.generation
            ? this.confirmedSnapshot ?? previous : this.displayedSnapshot ?? previous;
          publish(mergeSettingsSnapshot(base, rollback));
        }
      } else if (epoch === this.epoch && generation === this.generation) {
        publish(this.confirmedSnapshot ?? previous);
      }
      throw error;
    } finally {
      this.submittedSlider = null;
      this.activeSaves -= 1;
      if (this.activeSaves === 0) this.confirmedSnapshot = null;
    }
  }
}

type SliderField = "fontSize" | "subtitleBackgroundOpacity";
interface SettingsSaveOperation {
  previous: SettingsSnapshot;
  draft: SettingsDraft;
  generation: number;
  epoch: number;
  persist: (draft: SettingsDraft) => Promise<SettingsSnapshot>;
  apply: (settings: SettingsSnapshot) => void;
  sliderGenerations: Partial<Record<SliderField, number>>;
  completion: Promise<void>;
}
const SLIDER_FIELDS: SliderField[] = ["fontSize", "subtitleBackgroundOpacity"];
function sliderDraftFields(draft: SettingsDraft): SliderField[] {
  const fields = Object.keys(draft);
  return fields.length && fields.every((field): field is SliderField =>
    (field === "fontSize" || field === "subtitleBackgroundOpacity") && typeof draft[field] === "number")
    ? fields : [];
}
function sliderValue(snapshot: SettingsSnapshot, field: SliderField): number {
  return field === "fontSize" ? snapshot.fontSize : snapshot.subtitleBackgroundOpacity ?? 80;
}

/** Prevents an older command response from overwriting a newer event or save. */
export class SnapshotResponseGate {
  private revision = 0;

  capture(): number {
    return this.revision;
  }

  advance(): void {
    this.revision += 1;
  }

  applyIfCurrent(expectedRevision: number): boolean {
    if (expectedRevision !== this.revision) return false;
    this.advance();
    return true;
  }
}
