import { afterEach, describe, expect, it, vi } from "vitest";
import type { SettingsSnapshot } from "./types";
import {
  SnapshotStreamBootstrap,
  mergeSettingsSnapshot,
  SettingsSaveCoordinator,
  SnapshotResponseGate,
  SNAPSHOT_STEP_TIMEOUT_MS,
} from "./settingsState";

afterEach(() => vi.useRealTimers());

const SETTINGS: SettingsSnapshot = {
  profiles: [
    {
      id: "alibaba-default",
      name: "Alibaba Cloud",
      provider: "alibabaCloud",
      credentialState: "present",
    },
  ],
  activeProfileId: "alibaba-default",
  sourceLanguage: "auto",
  targetLanguage: "zh",
  translationMode: "lowLatency",
  fontSize: 18,
  subtitleBackgroundOpacity: 80,
  subtitleColor: "white",
  subtitleAlignment: "center",
  subtitleDisplayMode: "translation",
  showSubtitleDividers: false,
  pulseAnimation: null,
  pulseStyle: "ribbon",
  subtitleAnimation: null,
  subtitleBlendsWithBackground: false,
  isOverlayLocked: false,
  uiLanguage: null,
  retainSessionHistory: false,
  recordSessionAudio: false, audioInput: "system",
  windowsAudioSource: "",
  systemAudioTarget: { kind: "system" },
  showInDock: false,
  networkProxy: { mode: "system", url: null },
};

