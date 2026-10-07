// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SessionStateEvent, SettingsSnapshot } from "./types";
import { SNAPSHOT_STEP_TIMEOUT_MS } from "./settingsState";
import { disposeStoreSnapshotStreams, useStore } from "./store";

const sources = vi.hoisted(() => ({ listenSettings: vi.fn(), listenSession: vi.fn(), getSettings: vi.fn(), getSession: vi.fn() }));
vi.mock("./ipc", async (original) => ({ ...await original<typeof import("./ipc")>(), isTauri: true,
  listenSettingsChanged: sources.listenSettings, listenSessionState: sources.listenSession,
  settingsGet: sources.getSettings, sessionGetState: sources.getSession,
}));

const initial = useStore.getState();
const settings: SettingsSnapshot = { ...initial.settings, activeProfileId: "existing", profiles: [{ id: "existing", provider: "alibabaCloud", name: "Current service", credentialState: "present" }] };
const session: SessionStateEvent = { ...initial.session, status: { kind: "listening" }, isActive: true };
beforeEach(() => {
  disposeStoreSnapshotStreams();
  vi.useFakeTimers();
  Object.values(sources).forEach((source) => source.mockReset());
  sources.listenSettings.mockResolvedValue(vi.fn()); sources.listenSession.mockResolvedValue(vi.fn());
  sources.getSettings.mockResolvedValue(settings); sources.getSession.mockResolvedValue(session);
  useStore.setState(initial, true);
});
afterEach(() => { disposeStoreSnapshotStreams(); vi.useRealTimers(); useStore.setState(initial, true); });

it("keeps native startup and a fresh settings snapshot free of placeholder profiles", async () => {
  expect(initial.settings.profiles).toEqual([]);
  expect(initial.settings.activeProfileId).toBe("");
  sources.getSettings.mockResolvedValueOnce(initial.settings);
  await useStore.getState().init();
  expect(useStore.getState().settings.profiles).toEqual([]);
  expect(useStore.getState().initializationStatus).toBe("ready");
});

it("keeps overlay session events alive after overnight settings timeout and accepts the original late Keychain response", async () => {
  let complete!: (snapshot: SettingsSnapshot) => void;
  let sessionHandler!: (snapshot: SessionStateEvent) => void;
  const unlistenSettings = vi.fn(); const unlistenSession = vi.fn();
  sources.listenSettings.mockResolvedValueOnce(unlistenSettings);
  sources.listenSession.mockImplementationOnce(async (handler) => { sessionHandler = handler; return unlistenSession; });
  sources.getSettings.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  sources.getSession.mockResolvedValueOnce(initial.session);
  const first = useStore.getState().init();
  expect(useStore.getState().init()).toBe(first);
  await vi.advanceTimersByTimeAsync(SNAPSHOT_STEP_TIMEOUT_MS); await first;
  expect(useStore.getState()).toMatchObject({ initialized: false, initializationStatus: "error", initializationError: "timeout" });
  expect(unlistenSettings).not.toHaveBeenCalled(); expect(unlistenSession).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(8 * 60 * 60 * 1000);
  const listening = { ...session, subtitles: { ...session.subtitles,
    previewPair: { source: "Synthetic source", translation: "Synthetic translation" } } };
  sessionHandler(listening);
  expect(useStore.getState().session.status.kind).toBe("listening");
  expect(useStore.getState().session.isActive).toBe(true);
  expect(useStore.getState().session.subtitles.previewPair).toEqual(listening.subtitles.previewPair);
  expect(sources.getSettings).toHaveBeenCalledOnce();
  const persisted = { ...settings, fontSize: 24, subtitleDisplayMode: "bilingual" as const,
    networkProxy: { mode: "direct" as const, url: null } };
  complete(persisted); await vi.advanceTimersByTimeAsync(0);
  expect(useStore.getState().settings).toEqual(persisted);
  expect(useStore.getState()).toMatchObject({ initialized: true, initializationStatus: "ready", initializationError: null, hasSettingsSnapshot: true });
  expect(sources.listenSettings).toHaveBeenCalledOnce(); expect(sources.listenSession).toHaveBeenCalledOnce();
});

it("reuses the blocked read/listeners on retry and prevents a late response from overwriting a newer event", async () => {
  let complete!: (snapshot: SettingsSnapshot) => void;
  let settingsHandler!: (snapshot: SettingsSnapshot) => void;
  sources.listenSettings.mockImplementationOnce(async (handler) => { settingsHandler = handler; return vi.fn(); });
  sources.getSettings.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve; }));
  const first = useStore.getState().init();
  await vi.advanceTimersByTimeAsync(SNAPSHOT_STEP_TIMEOUT_MS); await first;
  const retry = useStore.getState().init();
  expect(useStore.getState().init()).toBe(retry);
  await vi.advanceTimersByTimeAsync(0);
  const newer = { ...settings, fontSize: 24 };
  settingsHandler(newer);
  expect(useStore.getState().initializationStatus).toBe("ready");
  complete({ ...settings, fontSize: 22 }); await retry;
  expect(useStore.getState().settings).toEqual(newer);
  expect(sources.getSettings).toHaveBeenCalledOnce(); expect(sources.getSession).toHaveBeenCalledOnce();
  expect(sources.listenSettings).toHaveBeenCalledOnce(); expect(sources.listenSession).toHaveBeenCalledOnce();
});

