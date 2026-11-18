import { describe, expect, it, vi } from "vitest";
import {
  AUTOMATIC_UPDATE_CHECK_INTERVAL_MS,
  createSoftwareUpdateSession,
} from "./softwareUpdateSession";
import type {
  SoftwareUpdater,
  UpdateCandidate,
  UpdateDownloadEvent,
} from "./softwareUpdater";

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((accept, fail) => {
    resolve = accept;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function fixture(platform: SoftwareUpdater["platform"] = "other") {
  const candidate = {
    version: "2.0.0",
    notes: "Release notes",
    download: vi.fn<UpdateCandidate["download"]>().mockResolvedValue(undefined),
    install: vi.fn<UpdateCandidate["install"]>().mockResolvedValue(undefined),
    close: vi.fn<UpdateCandidate["close"]>().mockResolvedValue(undefined),
  };
  const updater = {
    currentVersion: "1.0.0",
    platform,
    check: vi.fn<SoftwareUpdater["check"]>().mockResolvedValue(candidate),
    relaunch: vi.fn<SoftwareUpdater["relaunch"]>().mockResolvedValue(undefined),
  };
  const createEnvironment = vi.fn(async () => ({
    kind: "installed" as const,
    updater,
  }));
  return { candidate, updater, createEnvironment };
}

describe("software update session", () => {
  it("deduplicates initialization and checks across subscriptions and repeated entry", async () => {
    const { updater, createEnvironment } = fixture();
    const response = deferred<UpdateCandidate | null>();
    updater.check.mockReturnValue(response.promise);
    const session = createSoftwareUpdateSession({ createEnvironment });
    const listener = vi.fn();
    const unsubscribe = session.subscribe(listener);

    const first = session.enter();
    expect(session.getSnapshot().state.kind).toBe("checking");
    unsubscribe();
    const remounted = vi.fn();
    session.subscribe(remounted);
    const second = session.enter();
    await Promise.resolve();
    await Promise.resolve();
    expect(createEnvironment).toHaveBeenCalledOnce();
    expect(updater.check).toHaveBeenCalledOnce();
    response.resolve(null);
    await Promise.all([first, second]);

    expect(listener).toHaveBeenCalledOnce();
    expect(remounted).toHaveBeenCalled();
    expect(session.getSnapshot()).toMatchObject({
      environment: "installed",
      currentVersion: "1.0.0",
      state: { kind: "noUpdate" },
    });
    const stable = session.getSnapshot();
    expect(session.getSnapshot()).toBe(stable);
    await session.enter();
    expect(updater.check).toHaveBeenCalledOnce();
  });

  it("cools down automatic checks for 15 minutes while manual checks bypass it", async () => {
    const { updater, createEnvironment } = fixture();
    updater.check.mockResolvedValue(null);
    let time = 0;
    const session = createSoftwareUpdateSession({ createEnvironment, now: () => time });

    expect(await session.enter()).toBeUndefined();
    time = AUTOMATIC_UPDATE_CHECK_INTERVAL_MS - 1;
    await session.enter();
    expect(updater.check).toHaveBeenCalledTimes(1);
    time += 1;
    await session.enter();
    expect(updater.check).toHaveBeenCalledTimes(2);
    expect(await session.performAction()).toBe("noUpdate");
    expect(updater.check).toHaveBeenCalledTimes(3);
    expect(createEnvironment).toHaveBeenCalledOnce();
  });

  it("includes failed checks in the cooldown and allows explicit retry", async () => {
    const { updater, createEnvironment } = fixture();
    updater.check.mockRejectedValueOnce(new Error("private response details"));
    const session = createSoftwareUpdateSession({ createEnvironment, now: () => 0 });
    expect(await session.enter()).toBeUndefined();
    expect(session.getSnapshot().state).toEqual({ kind: "checkError", recovery: "idle" });
    await session.enter();
    expect(updater.check).toHaveBeenCalledOnce();
    await session.performAction();
    expect(updater.check).toHaveBeenCalledTimes(2);
    expect(session.getSnapshot().state.kind).toBe("available");
  });

  it("retries failed native initialization manually without disabling its action", async () => {
    const { updater, createEnvironment } = fixture();
    createEnvironment.mockRejectedValueOnce(new Error("mode probe unavailable"));
    const session = createSoftwareUpdateSession({ createEnvironment, now: () => 0 });
    await session.enter();
    expect(session.getSnapshot()).toEqual({
      environment: "uninitialized",
      state: { kind: "checkError", recovery: "idle" },
    });
    await session.enter();
    expect(createEnvironment).toHaveBeenCalledOnce();
    await session.performAction();
    expect(createEnvironment).toHaveBeenCalledTimes(2);
    expect(updater.check).toHaveBeenCalledOnce();
    expect(session.getSnapshot().state.kind).toBe("available");
  });

  it.each(["portable", "linuxPackage"] as const)(
    "keeps %s distributions outside the installer path",
    async (kind) => {
      const createEnvironment = vi.fn(async () => ({ kind, currentVersion: "1.0.0" }));
      const session = createSoftwareUpdateSession({ createEnvironment });
      await session.enter();
      await session.performAction();
      await session.enter();
      expect(createEnvironment).toHaveBeenCalledOnce();
      expect(session.getSnapshot()).toEqual({
        environment: kind,
        currentVersion: "1.0.0",
        state: { kind: "idle" },
      });
    },
  );

  it("preserves an available candidate on later entries without downloading it", async () => {
    const { candidate, updater, createEnvironment } = fixture();
    let time = 0;
    const session = createSoftwareUpdateSession({ createEnvironment, now: () => time });
    await session.enter();
    time = AUTOMATIC_UPDATE_CHECK_INTERVAL_MS * 2;
    const previous = session.getSnapshot();
    await session.enter();
    expect(session.getSnapshot()).toBe(previous);
    expect(updater.check).toHaveBeenCalledOnce();
    expect(candidate.download).not.toHaveBeenCalled();
    expect(candidate.install).not.toHaveBeenCalled();
    expect(candidate.close).not.toHaveBeenCalled();
  });

  it("retains progress across remounts and requires explicit verified download, install, and relaunch", async () => {
    const { candidate, updater, createEnvironment } = fixture();
    const download = deferred<void>();
    let report!: (event: UpdateDownloadEvent) => void;
    candidate.download.mockImplementation((onEvent) => {
      report = onEvent;
      return download.promise;
    });
    const session = createSoftwareUpdateSession({ createEnvironment });
    await session.enter();
    const unsubscribe = session.subscribe(vi.fn());
    const action = session.performAction();
    await Promise.resolve();
    report({ event: "Started", data: { contentLength: 100 } });
    report({ event: "Progress", data: { chunkLength: 100 } });
    report({ event: "Finished" });
    expect(session.getSnapshot().state).toMatchObject({
      kind: "downloading", downloadedBytes: 100, totalBytes: 100, transferComplete: true,
    });
    unsubscribe();
    const remount = session.enter();
    expect(await session.performAction()).toBeUndefined();
    expect(candidate.download).toHaveBeenCalledOnce();
    expect(candidate.install).not.toHaveBeenCalled();
    download.resolve();
    await Promise.all([action, remount]);
    expect(session.getSnapshot().state.kind).toBe("downloaded");
    await session.enter();
    expect(candidate.close).not.toHaveBeenCalled();
    expect(updater.check).toHaveBeenCalledOnce();
    expect(candidate.install).not.toHaveBeenCalled();

    const install = session.performAction();
    expect(session.getSnapshot().state.kind).toBe("installing");
    await install;
    expect(session.getSnapshot().state.kind).toBe("restartReady");
    expect(candidate.install).toHaveBeenCalledOnce();
    expect(updater.relaunch).not.toHaveBeenCalled();
    expect(await session.performAction()).toBe("restartRequested");
    expect(session.getSnapshot().state.kind).toBe("restartRequested");
    expect(updater.relaunch).toHaveBeenCalledOnce();
  });

  it("does not offer installation when signature verification fails after transfer finishes", async () => {
    const { candidate, createEnvironment } = fixture();
    candidate.download.mockImplementationOnce(async (onEvent) => {
      onEvent({ event: "Finished" });
      throw new Error("signature verification failed");
    });
    const session = createSoftwareUpdateSession({ createEnvironment });
    await session.enter();
    await session.performAction();
    expect(session.getSnapshot().state.kind).toBe("downloadError");
    expect(candidate.install).not.toHaveBeenCalled();
    await session.performAction();
    expect(candidate.download).toHaveBeenCalledTimes(2);
    expect(session.getSnapshot().state.kind).toBe("downloaded");
    expect(candidate.install).not.toHaveBeenCalled();
  });

  it("preserves the verified candidate for install and restart retries", async () => {
    const { candidate, updater, createEnvironment } = fixture();
    candidate.install.mockRejectedValueOnce(new Error("installer unavailable"));
    updater.relaunch.mockRejectedValueOnce(new Error("relaunch unavailable"));
    const session = createSoftwareUpdateSession({ createEnvironment });
    await session.enter();
    await session.performAction();
    await session.performAction();
    expect(session.getSnapshot().state.kind).toBe("installError");
    session.setRecoveryStatus("opening");
    expect(session.getSnapshot().state).toMatchObject({ kind: "installError", recovery: "opening" });
    session.setRecoveryStatus("idle");
    await session.enter();
    await session.performAction();
    expect(session.getSnapshot().state.kind).toBe("restartReady");
    await session.performAction();
    expect(session.getSnapshot().state.kind).toBe("restartError");
    expect(await session.performAction()).toBe("restartRequested");
    expect(candidate.download).toHaveBeenCalledOnce();
    expect(candidate.install).toHaveBeenCalledTimes(2);
    expect(updater.relaunch).toHaveBeenCalledTimes(2);
  });

  it("lets the Windows installer own restart after an explicit installation", async () => {
    const { candidate, updater, createEnvironment } = fixture("windows");
    const session = createSoftwareUpdateSession({ createEnvironment });
    await session.enter();
    await session.performAction();
    await session.performAction();
    expect(session.getSnapshot().state.kind).toBe("windowsInstallerStarted");
    await session.performAction();
    await session.enter();
    expect(candidate.install).toHaveBeenCalledOnce();
    expect(updater.relaunch).not.toHaveBeenCalled();
  });
});