describe("mergeSettingsSnapshot", () => {
  it("preserves input choices and resets recording only when the actual source changes", () => {
    const recording = { ...SETTINGS, recordSessionAudio: true };
    const changed = mergeSettingsSnapshot(recording, { audioInput: "microphone", recordSessionAudio: true });
    expect(changed).toMatchObject({ audioInput: "microphone", recordSessionAudio: false });
    expect(mergeSettingsSnapshot(changed, { fontSize: 20 }).audioInput).toBe("microphone");
    const optedIn = mergeSettingsSnapshot(changed, { recordSessionAudio: true });
    expect(mergeSettingsSnapshot(optedIn, { audioInput: "microphone" }).recordSessionAudio).toBe(true);
    expect(mergeSettingsSnapshot(optedIn, { audioInput: "system" }).recordSessionAudio).toBe(false);
    expect(mergeSettingsSnapshot(recording, { audioInput: "both", recordSessionAudio: true }).recordSessionAudio).toBe(false);
    expect(mergeSettingsSnapshot({ ...recording, audioInput: "both" }, { audioInput: "microphone" }).recordSessionAudio).toBe(false);
    expect(mergeSettingsSnapshot({ ...recording, audioInput: "both" }, { audioInput: "system" }).recordSessionAudio).toBe(false);
    expect(mergeSettingsSnapshot(recording, { audioInput: "system" }).recordSessionAudio).toBe(true);
  });
  it("normalizes the proxy route while preserving it across unrelated saves", () => {
    const custom = mergeSettingsSnapshot(SETTINGS, { networkProxy: { mode: "custom", url: "socks5h://127.0.0.1" } });
    expect(custom.networkProxy).toEqual({ mode: "custom", url: "socks5h://127.0.0.1:1080" });
    expect(mergeSettingsSnapshot(custom, { fontSize: 20 }).networkProxy).toEqual(custom.networkProxy);
    expect(mergeSettingsSnapshot(custom, { networkProxy: { mode: "direct", url: "discarded" } }).networkProxy).toEqual({ mode: "direct", url: null });
    expect(mergeSettingsSnapshot(custom, { networkProxy: { mode: "custom", url: "http://user:synthetic-secret@127.0.0.1" } }).networkProxy).toEqual(custom.networkProxy);
  });
  it("keeps a divider choice through unrelated saves and permits disabling it", () => {
    expect(mergeSettingsSnapshot(SETTINGS, {}).showIntermediateSubtitles).toBe(true);
    const interimOff = mergeSettingsSnapshot(SETTINGS, { showIntermediateSubtitles: false });
    expect(mergeSettingsSnapshot(interimOff, { fontSize: 20 }).showIntermediateSubtitles).toBe(false);
    expect(mergeSettingsSnapshot(interimOff, { showIntermediateSubtitles: true }).showIntermediateSubtitles).toBe(true);
    const enabled = mergeSettingsSnapshot(SETTINGS, { showSubtitleDividers: true });
    expect(mergeSettingsSnapshot(enabled, { fontSize: 20 }).showSubtitleDividers).toBe(true);
    expect(mergeSettingsSnapshot(enabled, { showSubtitleDividers: false }).showSubtitleDividers).toBe(false);
    expect(enabled.sourceLanguage).toBe(SETTINGS.sourceLanguage);
    expect(enabled.translationMode).toBe(SETTINGS.translationMode);
  });
  it("keeps a Dock choice through unrelated settings and allows explicitly hiding again", () => {
    const enabled = mergeSettingsSnapshot(SETTINGS, { showInDock: true });
    expect(mergeSettingsSnapshot(enabled, { uiLanguage: "ja" }).showInDock).toBe(true);
    expect(mergeSettingsSnapshot(enabled, { showInDock: false }).showInDock).toBe(false);
  });

  it("defaults legacy time display off and preserves either choice across unrelated saves", () => {
    expect(mergeSettingsSnapshot(SETTINGS, {}).showSubtitleTimestamps).toBe(false);
    const enabled = mergeSettingsSnapshot(SETTINGS, { showSubtitleTimestamps: true });
    expect(mergeSettingsSnapshot(enabled, { fontSize: 20 }).showSubtitleTimestamps).toBe(true);
    const disabled = mergeSettingsSnapshot(enabled, { showSubtitleTimestamps: false });
    expect(mergeSettingsSnapshot(disabled, { audioInput: "both" }).showSubtitleTimestamps).toBe(false);
    expect(enabled.translationMode).toBe(SETTINGS.translationMode);
  });

  it("changes pulse style without changing explicit motion or unrelated choices", () => {
    const previous = { ...SETTINGS, pulseAnimation: false, subtitleAnimation: true, fontSize: 19 };
    const changed = mergeSettingsSnapshot(previous, { pulseStyle: "ribbon" });
    expect(changed).toMatchObject({ pulseStyle: "ribbon", pulseAnimation: false, subtitleAnimation: true, fontSize: 19 });
    expect(mergeSettingsSnapshot(changed, { fontSize: 20 }).pulseStyle).toBe("ribbon");
  });
  it("keeps an optimistic Windows source choice without changing pulse preferences", () => {
    const changed = mergeSettingsSnapshot({ ...SETTINGS, pulseStyle: "ribbon", pulseAnimation: false }, { windowsAudioSource: "synthetic-render-endpoint" });
    expect(changed).toMatchObject({ windowsAudioSource: "synthetic-render-endpoint", pulseStyle: "ribbon", pulseAnimation: false });
    expect(mergeSettingsSnapshot(changed, { fontSize: 20 }).windowsAudioSource).toBe("synthetic-render-endpoint");
  });
  it("merges runtime-safe subtitle presentation preferences", () => {
    expect(
      mergeSettingsSnapshot(SETTINGS, {
        subtitleColor: "#123456",
        subtitleAlignment: "right",
        subtitleDisplayMode: "bilingual",
        showSubtitleDividers: true,
        subtitleAnimation: true,
        pulseAnimation: false,
        subtitleBlendsWithBackground: true,
      }),
    ).toMatchObject({
      subtitleColor: "#123456",
      subtitleAlignment: "right",
      subtitleDisplayMode: "bilingual",
      showSubtitleDividers: true,
      subtitleAnimation: true,
      pulseAnimation: false,
      subtitleBlendsWithBackground: true,
    });
  });
});

