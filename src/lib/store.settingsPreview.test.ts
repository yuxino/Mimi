// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SettingsDraft, SettingsSnapshot } from "./types";
import { disposeStoreSnapshotStreams, useStore } from "./store";

const native = vi.hoisted(() => ({ listenSettings: vi.fn(), listenSession: vi.fn(), getSettings: vi.fn(), getSession: vi.fn(), save: vi.fn() }));
vi.mock("./ipc", async original => ({ ...await original<typeof import("./ipc")>(), isTauri: true,
  listenSettingsChanged: native.listenSettings, listenSessionState: native.listenSession,
  settingsGet: native.getSettings, sessionGetState: native.getSession, settingsSave: native.save,
}));

const initial = useStore.getState();
let emitSettings: (snapshot: SettingsSnapshot) => void;
let persisted: SettingsSnapshot;
let requests: Array<{ draft: SettingsDraft; resolve: (snapshot: SettingsSnapshot) => void; reject: (reason: Error) => void }>;
let operations: Promise<void>[];
const flush = async () => { for (let index = 0; index < 10; index++) await Promise.resolve(); };
const save = (draft: SettingsDraft) => {
  const operation = useStore.getState().saveSettings(draft);
  operations.push(operation);
  // Each test can inspect rejection after the synthetic native reply.
  void operation.catch(() => {});
  return operation;
};
async function finish(index: number, broadcast = true) {
  persisted = { ...persisted, ...requests[index].draft };
  if (broadcast) emitSettings(persisted);
  requests[index].resolve(persisted);
  await flush();
}

beforeEach(async () => {
  disposeStoreSnapshotStreams();
  Object.values(native).forEach(mock => mock.mockReset());
  persisted = { ...initial.settings, fontSize: 18, subtitleBackgroundOpacity: 80, retainSessionHistory: true, recordSessionAudio: true };
  requests = []; operations = [];
  native.listenSettings.mockImplementation(async handler => { emitSettings = handler; return vi.fn(); });
  native.listenSession.mockResolvedValue(vi.fn());
  native.getSettings.mockResolvedValue(persisted); native.getSession.mockResolvedValue(initial.session);
  native.save.mockImplementation(draft => new Promise<SettingsSnapshot>((resolve, reject) => { requests.push({ draft, resolve, reject }); }));
  useStore.setState(initial, true);
  await useStore.getState().init();
});
afterEach(async () => {
  // Drain the real coordinator even when an assertion detects the old bug.
  for (let index = 0; index < operations.length; index++) {
    await flush();
    if (requests[index]) requests[index].resolve({ ...persisted, ...requests[index].draft });
  }
  await Promise.allSettled(operations);
  disposeStoreSnapshotStreams(); useStore.setState(initial, true);
});

it.each([
  ["fontSize", [19, 20]],
  ["fontSize", [19, 20, 19]],
  ["subtitleBackgroundOpacity", [70, 60, 50]],
  ["subtitleBackgroundOpacity", [70, 60, 70]],
] as const)("keeps the latest %s preview when an earlier native save broadcasts", async (field, values) => {
  values.forEach(value => save({ [field]: value }));
  await flush();
  expect(requests).toHaveLength(1);
  expect(useStore.getState().settings[field]).toBe(values.at(-1));
  for (let index = 0; index < 2; index++) {
    persisted = { ...persisted, ...requests[index].draft };
    emitSettings(persisted);
    expect(useStore.getState().settings[field]).toBe(values.at(-1));
    requests[index].resolve(persisted); await flush();
  }
  await Promise.all(operations);
  expect(requests.map(request => request.draft)).toEqual([{ [field]: values[0] }, { [field]: values.at(-1) }]);
  expect(useStore.getState().settings[field]).toBe(values.at(-1));
});

it("merges native changes to other fields while both slider previews are pending", async () => {
  save({ fontSize: 19 }); save({ fontSize: 20 }); save({ subtitleBackgroundOpacity: 50 });
  await flush();
  persisted = { ...persisted, fontSize: 19, subtitleColor: "pink", showSubtitleTimestamps: true };
  emitSettings(persisted);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, subtitleBackgroundOpacity: 50, subtitleColor: "pink", showSubtitleTimestamps: true });
  await finish(0); await finish(1); await Promise.all(operations);
  expect(requests.map(request => request.draft)).toEqual([{ fontSize: 19 }, { fontSize: 20, subtitleBackgroundOpacity: 50 }]);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, subtitleBackgroundOpacity: 50, subtitleColor: "pink", showSubtitleTimestamps: true });
});

