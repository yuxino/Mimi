import { beforeEach, describe, expect, it, vi } from "vitest";
import { createUpdaterForEnvironment } from "./softwareUpdateEnvironment";

const mocks = vi.hoisted(() => ({
  appIsUiTest: vi.fn(),
  appIsPortable: vi.fn(),
  appIsLinuxPackage: vi.fn(),
  isWindowsUserAgent: vi.fn(),
  createTauriSoftwareUpdater: vi.fn(),
  getVersion: vi.fn(),
}));

vi.mock("../../lib/ipc", () => ({
  isTauri: true,
  setOverlayPointerCursor: vi.fn(),
  appIsUiTest: mocks.appIsUiTest,
  appIsPortable: mocks.appIsPortable,
  appIsLinuxPackage: mocks.appIsLinuxPackage,
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: mocks.getVersion }));
vi.mock("./softwareUpdater", () => ({
  isWindowsUserAgent: mocks.isWindowsUserAgent,
  createTauriSoftwareUpdater: mocks.createTauriSoftwareUpdater,
}));

beforeEach(() => {
  vi.resetAllMocks();
  mocks.appIsUiTest.mockResolvedValue(false);
  mocks.appIsLinuxPackage.mockResolvedValue(false);
  mocks.getVersion.mockResolvedValue("1.4.2");
  mocks.isWindowsUserAgent.mockReturnValue(true);
  mocks.createTauriSoftwareUpdater.mockResolvedValue({ currentVersion: "1.4.2" });
});

describe("portable update distribution", () => {
  it("routes a marked Windows ZIP to manual Releases updates", async () => {
    mocks.appIsPortable.mockResolvedValue(true);
    mocks.appIsUiTest.mockResolvedValue(true);
    expect(await createUpdaterForEnvironment()).toEqual({
      kind: "portable",
      currentVersion: "1.4.2",
    });
    expect(mocks.createTauriSoftwareUpdater).not.toHaveBeenCalled();
  });

  it("keeps the signed updater for an installed Windows copy", async () => {
    mocks.appIsPortable.mockResolvedValue(false);
    expect((await createUpdaterForEnvironment()).kind).toBe("installed");
    expect(mocks.createTauriSoftwareUpdater).toHaveBeenCalledOnce();
  });

  it("does not offer NSIS when Windows distribution detection fails", async () => {
    mocks.appIsPortable.mockRejectedValue(new Error("unavailable"));
    expect((await createUpdaterForEnvironment()).kind).toBe("portable");
    expect(mocks.createTauriSoftwareUpdater).not.toHaveBeenCalled();
  });

  it("leaves macOS on its updater path", async () => {
    mocks.isWindowsUserAgent.mockReturnValue(false);
    expect((await createUpdaterForEnvironment()).kind).toBe("installed");
    expect(mocks.appIsPortable).not.toHaveBeenCalled();
  });

  it("routes Linux packages to Releases without constructing an AppImage updater", async () => {
    mocks.isWindowsUserAgent.mockReturnValue(false);
    mocks.appIsLinuxPackage.mockResolvedValue(true);
    expect(await createUpdaterForEnvironment()).toEqual({
      kind: "linuxPackage",
      currentVersion: "1.4.2",
    });
    expect(mocks.createTauriSoftwareUpdater).not.toHaveBeenCalled();
  });

  it("keeps the signed updater for AppImage", async () => {
    mocks.isWindowsUserAgent.mockReturnValue(false);
    expect((await createUpdaterForEnvironment()).kind).toBe("installed");
    expect(mocks.createTauriSoftwareUpdater).toHaveBeenCalledOnce();
  });

  it("does not create an updater when native Linux package detection fails", async () => {
    mocks.isWindowsUserAgent.mockReturnValue(false);
    mocks.appIsLinuxPackage.mockRejectedValue(new Error("unavailable"));
    await expect(createUpdaterForEnvironment()).rejects.toThrow("unavailable");
    expect(mocks.createTauriSoftwareUpdater).not.toHaveBeenCalled();
  });
});