describe("SettingsSaveCoordinator", () => {
  it("rolls the latest optimistic draft back when persistence fails", async () => {
    const coordinator = new SettingsSaveCoordinator();
    let current = SETTINGS;

    await expect(
      coordinator.save(
        current,
        { fontSize: 20 },
        async () => {
          throw new Error("save failed");
        },
        (settings) => {
          current = settings;
        },
      ),
    ).rejects.toThrow("save failed");

    expect(current.fontSize).toBe(18);
  });

  it("does not let an older failure roll back a newer successful save", async () => {
    const coordinator = new SettingsSaveCoordinator();
    let current = SETTINGS;
    let rejectFirst: ((error: Error) => void) | undefined;

    const first = coordinator.save(
      current,
      { fontSize: 19 },
      () =>
        new Promise<SettingsSnapshot>((_resolve, reject) => {
          rejectFirst = reject;
        }),
      (settings) => {
        current = settings;
      },
    );
    const second = coordinator.save(
      current,
      { fontSize: 20 },
      async () => ({ ...SETTINGS, fontSize: 20 }),
      (settings) => {
        current = settings;
      },
    );

    await Promise.resolve();
    rejectFirst?.(new Error("stale failure"));
    await expect(first).rejects.toThrow("stale failure");
    await second;

    expect(current.fontSize).toBe(20);
  });

  it("serializes persistence so the backend cannot finish saves out of order", async () => {
    const coordinator = new SettingsSaveCoordinator();
    let current = SETTINGS;
    let persistedFontSize = SETTINGS.fontSize;
    let finishFirst: (() => void) | undefined;
    let secondStarted = false;

    const persist = (draft: { fontSize?: number }) => {
      if (draft.fontSize === 19) {
        return new Promise<SettingsSnapshot>((resolve) => {
          finishFirst = () => {
            persistedFontSize = 19;
            resolve({ ...SETTINGS, fontSize: 19 });
          };
        });
      }
      secondStarted = true;
      persistedFontSize = 20;
      return Promise.resolve({ ...SETTINGS, fontSize: 20 });
    };

    const first = coordinator.save(
      current,
      { fontSize: 19 },
      persist,
      (settings) => {
        current = settings;
      },
    );
    const second = coordinator.save(
      current,
      { fontSize: 20 },
      persist,
      (settings) => {
        current = settings;
      },
    );

    await Promise.resolve();
    expect(secondStarted).toBe(false);
    expect(current.fontSize).toBe(20);

    finishFirst?.();
    await first;
    await second;

    expect(secondStarted).toBe(true);
    expect(persistedFontSize).toBe(20);
    expect(current.fontSize).toBe(20);
  });

  it("rolls a newer failed save back to the last confirmed snapshot", async () => {
    const coordinator = new SettingsSaveCoordinator();
    let current = SETTINGS;

    const first = coordinator.save(
      current,
      { fontSize: 19 },
      async () => ({ ...SETTINGS, fontSize: 19 }),
      (settings) => {
        current = settings;
      },
    );
    const second = coordinator.save(
      current,
      { fontSize: 20 },
      async () => {
        throw new Error("latest save failed");
      },
      (settings) => {
        current = settings;
      },
    );

    await first;
    await expect(second).rejects.toThrow("latest save failed");

    expect(current.fontSize).toBe(19);
  });

  it("ignores a pending response after an external snapshot", async () => {
    const coordinator = new SettingsSaveCoordinator();
    let current = SETTINGS;
    let resolveSave: ((snapshot: SettingsSnapshot) => void) | undefined;
    const pending = coordinator.save(
      current,
      { fontSize: 19 },
      () =>
        new Promise<SettingsSnapshot>((resolve) => {
          resolveSave = resolve;
        }),
      (settings) => {
        current = settings;
      },
    );

    await Promise.resolve();
    coordinator.invalidate();
    current = { ...SETTINGS, fontSize: 20 };
    resolveSave?.({ ...SETTINGS, fontSize: 19 });
    await pending;

    expect(current.fontSize).toBe(20);
  });
});