it("keeps a pending listener through timeout and retry without duplicate native registrations", async () => {
  let finishListener!: (unlisten: () => void) => void;
  sources.listenSettings.mockImplementationOnce(() => new Promise((resolve) => { finishListener = resolve; }));
  const first = useStore.getState().init();
  await vi.advanceTimersByTimeAsync(SNAPSHOT_STEP_TIMEOUT_MS); await first;
  expect(useStore.getState().initializationError).toBe("timeout");
  expect(sources.getSettings).not.toHaveBeenCalled();
  const retry = useStore.getState().init();
  await vi.advanceTimersByTimeAsync(0);
  const unlisten = vi.fn(); finishListener(unlisten); await retry;
  expect(unlisten).not.toHaveBeenCalled();
  expect(sources.listenSettings).toHaveBeenCalledOnce();
  expect(sources.getSettings).toHaveBeenCalledOnce();
  expect(useStore.getState().initializationStatus).toBe("ready");
});

it("recovers a hidden overlay after late listener acknowledgement without a retry button", async () => {
  let finishListener!: (unlisten: () => void) => void;
  sources.listenSettings.mockImplementationOnce(() => new Promise((resolve) => { finishListener = resolve; }));
  const first = useStore.getState().init();
  await vi.advanceTimersByTimeAsync(SNAPSHOT_STEP_TIMEOUT_MS); await first;
  expect(useStore.getState().initializationStatus).toBe("error");
  expect(sources.getSettings).not.toHaveBeenCalled();
  finishListener(vi.fn()); await vi.advanceTimersByTimeAsync(0);
  expect(useStore.getState().settings).toBe(settings);
  expect(useStore.getState().initializationStatus).toBe("ready");
  expect(sources.listenSettings).toHaveBeenCalledOnce();
  expect(sources.getSettings).toHaveBeenCalledOnce();
});

it("permits a new read after genuine IPC rejection while retaining both event listeners", async () => {
  sources.getSettings.mockRejectedValueOnce("synthetic-native-error");
  await useStore.getState().init();
  expect(useStore.getState()).toMatchObject({ initialized: false, initializationStatus: "error", initializationError: "unavailable" });
  expect(useStore.getState().settings).toBe(initial.settings);
  expect(useStore.getState().hasSettingsSnapshot).toBe(false);
  await useStore.getState().init();
  expect(useStore.getState().settings).toBe(settings);
  expect(useStore.getState().initializationStatus).toBe("ready");
  expect(sources.getSettings).toHaveBeenCalledTimes(2);
  expect(sources.listenSettings).toHaveBeenCalledOnce(); expect(sources.listenSession).toHaveBeenCalledOnce();
});

it("expires only a disposed WebView generation and rejects its callbacks after replacement hydration", async () => {
  let completeOld!: (snapshot: SettingsSnapshot) => void;
  let oldHandler!: (snapshot: SettingsSnapshot) => void;
  let finishOldListener!: (unlisten: () => void) => void;
  sources.listenSettings.mockImplementationOnce(async (handler) => { oldHandler = handler; return vi.fn(); });
  sources.getSettings.mockImplementationOnce(() => new Promise((resolve) => { completeOld = resolve; }));
  sources.listenSession.mockImplementationOnce(() => new Promise((resolve) => { finishOldListener = resolve; }));
  const old = useStore.getState().init(); await vi.advanceTimersByTimeAsync(0);
  window.dispatchEvent(new Event("pagehide")); await old;
  useStore.setState(initial, true); await useStore.getState().init();
  expect(useStore.getState().settings).toBe(settings);
  oldHandler({ ...settings, fontSize: 28 }); completeOld({ ...settings, fontSize: 30 });
  const lateUnlisten = vi.fn(); finishOldListener(lateUnlisten); await vi.advanceTimersByTimeAsync(0);
  expect(lateUnlisten).toHaveBeenCalledOnce();
  expect(useStore.getState().settings).toBe(settings);
  expect(useStore.getState().initializationStatus).toBe("ready");
});

it("keeps live subscriptions when a persisted pagehide retains the same document", async () => {
  let sessionHandler!: (snapshot: SessionStateEvent) => void;
  const unlisten = vi.fn();
  sources.listenSession.mockImplementationOnce(async (handler) => { sessionHandler = handler; return unlisten; });
  await useStore.getState().init();
  window.dispatchEvent(new PageTransitionEvent("pagehide", { persisted: true }));
  sessionHandler({ ...session, isPaused: true });
  expect(useStore.getState().session.isPaused).toBe(true);
  expect(unlisten).not.toHaveBeenCalled();
  expect(useStore.getState().initialized).toBe(true);
});
