import { describe, expect, it } from "vitest";
import {
  SettingsSessionActionCoordinator,
  settingsSessionControlState,
  type SettingsSessionVisibleStatus,
} from "./settingsSessionControlModel";

type SettingsSessionControlInput = Parameters<
  typeof settingsSessionControlState
>[0];

describe("settings session control", () => {
  it.each<{
    input: SettingsSessionControlInput;
    checked: boolean;
    disabled: boolean;
    canConfigure: boolean;
    visibleStatus: SettingsSessionVisibleStatus;
  }>([
    {
      input: {
        statusKind: "idle",
        isActive: false,
        isPaused: false,
        credentialState: "present",
        pendingAction: null,
      },
      checked: false,
      disabled: false,
      canConfigure: false,
      visibleStatus: "idle",
    },
    {
      input: {
        statusKind: "connecting",
        isActive: true,
        isPaused: false,
        credentialState: "present",
        pendingAction: null,
      },
      checked: true,
      disabled: true,
      canConfigure: false,
      visibleStatus: "connecting",
    },
    {
      input: {
        statusKind: "listening",
        isActive: true,
        isPaused: false,
        credentialState: "present",
        pendingAction: null,
      },
      checked: true,
      disabled: false,
      canConfigure: false,
      visibleStatus: "listening",
    },
    {
      input: {
        statusKind: "listening",
        isActive: true,
        isPaused: true,
        credentialState: "present",
        pendingAction: null,
      },
      checked: true,
      disabled: false,
      canConfigure: false,
      visibleStatus: "paused",
    },
    {
      input: {
        statusKind: "stopping",
        isActive: true,
        isPaused: false,
        credentialState: "present",
        pendingAction: null,
      },
      checked: true,
      disabled: true,
      canConfigure: false,
      visibleStatus: "stopping",
    },
    {
      input: {
        statusKind: "error",
        isActive: false,
        isPaused: false,
        credentialState: "present",
        pendingAction: null,
      },
      checked: false,
      disabled: false,
      canConfigure: false,
      visibleStatus: "error",
    },
    {
      input: {
        statusKind: "idle",
        isActive: false,
        isPaused: false,
        credentialState: "missing",
        pendingAction: null,
      },
      checked: false,
      disabled: true,
      canConfigure: true,
      visibleStatus: "setupRequired",
    },
    {
      input: {
        statusKind: "idle",
        isActive: false,
        isPaused: false,
        credentialState: "unavailable",
        pendingAction: null,
      },
      checked: false,
      disabled: true,
      canConfigure: true,
      visibleStatus: "credentialUnavailable",
    },
    {
      input: {
        statusKind: "idle",
        isActive: false,
        isPaused: false,
        credentialState: "present",
        pendingAction: "start",
      },
      checked: true,
      disabled: true,
      canConfigure: false,
      visibleStatus: "connecting",
    },
    {
      input: {
        statusKind: "listening",
        isActive: true,
        isPaused: false,
        credentialState: "present",
        pendingAction: "stop",
      },
      checked: false,
      disabled: true,
      canConfigure: false,
      visibleStatus: "stopping",
    },
  ])("derives $visibleStatus", (expected) => {
    expect(settingsSessionControlState(expected.input)).toEqual({
      checked: expected.checked,
      disabled: expected.disabled,
      canConfigure: expected.canConfigure,
      visibleStatus: expected.visibleStatus,
    });
  });

  it("keeps an optimistic start pending until native confirmation", () => {
    const coordinator = new SettingsSessionActionCoordinator();

    expect(coordinator.begin("start")).toBe(true);
    expect(coordinator.pendingAction).toBe("start");
    expect(coordinator.begin("start")).toBe(false);
    expect(
      coordinator.observeNativeState({
        statusKind: "connecting",
        isActive: true,
      }),
    ).toBeNull();
  });

  it("keeps stop optimistic through the native stopping state", () => {
    const coordinator = new SettingsSessionActionCoordinator();

    expect(coordinator.begin("stop")).toBe(true);
    expect(coordinator.pendingAction).toBe("stop");
    expect(
      coordinator.observeNativeState({
        statusKind: "stopping",
        isActive: true,
      }),
    ).toBe("stop");
    expect(coordinator.begin("start")).toBe(false);
    expect(
      coordinator.observeNativeState({ statusKind: "idle", isActive: false }),
    ).toBeNull();
  });

  it("hands off immediately when native confirmation arrives", () => {
    const coordinator = new SettingsSessionActionCoordinator();

    expect(coordinator.begin("stop")).toBe(true);
    expect(
      coordinator.observeNativeState({ statusKind: "idle", isActive: false }),
    ).toBeNull();
    expect(coordinator.begin("start")).toBe(true);
    expect(coordinator.pendingAction).toBe("start");
  });

  it("clears the matching pending action when its command is rejected", () => {
    const coordinator = new SettingsSessionActionCoordinator();

    expect(coordinator.begin("start")).toBe(true);
    expect(coordinator.commandRejected("start")).toBe(true);
    expect(coordinator.pendingAction).toBeNull();
  });

  it("does not let a stale rejection clear a newer action", () => {
    const coordinator = new SettingsSessionActionCoordinator();

    expect(coordinator.begin("start")).toBe(true);
    expect(
      coordinator.observeNativeState({
        statusKind: "listening",
        isActive: true,
      }),
    ).toBeNull();
    expect(coordinator.begin("stop")).toBe(true);
    expect(coordinator.commandRejected("start")).toBe(false);
    expect(coordinator.pendingAction).toBe("stop");
  });

  it.each(["idle", "error"] as const)(
    "clears start on a fresh inactive %s state",
    (statusKind) => {
      const coordinator = new SettingsSessionActionCoordinator();

      expect(coordinator.begin("start")).toBe(true);
      expect(
        coordinator.observeNativeState({ statusKind, isActive: false }),
      ).toBeNull();
    },
  );

  it("keeps stop pending through an active error and clears only when inactive", () => {
    const coordinator = new SettingsSessionActionCoordinator();

    expect(coordinator.begin("stop")).toBe(true);
    expect(
      coordinator.observeNativeState({ statusKind: "error", isActive: true }),
    ).toBe("stop");
    expect(
      coordinator.observeNativeState({ statusKind: "error", isActive: false }),
    ).toBeNull();
  });
});