it("coalesces interleaved slider changes and completes every caller after the shared native write", async () => {
  save({ fontSize: 19 }); await flush();
  const settled = vi.fn();
  const pending = [
    save({ fontSize: 20 }), save({ subtitleBackgroundOpacity: 60 }),
    save({ fontSize: 16 }), save({ subtitleBackgroundOpacity: 50 }),
  ];
  pending.forEach(operation => { void operation.then(settled); });
  expect(useStore.getState().settings).toMatchObject({ fontSize: 16, subtitleBackgroundOpacity: 50 });
  await finish(0);
  expect(settled).not.toHaveBeenCalled();
  expect(requests.map(request => request.draft)).toEqual([
    { fontSize: 19 }, { fontSize: 16, subtitleBackgroundOpacity: 50 },
  ]);
  await finish(1); await Promise.all(pending);
  expect(settled).toHaveBeenCalledTimes(4);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 16, subtitleBackgroundOpacity: 50 });
});

it("keeps ordinary and mixed drafts as ordered barriers between coalesced slider writes", async () => {
  save({ fontSize: 19 }); await flush();
  save({ fontSize: 20 }); save({ fontSize: 14 });
  save({ retainSessionHistory: false });
  save({ fontSize: 16 }); save({ fontSize: 17 }); save({ subtitleBackgroundOpacity: 60 });
  save({ fontSize: 18, recordSessionAudio: false });
  save({ subtitleBackgroundOpacity: 40 }); save({ subtitleBackgroundOpacity: 30 });
  const expected: SettingsDraft[] = [
    { fontSize: 19 }, { fontSize: 14 }, { retainSessionHistory: false },
    { fontSize: 17, subtitleBackgroundOpacity: 60 }, { fontSize: 18, recordSessionAudio: false },
    { subtitleBackgroundOpacity: 30 },
  ];
  for (let index = 0; index < expected.length; index++) {
    expect(requests[index].draft).toEqual(expected[index]);
    await finish(index);
  }
  await Promise.all(operations);
  expect(requests).toHaveLength(expected.length);
  expect(useStore.getState().settings).toMatchObject({
    fontSize: 18, subtitleBackgroundOpacity: 30, retainSessionHistory: false, recordSessionAudio: false,
  });
});

it("rejects every coalesced caller, rolls back both sliders and continues the queue after a failed write", async () => {
  save({ fontSize: 19 }); await flush();
  const pending = [save({ fontSize: 20 }), save({ subtitleBackgroundOpacity: 50 }), save({ fontSize: 16 })];
  const results = Promise.allSettled(pending);
  const next = save({ subtitleColor: "pink" });
  await finish(0);
  const failure = new Error("synthetic coalesced write failure");
  requests[1].reject(failure);
  expect(await results).toEqual(pending.map(() => ({ status: "rejected", reason: failure })));
  await flush();
  expect(useStore.getState().settings).toMatchObject({ fontSize: 19, subtitleBackgroundOpacity: 80 });
  expect(requests[2].draft).toEqual({ subtitleColor: "pink" });
  await finish(2); await next;
  expect(useStore.getState().settings).toMatchObject({ fontSize: 19, subtitleBackgroundOpacity: 80, subtitleColor: "pink" });
});

it("rolls back only the still-owned field of a failed coalesced write after an external slider edit", async () => {
  save({ fontSize: 19 }); await flush();
  const pending = [save({ fontSize: 20 }), save({ subtitleBackgroundOpacity: 50 })];
  const results = Promise.allSettled(pending);
  await finish(0);
  persisted = { ...persisted, fontSize: 17, subtitleColor: "pink" };
  emitSettings(persisted);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 17, subtitleBackgroundOpacity: 50, subtitleColor: "pink" });
  requests[1].reject(new Error("synthetic merged write failure"));
  expect((await results).every(result => result.status === "rejected")).toBe(true);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 17, subtitleBackgroundOpacity: 80, subtitleColor: "pink" });
  const retry = save({ subtitleBackgroundOpacity: 50 }); await flush(); await finish(2); await retry;
  expect(useStore.getState().settings).toMatchObject({ fontSize: 17, subtitleBackgroundOpacity: 50, subtitleColor: "pink" });
});

it.each(["success", "failure"] as const)("lets a genuinely changed native slider value supersede a pending preview and its late %s", async outcome => {
  const pending = save({ fontSize: 20 }); await flush();
  persisted = { ...persisted, fontSize: 16, subtitleColor: "pink" }; emitSettings(persisted);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 16, subtitleColor: "pink" });
  if (outcome === "failure") {
    requests[0].reject(new Error("synthetic native failure"));
    await expect(pending).rejects.toThrow("synthetic native failure");
  } else {
    requests[0].resolve({ ...persisted, fontSize: 20, subtitleColor: "white" });
    await pending;
  }
  expect(useStore.getState().settings).toMatchObject({ fontSize: 16, subtitleColor: "pink" });
});

it("rolls a failed latest slider back to the confirmed value while preserving unrelated native changes, then retries", async () => {
  save({ fontSize: 19 }); const latest = save({ fontSize: 20 }); await flush();
  await finish(0);
  persisted = { ...persisted, subtitleColor: "pink" }; emitSettings(persisted);
  expect(useStore.getState().settings.fontSize).toBe(20);
  requests[1].reject(new Error("synthetic write failure"));
  await expect(latest).rejects.toThrow("synthetic write failure");
  expect(useStore.getState().settings).toMatchObject({ fontSize: 19, subtitleColor: "pink" });
  const retry = save({ fontSize: 20 }); await flush(); await finish(2); await retry;
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, subtitleColor: "pink" });
});