describe("SnapshotStreamBootstrap", () => {
  function deferred<Value>() {
    let resolve!: (value: Value) => void;
    let reject!: (error: unknown) => void;
    const promise = new Promise<Value>((yes, no) => { resolve = yes; reject = no; });
    return { promise, resolve, reject };
  }

  function fixture() {
    let settingsHandler!: (value: string) => void;
    let sessionHandler!: (value: string) => void;
    const unlistenSettings = vi.fn();
    const unlistenSession = vi.fn();
    const sources = {
      listenSettings: vi.fn(async (handler: (value: string) => void): Promise<() => void> => { settingsHandler = handler; return unlistenSettings; }),
      listenSession: vi.fn(async (handler: (value: string) => void): Promise<() => void> => { sessionHandler = handler; return unlistenSession; }),
      getSettings: vi.fn(async () => "boot-settings"),
      getSession: vi.fn(async () => "idle"),
    };
    const consumers = { applySettings: vi.fn(), applySession: vi.fn(), onReady: vi.fn() };
    const bootstrap = new SnapshotStreamBootstrap(sources, consumers);
    return { sources, consumers, bootstrap, unlistenSettings, unlistenSession,
      settings: (value: string) => settingsHandler(value), session: (value: string) => sessionHandler(value) };
  }

  it("keeps backend listening events alive after an overnight Keychain timeout and hydrates the late original read", async () => {
    vi.useFakeTimers();
    const f = fixture();
    const read = deferred<string>();
    f.sources.getSettings.mockReturnValue(read.promise);
    const initialization = f.bootstrap.initialize();
    const expired = expect(initialization).rejects.toThrow("snapshot-step-timeout");
    await vi.advanceTimersByTimeAsync(0);
    expect(f.consumers.applySession).toHaveBeenCalledExactlyOnceWith("idle");
    await vi.advanceTimersByTimeAsync(SNAPSHOT_STEP_TIMEOUT_MS);
    await expired;
    expect(f.unlistenSession).not.toHaveBeenCalled();
    expect(f.unlistenSettings).not.toHaveBeenCalled();
    f.session("listening"); f.session("confirmed-subtitle-snapshot");
    expect(f.consumers.applySession.mock.calls.flat()).toEqual(["idle", "listening", "confirmed-subtitle-snapshot"]);
    await vi.advanceTimersByTimeAsync(8 * 60 * 60 * 1000);
    expect(f.sources.getSettings).toHaveBeenCalledOnce();
    read.resolve("persisted-display-and-proxy-preferences");
    await vi.advanceTimersByTimeAsync(0);
    expect(f.consumers.applySettings).toHaveBeenCalledExactlyOnceWith("persisted-display-and-proxy-preferences");
    expect(f.bootstrap.ready).toBe(true);
    expect(f.consumers.onReady).toHaveBeenCalledOnce();
    f.bootstrap.dispose();
    expect(f.unlistenSession).toHaveBeenCalledOnce();
    expect(f.unlistenSettings).toHaveBeenCalledOnce();
  });

  it("reuses a pending native read on explicit retry and lets a newer event beat its late response", async () => {
    vi.useFakeTimers();
    const f = fixture();
    const read = deferred<string>();
    f.sources.getSettings.mockReturnValue(read.promise);
    const expired = expect(f.bootstrap.initialize()).rejects.toThrow("snapshot-step-timeout");
    await vi.advanceTimersByTimeAsync(SNAPSHOT_STEP_TIMEOUT_MS); await expired;
    const retry = f.bootstrap.initialize();
    await vi.advanceTimersByTimeAsync(0);
    f.settings("new-settings-event");
    expect(f.bootstrap.ready).toBe(true);
    read.resolve("old-settings-response");
    await retry;
    expect(f.consumers.applySettings).toHaveBeenCalledExactlyOnceWith("new-settings-event");
    expect(f.sources.getSettings).toHaveBeenCalledOnce();
    expect(f.sources.getSession).toHaveBeenCalledOnce();
    expect(f.sources.listenSettings).toHaveBeenCalledOnce();
    expect(f.sources.listenSession).toHaveBeenCalledOnce();
    expect(f.consumers.onReady).toHaveBeenCalledOnce();
    f.bootstrap.dispose();
  });

  it("retries a rejected settings read without removing or duplicating the healthy session stream", async () => {
    const f = fixture();
    f.sources.getSettings.mockRejectedValueOnce("synthetic-native-error");
    await expect(f.bootstrap.initialize()).rejects.toThrow("boot-snapshot-unavailable");
    f.session("listening");
    expect(f.consumers.applySession).toHaveBeenLastCalledWith("listening");
    await f.bootstrap.initialize();
    expect(f.bootstrap.ready).toBe(true);
    expect(f.sources.getSettings).toHaveBeenCalledTimes(2);
    expect(f.sources.getSession).toHaveBeenCalledOnce();
    expect(f.sources.listenSettings).toHaveBeenCalledOnce();
    expect(f.sources.listenSession).toHaveBeenCalledOnce();
    expect(f.unlistenSession).not.toHaveBeenCalled();
    f.bootstrap.dispose();
  });

  it("retries only a failed listener and retains the other hydrated stream", async () => {
    const f = fixture();
    f.sources.listenSession.mockRejectedValueOnce(new Error("synthetic-listener-error"));
    await expect(f.bootstrap.initialize()).rejects.toThrow("snapshot-listener-unavailable");
    expect(f.sources.getSession).not.toHaveBeenCalled();
    expect(f.unlistenSettings).not.toHaveBeenCalled();
    await f.bootstrap.initialize();
    expect(f.bootstrap.ready).toBe(true);
    expect(f.sources.listenSettings).toHaveBeenCalledOnce();
    expect(f.sources.getSettings).toHaveBeenCalledOnce();
    expect(f.sources.listenSession).toHaveBeenCalledTimes(2);
    f.bootstrap.dispose();
  });

  it("reuses a listener pending past its deadline and accepts its later completion", async () => {
    vi.useFakeTimers();
    const f = fixture();
    const listener = deferred<() => void>();
    f.sources.listenSettings.mockReturnValue(listener.promise);
    const expired = expect(f.bootstrap.initialize()).rejects.toThrow("snapshot-step-timeout");
    await vi.advanceTimersByTimeAsync(SNAPSHOT_STEP_TIMEOUT_MS); await expired;
    const retry = f.bootstrap.initialize();
    await vi.advanceTimersByTimeAsync(0);
    listener.resolve(f.unlistenSettings);
    await retry;
    expect(f.bootstrap.ready).toBe(true);
    expect(f.sources.listenSettings).toHaveBeenCalledOnce();
    expect(f.sources.getSettings).toHaveBeenCalledOnce();
    expect(f.unlistenSettings).not.toHaveBeenCalled();
    f.bootstrap.dispose();
  });

  it("hydrates after a listener acknowledges past its deadline without requiring a retry or event", async () => {
    vi.useFakeTimers();
    const f = fixture();
    const listener = deferred<() => void>();
    f.sources.listenSettings.mockReturnValue(listener.promise);
    const expired = expect(f.bootstrap.initialize()).rejects.toThrow("snapshot-step-timeout");
    await vi.advanceTimersByTimeAsync(SNAPSHOT_STEP_TIMEOUT_MS); await expired;
    expect(f.sources.getSettings).not.toHaveBeenCalled();
    listener.resolve(f.unlistenSettings); await vi.advanceTimersByTimeAsync(0);
    expect(f.sources.getSettings).toHaveBeenCalledOnce();
    expect(f.bootstrap.ready).toBe(true);
    expect(f.consumers.onReady).toHaveBeenCalledOnce();
    expect(f.consumers.applySettings).toHaveBeenCalledExactlyOnceWith("boot-settings");
    f.bootstrap.dispose();
  });

  it("publishes events immediately and ignores stale boot snapshots for each stream", async () => {
    vi.useFakeTimers();
    const f = fixture();
    const settings = deferred<string>(); const session = deferred<string>();
    f.sources.getSettings.mockReturnValue(settings.promise);
    f.sources.getSession.mockReturnValue(session.promise);
    const initialization = f.bootstrap.initialize();
    await vi.advanceTimersByTimeAsync(0);
    f.settings("new-settings"); f.session("connecting"); f.session("listening");
    expect(f.consumers.applySession.mock.calls.flat()).toEqual(["connecting", "listening"]);
    settings.resolve("stale-settings"); session.resolve("idle"); await initialization;
    expect(f.consumers.applySettings.mock.calls.flat()).toEqual(["new-settings"]);
    expect(f.consumers.applySession.mock.calls.flat()).toEqual(["connecting", "listening"]);
    f.bootstrap.dispose();
  });

  it("accepts an event before the listener acknowledgement without a redundant snapshot read", async () => {
    const f = fixture();
    f.sources.listenSettings.mockImplementationOnce(async (handler) => {
      handler("new-settings-before-ack"); return f.unlistenSettings;
    });
    await f.bootstrap.initialize();
    expect(f.sources.getSettings).not.toHaveBeenCalled();
    expect(f.consumers.applySettings).toHaveBeenCalledExactlyOnceWith("new-settings-before-ack");
    expect(f.bootstrap.ready).toBe(true);
    f.bootstrap.dispose();
  });

  it("disposes pending listeners and reads safely without leaking late results into a replacement", async () => {
    vi.useFakeTimers();
    const f = fixture();
    const listener = deferred<() => void>(); const session = deferred<string>();
    f.sources.listenSettings.mockReturnValue(listener.promise);
    f.sources.getSession.mockReturnValue(session.promise);
    const initialization = f.bootstrap.initialize();
    await vi.advanceTimersByTimeAsync(0);
    f.bootstrap.dispose(); f.bootstrap.dispose();
    await initialization;
    expect(vi.getTimerCount()).toBe(0);
    f.session("stale-event"); session.resolve("stale-snapshot"); listener.resolve(f.unlistenSettings);
    await vi.advanceTimersByTimeAsync(0);
    expect(f.unlistenSettings).toHaveBeenCalledOnce();
    expect(f.unlistenSession).toHaveBeenCalledOnce();
    expect(f.sources.getSettings).not.toHaveBeenCalled();
    expect(f.consumers.applySession).not.toHaveBeenCalled();
    expect(f.consumers.onReady).not.toHaveBeenCalled();
    const replacement = fixture(); await replacement.bootstrap.initialize();
    expect(replacement.bootstrap.ready).toBe(true); replacement.bootstrap.dispose();
  });

  it("disposes before native calls begin and cleans both streams even if one unlisten throws", async () => {
    const early = fixture();
    const initialization = early.bootstrap.initialize(); early.bootstrap.dispose(); await initialization;
    expect(early.sources.listenSettings).not.toHaveBeenCalled();
    expect(early.sources.listenSession).not.toHaveBeenCalled();
    const f = fixture(); await f.bootstrap.initialize();
    f.unlistenSettings.mockImplementation(() => { throw new Error("synthetic-cleanup-error"); });
    f.bootstrap.dispose(); f.bootstrap.dispose();
    f.settings("stale-settings"); f.session("stale-session");
    expect(f.unlistenSession).toHaveBeenCalledOnce();
    expect(f.unlistenSettings).toHaveBeenCalledOnce();
    expect(f.consumers.applySettings).toHaveBeenCalledExactlyOnceWith("boot-settings");
    expect(f.consumers.applySession).toHaveBeenCalledExactlyOnceWith("idle");
  });
});

