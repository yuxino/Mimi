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
  return {
    ...current,
    sourceLanguage: draft.sourceLanguage ?? current.sourceLanguage,
    targetLanguage: draft.targetLanguage ?? current.targetLanguage,
    translationMode: draft.translationMode ?? current.translationMode,
    fontSize: draft.fontSize ?? current.fontSize,
    subtitleBackgroundOpacity: draft.subtitleBackgroundOpacity ?? current.subtitleBackgroundOpacity ?? 80,
    subtitleColor: draft.subtitleColor ?? current.subtitleColor,
    subtitleAlignment: draft.subtitleAlignment ?? current.subtitleAlignment,
    subtitleDisplayMode: draft.subtitleDisplayMode ?? current.subtitleDisplayMode,
    showSubtitleDividers: draft.showSubtitleDividers ?? current.showSubtitleDividers,
    pulseAnimation: draft.pulseAnimation ?? current.pulseAnimation,
    pulseStyle: draft.pulseStyle ?? current.pulseStyle,
    subtitleAnimation: draft.subtitleAnimation ?? current.subtitleAnimation,
    audioInput: draft.audioInput ?? current.audioInput ?? "system",
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
    recordSessionAudio: draft.audioInput !== undefined && draft.audioInput !== (current.audioInput ?? "system")
      ? false : draft.recordSessionAudio ?? current.recordSessionAudio,
  };
}

/**
 * Applies settings drafts immediately, persists them in call order, and keeps
 * the last confirmed snapshot for rollback. External snapshots invalidate
 * pending UI responses without allowing their older results to overwrite the
 * store.
 */
export class SettingsSaveCoordinator {
  private generation = 0;
  private epoch = 0;
  private persistenceQueue: Promise<void> = Promise.resolve();
  private confirmedSnapshot: SettingsSnapshot | null = null;
  private activeSaves = 0;

  invalidate(): void {
    this.generation += 1;
    this.epoch += 1;
    this.confirmedSnapshot = null;
  }

  async save(
    previous: SettingsSnapshot,
    draft: SettingsDraft,
    persist: (draft: SettingsDraft) => Promise<SettingsSnapshot>,
    apply: (settings: SettingsSnapshot) => void,
  ): Promise<void> {
    const generation = ++this.generation;
    const epoch = this.epoch;
    if (this.confirmedSnapshot === null) {
      this.confirmedSnapshot = previous;
    }
    this.activeSaves += 1;
    apply(mergeSettingsSnapshot(previous, draft));

    const operation = this.persistenceQueue.then(async () => {
      try {
        const snapshot = await persist(draft);
        if (epoch === this.epoch) {
          this.confirmedSnapshot = snapshot;
          if (generation === this.generation) apply(snapshot);
        }
      } catch (error) {
        if (epoch === this.epoch && generation === this.generation) {
          apply(this.confirmedSnapshot ?? previous);
        }
        throw error;
      } finally {
        this.activeSaves -= 1;
        if (this.activeSaves === 0) this.confirmedSnapshot = null;
      }
    });

    // A failed save must reject its own caller without poisoning later saves.
    this.persistenceQueue = operation.catch(() => {});
    return operation;
  }
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
