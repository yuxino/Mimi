import { describe, expect, it } from "vitest";
import {
  applyDownloadEvent,
  downloadPercent,
  normalizeReleaseNotes,
  updateInteraction,
  withRecoveryStatus,
  type UpdateCheckState,
} from "./softwareUpdateModel";
import {
  createFixtureSoftwareUpdater,
  isWindowsUserAgent,
} from "./softwareUpdater";

const update = { version: "1.4.0", notes: "Changes" };

describe("software update interaction", () => {
  it.each<{
    state: UpdateCheckState;
    action?: "check" | "download" | "install" | "restart";
    busy: boolean;
    emphasized: boolean;
  }>([
    { state: { kind: "idle" }, action: "check", busy: false, emphasized: false },
    { state: { kind: "checking" }, busy: true, emphasized: true },
    { state: { kind: "noUpdate" }, action: "check", busy: false, emphasized: false },
    { state: { kind: "available", update }, action: "download", busy: false, emphasized: true },
    {
      state: { kind: "downloading", update, downloadedBytes: 10, totalBytes: 100 },
      busy: true,
      emphasized: true,
    },
    { state: { kind: "downloaded", update }, action: "install", busy: false, emphasized: true },
    {
      state: { kind: "restartReady", update },
      action: "restart",
      busy: false,
      emphasized: true,
    },
    {
      state: { kind: "downloadError", update, recovery: "idle" },
      action: "download",
      busy: false,
      emphasized: true,
    },
    {
      state: { kind: "installError", update, recovery: "idle" },
      action: "install",
      busy: false,
      emphasized: true,
    },
  ])("derives $state.kind button behavior", (expected) => {
    expect(updateInteraction(expected.state)).toEqual({
      action: expected.action,
      busy: expected.busy,
      emphasized: expected.emphasized,
    });
  });

  it("derives exact and indeterminate progress without inventing totals", () => {
    let known: UpdateCheckState = {
      kind: "downloading",
      update,
      downloadedBytes: 0,
    };
    known = applyDownloadEvent(known, {
      event: "Started",
      data: { contentLength: 200 },
    });
    known = applyDownloadEvent(known, {
      event: "Progress",
      data: { chunkLength: 75 },
    });
    expect(downloadPercent(known)).toBe(37);

    const unknown = applyDownloadEvent(
      { kind: "downloading", update, downloadedBytes: 12 },
      { event: "Started", data: {} },
    );
    expect(downloadPercent(unknown)).toBeUndefined();
  });

  it("keeps error retry state while changing recovery activity", () => {
    const state: UpdateCheckState = {
      kind: "downloadError",
      update,
      recovery: "idle",
    };
    expect(withRecoveryStatus(state, "opening")).toEqual({
      ...state,
      recovery: "opening",
    });
    expect(updateInteraction(withRecoveryStatus(state, "error")).action).toBe(
      "download",
    );
  });

  it("bounds release notes and detects Windows only from a Windows user agent", () => {
    expect(normalizeReleaseNotes(`  a\r\nb  `)).toBe("a\nb");
    expect(normalizeReleaseNotes("x".repeat(40_000))).toHaveLength(32_000);
    expect(isWindowsUserAgent("Windows NT 10.0")).toBe(true);
    expect(isWindowsUserAgent("Macintosh; Intel Mac OS X")).toBe(false);
  });
});

describe("controlled updater fixture", () => {
  it("emits known-size progress and completes a verified-style download", async () => {
    const updater = createFixtureSoftwareUpdater({
      contentLength: 100,
      chunks: [25, 75],
    });
    const candidate = await updater.check();
    const events: string[] = [];
    await candidate?.download((event) => events.push(event.event));
    expect(events).toEqual(["Started", "Progress", "Progress", "Finished"]);
  });

  it("covers no update, network, signature, install, and relaunch failures", async () => {
    await expect(
      createFixtureSoftwareUpdater({ updateVersion: null }).check(),
    ).resolves.toBeNull();
    await expect(
      createFixtureSoftwareUpdater({ checkError: new Error("network") }).check(),
    ).rejects.toThrow("network");

    const signatureFailure = await createFixtureSoftwareUpdater({
      downloadError: new Error("signature verification failed"),
    }).check();
    await expect(signatureFailure?.download(() => {})).rejects.toThrow(
      "signature verification failed",
    );

    const installFailure = await createFixtureSoftwareUpdater({
      installError: new Error("install failed"),
    }).check();
    await expect(installFailure?.install()).rejects.toThrow("install failed");
    await expect(
      createFixtureSoftwareUpdater({
        relaunchError: new Error("relaunch failed"),
      }).relaunch(),
    ).rejects.toThrow("relaunch failed");
  });
});