describe("SnapshotResponseGate", () => {
  it("rejects an older response after a newer settings event", () => {
    const gate = new SnapshotResponseGate();
    const oldResponse = gate.capture();

    gate.advance();

    expect(gate.applyIfCurrent(oldResponse)).toBe(false);
    expect(gate.applyIfCurrent(gate.capture())).toBe(true);
  });
});

it("updates background opacity immediately, preserves zero and keeps it through unrelated saves", () => {
  const transparent = mergeSettingsSnapshot(SETTINGS, { subtitleBackgroundOpacity: 0 });
  expect(transparent.subtitleBackgroundOpacity).toBe(0);
  expect(mergeSettingsSnapshot(transparent, { fontSize: 20 }).subtitleBackgroundOpacity).toBe(0);
  expect(mergeSettingsSnapshot(SETTINGS, { subtitleBackgroundOpacity: 35 }).subtitleBackgroundOpacity).toBe(35);
});

it("application target changes require a new recording opt-in, including combined drafts", () => {
  const recording = { ...SETTINGS, recordSessionAudio: true };
  const target = { kind: "application" as const, id: "com.example.player", name: "Player" };
  const changed = mergeSettingsSnapshot(recording, { systemAudioTarget: target, recordSessionAudio: true });
  expect(changed.recordSessionAudio).toBe(false);
  const optedIn = mergeSettingsSnapshot(changed, { recordSessionAudio: true });
  expect(mergeSettingsSnapshot(optedIn, { systemAudioTarget: target }).recordSessionAudio).toBe(true);
  expect(mergeSettingsSnapshot(optedIn, { systemAudioTarget: { kind: "system" } }).recordSessionAudio).toBe(false);
});
