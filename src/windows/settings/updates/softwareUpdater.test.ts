import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createTauriSoftwareUpdater } from "./softwareUpdater";

const native = vi.hoisted(() => ({
  getVersion: vi.fn(),
  check: vi.fn(),
  relaunch: vi.fn(),
  update: {
    version: "1.4.0",
    body: "Release notes",
    download: vi.fn(),
    install: vi.fn(),
    close: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/app", () => ({ getVersion: native.getVersion }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: native.relaunch }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: native.check }));

beforeEach(() => {
  vi.resetAllMocks();
  native.getVersion.mockResolvedValue("1.3.11");
  native.check.mockResolvedValue(native.update);
  native.update.install.mockResolvedValue(undefined);
});

afterEach(() => vi.unstubAllGlobals());

describe("native updater installation", () => {
  it("asks the Windows installer to reopen Mimi after installation and retries", async () => {
    vi.stubGlobal("navigator", { userAgent: "Windows NT 10.0; Win64; x64" });
    const updater = await createTauriSoftwareUpdater();
    const candidate = await updater.check();
    expect(updater.platform).toBe("windows");
    expect(candidate).not.toBeNull();

    native.update.install.mockRejectedValueOnce(
      new Error("installer unavailable"),
    );
    await expect(candidate!.install()).rejects.toThrow("installer unavailable");
    await candidate!.install();

    expect(native.update.install.mock.calls).toEqual([
      [{ restartAfterInstall: true }],
      [{ restartAfterInstall: true }],
    ]);
    expect(native.relaunch).not.toHaveBeenCalled();
  });

  it("keeps macOS relaunch separate from installing the verified update", async () => {
    vi.stubGlobal("navigator", { userAgent: "Macintosh; Intel Mac OS X" });
    const updater = await createTauriSoftwareUpdater();
    const candidate = await updater.check();
    await candidate!.install();

    expect(native.update.install).toHaveBeenCalledWith({
      restartAfterInstall: false,
    });
    expect(native.relaunch).not.toHaveBeenCalled();
    await updater.relaunch();
    expect(native.relaunch).toHaveBeenCalledOnce();
  });

  it("forwards real download progress without starting installation", async () => {
    const updater = await createTauriSoftwareUpdater();
    const candidate = await updater.check();
    const onEvent = vi.fn();
    native.update.download.mockImplementation(async (callback) => {
      callback({ event: "Started", data: { contentLength: 200 } });
      callback({ event: "Progress", data: { chunkLength: 120 } });
    });
    await candidate!.download(onEvent);

    expect(onEvent.mock.calls).toEqual([
      [{ event: "Started", data: { contentLength: 200 } }],
      [{ event: "Progress", data: { chunkLength: 120 } }],
    ]);
    expect(native.update.install).not.toHaveBeenCalled();
    expect(native.check).toHaveBeenCalledWith({ timeout: 15_000 });
    await candidate!.close();
    expect(native.update.close).toHaveBeenCalledOnce();
  });
});