it("does not replay a stale full snapshot when a slider command responds after an unrelated native event", async () => {
  save({ fontSize: 20 }); await flush();
  persisted = { ...persisted, subtitleColor: "pink" }; emitSettings(persisted);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, subtitleColor: "pink" });
  requests[0].resolve({ ...persisted, fontSize: 20, subtitleColor: "white" }); await operations[0];
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, subtitleColor: "pink" });
});

it("keeps history and recording opt-outs ordered and merges their broadcasts with pending slider intent", async () => {
  save({ fontSize: 19 }); save({ retainSessionHistory: false }); save({ recordSessionAudio: false }); save({ fontSize: 20 });
  await flush();
  for (let index = 0; index < operations.length; index++) await finish(index);
  expect(requests.map(request => request.draft)).toEqual([
    { fontSize: 19 }, { retainSessionHistory: false }, { recordSessionAudio: false }, { fontSize: 20 },
  ]);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, retainSessionHistory: false, recordSessionAudio: false });
});

it("keeps a mixed draft's explicit slider and privacy values together", async () => {
  save({ fontSize: 19 }); save({ fontSize: 20, recordSessionAudio: false }); await flush();
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, recordSessionAudio: false });
  await finish(0); await finish(1);
  expect(requests[1].draft).toEqual({ fontSize: 20, recordSessionAudio: false });
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, recordSessionAudio: false });
});

it("keeps a newer slider preview after an older request fails", async () => {
  const earlier = save({ fontSize: 19 }); save({ fontSize: 20 }); await flush();
  requests[0].reject(new Error("synthetic earlier failure"));
  await expect(earlier).rejects.toThrow("synthetic earlier failure"); await flush();
  expect(useStore.getState().settings.fontSize).toBe(20);
  await finish(1);
  expect(useStore.getState().settings.fontSize).toBe(20);
});

it.each([false, true])("does not use a pre-external-edit acknowledgement as a newer slider's failure baseline (native returns to initial=%s)", async returnsToInitial => {
  const earlier = save({ fontSize: 19 }); await flush();
  persisted = { ...persisted, fontSize: 16 }; emitSettings(persisted);
  if (returnsToInitial) { persisted = { ...persisted, fontSize: 18 }; emitSettings(persisted); }
  const latest = save({ fontSize: 20 });
  requests[0].resolve({ ...persisted, fontSize: 19 }); await earlier; await flush();
  expect(useStore.getState().settings.fontSize).toBe(20);
  requests[1].reject(new Error("synthetic latest failure"));
  await expect(latest).rejects.toThrow("synthetic latest failure");
  expect(useStore.getState().settings.fontSize).toBe(returnsToInitial ? 18 : 16);
});

it.each(["success", "failure"] as const)("removes an earlier failed ordinary preview when the last slider save settles with %s without a broadcast", async outcome => {
  const color = save({ subtitleColor: "pink" }); const font = save({ fontSize: 20 }); await flush();
  requests[0].reject(new Error("synthetic color failure"));
  await expect(color).rejects.toThrow("synthetic color failure"); await flush();
  if (outcome === "success") {
    requests[1].resolve({ ...persisted, fontSize: 20 }); await font;
  } else {
    requests[1].reject(new Error("synthetic font failure"));
    await expect(font).rejects.toThrow("synthetic font failure");
  }
  expect(useStore.getState().settings).toMatchObject({ fontSize: outcome === "success" ? 20 : 18, subtitleColor: "white" });
});

it("keeps a newer ordinary preview when an earlier slider save succeeds without a broadcast", async () => {
  save({ fontSize: 20 }); const color = save({ subtitleColor: "pink" }); await flush();
  await finish(0, false);
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, subtitleColor: "pink" });
  requests[1].reject(new Error("synthetic later color failure"));
  await expect(color).rejects.toThrow("synthetic later color failure");
  expect(useStore.getState().settings).toMatchObject({ fontSize: 20, subtitleColor: "white" });
});

it("does not resurrect a failed slider from a later ordinary operation's rollback snapshot", async () => {
  save({ fontSize: 19 }); const font = save({ fontSize: 20 }); await flush();
  persisted = { ...persisted, fontSize: 19 }; emitSettings(persisted);
  const color = save({ subtitleColor: "pink" });
  requests[0].resolve(persisted); await operations[0]; await flush();
  requests[1].reject(new Error("synthetic font failure"));
  await expect(font).rejects.toThrow("synthetic font failure"); await flush();
  expect(useStore.getState().settings).toMatchObject({ fontSize: 19, subtitleColor: "pink" });
  requests[2].reject(new Error("synthetic color failure"));
  await expect(color).rejects.toThrow("synthetic color failure");
  expect(useStore.getState().settings).toMatchObject({ fontSize: 19, subtitleColor: "white" });
});
