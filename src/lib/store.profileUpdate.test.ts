// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SettingsSnapshot } from "./types";
import { disposeStoreSnapshotStreams, useStore } from "./store";

const native = vi.hoisted(() => ({ listenSettings: vi.fn(), listenSession: vi.fn(), getSettings: vi.fn(), getSession: vi.fn(), updateProfile: vi.fn() }));
vi.mock("./ipc", async original => ({ ...await original<typeof import("./ipc")>(), isTauri: true,
  listenSettingsChanged: native.listenSettings, listenSessionState: native.listenSession,
  settingsGet: native.getSettings, sessionGetState: native.getSession, profileUpdate: native.updateProfile,
}));

const initial = useStore.getState();
let emitSettings: (snapshot: SettingsSnapshot) => void;
const settings: SettingsSnapshot = { ...initial.settings, activeProfileId: "existing", profiles: [{ id: "existing", provider: "alibabaCloud", name: "Original name", credentialState: "present" }] };
const profileId = settings.profiles[0].id;
const renamed = (name: string): SettingsSnapshot => ({ ...settings, profiles: [{ ...settings.profiles[0], name }] });

beforeEach(async () => {
  disposeStoreSnapshotStreams();
  Object.values(native).forEach(mock => mock.mockReset());
  native.listenSettings.mockImplementation(async handler => { emitSettings = handler; return vi.fn(); });
  native.listenSession.mockResolvedValue(vi.fn());
  native.getSettings.mockResolvedValue(settings);
  native.getSession.mockResolvedValue(initial.session);
  useStore.setState(initial, true);
  await useStore.getState().init();
});
afterEach(() => { disposeStoreSnapshotStreams(); useStore.setState(initial, true); });

it("acknowledges a persisted rename after an unrelated event without replacing newer global settings", async () => {
  let finish!: (snapshot: SettingsSnapshot) => void;
  native.updateProfile.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const pending = useStore.getState().updateProfile(profileId, "B 站 · 😀");
  const newer = { ...settings, fontSize: settings.fontSize + 1 };
  emitSettings(newer);
  const acknowledgment = renamed("B 站 · 😀");
  finish(acknowledgment);
  expect(await pending).toBe(acknowledgment);
  expect(useStore.getState().settings).toBe(newer);
  const ownBroadcast = { ...acknowledgment, fontSize: newer.fontSize };
  emitSettings(ownBroadcast);
  expect(useStore.getState().settings).toBe(ownBroadcast);
  expect(useStore.getState().settings.profiles[0].name).toBe("B 站 · 😀");
});

it("retains the newer name when an older rename response arrives after its broadcast", async () => {
  let finish!: (snapshot: SettingsSnapshot) => void;
  native.updateProfile.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const pending = useStore.getState().updateProfile(profileId, "First name");
  const newer = renamed("Second name");
  emitSettings(newer);
  const acknowledgment = renamed("First name");
  finish(acknowledgment);
  expect(await pending).toBe(acknowledgment);
  expect(useStore.getState().settings).toBe(newer);
});

it("sends an alias-only patch without a stale configuration name and accepts its broadcast before the response", async () => {
  let finish!: (snapshot: SettingsSnapshot) => void;
  native.updateProfile.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const options = { textTranslationName: { route: "openAICompatible" as const, name: "B 站" } };
  const pending = useStore.getState().updateProfile(profileId, undefined, options);
  expect(native.updateProfile).toHaveBeenCalledExactlyOnceWith(profileId, undefined, options);
  const saved: SettingsSnapshot = { ...settings, profiles: [{ ...settings.profiles[0], textTranslationNames: { openAICompatible: "B 站" } }] };
  emitSettings(saved);
  finish(saved);
  expect(await pending).toBe(saved);
  expect(useStore.getState().settings).toBe(saved);
});

it("propagates a failed metadata write without changing the current snapshot", async () => {
  native.updateProfile.mockRejectedValueOnce(new Error("profile_catalog_unavailable"));
  await expect(useStore.getState().updateProfile(profileId, "Unsaved name")).rejects.toThrow("profile_catalog_unavailable");
  expect(useStore.getState().settings).toBe(settings);
});
