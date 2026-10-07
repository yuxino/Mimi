import { beforeEach, expect, it, vi } from "vitest";
import { verifyUiTestConnectionReadiness } from "./uiTestConnectionReadiness";

const { appIsUiTest, testProfileConnection } = vi.hoisted(() => ({
  appIsUiTest: vi.fn(),
  testProfileConnection: vi.fn(),
}));
vi.mock("./ipc", () => ({ appIsUiTest, testProfileConnection }));

beforeEach(() => {
  vi.resetAllMocks();
  appIsUiTest.mockResolvedValue(true);
  testProfileConnection.mockResolvedValue({ credential: "present", service: "notTested", reason: null });
});

it("never probes a provider during ordinary application startup", async () => {
  appIsUiTest.mockResolvedValue(false);
  await verifyUiTestConnectionReadiness("configured");
  expect(testProfileConnection).not.toHaveBeenCalled();
});

it("accepts the real diagnostic response for an explicitly configured UI fixture", async () => {
  await expect(verifyUiTestConnectionReadiness("configured")).resolves.toBeUndefined();
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith("configured");
});

it("accepts the native missing-profile contract for a fresh empty installation", async () => {
  testProfileConnection.mockRejectedValue("profile_not_found");
  await expect(verifyUiTestConnectionReadiness("")).resolves.toBeUndefined();
  expect(testProfileConnection).toHaveBeenCalledExactlyOnceWith("");
});

it("rejects a missing configured profile", async () => {
  testProfileConnection.mockRejectedValue("profile_not_found");
  await expect(verifyUiTestConnectionReadiness("configured")).rejects.toBe("profile_not_found");
});

it("does not hide a missing native command behind empty-profile handling", async () => {
  testProfileConnection.mockRejectedValue("Command profile_test_connection not found");
  await expect(verifyUiTestConnectionReadiness("")).rejects.toBe("Command profile_test_connection not found");
});

it("rejects an unexpected successful diagnostic for an empty profile", async () => {
  await expect(verifyUiTestConnectionReadiness("")).rejects.toThrow("connection_diagnostic_smoke_failed");
});

it.each([
  { credential: "missing", service: "notTested", reason: null },
  { credential: "present", service: "available", reason: null },
  { credential: "present", service: "notTested", reason: "credentialsMissing" },
])("rejects an unexpected configured diagnostic: %j", async (response) => {
  testProfileConnection.mockResolvedValue(response);
  await expect(verifyUiTestConnectionReadiness("configured")).rejects.toThrow("connection_diagnostic_smoke_failed");
});
