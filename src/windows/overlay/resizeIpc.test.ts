import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { NativeResizeGesture } from "./resizeIpc";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

type Request = {
  command: string;
  args?: Record<string, unknown>;
  resolve: () => void;
  reject: (error: Error) => void;
  settled: boolean;
};
let requests: Request[];
let owners: NativeResizeGesture[];
let start: typeof import("./resizeIpc").startNativeResize;
let inFlight: number;
let peakInFlight: number;

beforeEach(async () => {
  vi.resetModules();
  requests = [];
  owners = [];
  inFlight = 0;
  peakInFlight = 0;
  invoke.mockReset().mockImplementation((command: string, args?: Record<string, unknown>) => {
    peakInFlight = Math.max(peakInFlight, ++inFlight);
    return new Promise<void>((resolve, reject) => {
      requests.push({ command, args, resolve, reject, settled: false });
    });
  });
  const module = await import("./resizeIpc");
  start = (...args) => {
    const owner = module.startNativeResize(...args);
    owners.push(owner);
    return owner;
  };
});

async function settle(index: number, fail = false) {
  const request = requests[index];
  expect(request.settled).toBe(false);
  request.settled = true;
  inFlight--;
  if (fail) request.reject(new Error("bridge unavailable"));
  else request.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

afterEach(async () => {
  owners.forEach(owner => owner.finish());
  for (let index = 0; index < requests.length; index++) {
    if (!requests[index].settled) await settle(index);
  }
  expect(inFlight).toBe(0);
  expect(peakInFlight).toBeLessThanOrEqual(1);
});

it("bounds 120 pointer updates behind start to one latest coordinate and one request in flight", async () => {
  const owner = start("topLeft", 800, 600);
  for (let i = 0; i < 120; i++) owner.move(801 + i, 601 + i);
  owner.finish();
  expect(invoke.mock.calls).toEqual([["resize_start", { region: "topLeft", x: 800, y: 600 }]]);
  expect(inFlight).toBe(1);
  await settle(0);
  expect(invoke.mock.lastCall).toEqual(["resize_move", { x: 920, y: 720 }]);
  await settle(1);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  await settle(2);
  expect(requests).toHaveLength(3);
});

it("forwards immediately while idle and merges later moves behind the current acknowledgement", async () => {
  const owner = start("bottomRight", 100, 200);
  await settle(0);
  owner.move(110, 210);
  expect(invoke.mock.lastCall).toEqual(["resize_move", { x: 110, y: 210 }]);
  owner.move(120, 220);
  owner.move(130, 230);
  expect(requests).toHaveLength(2);
  await settle(1);
  expect(invoke.mock.lastCall).toEqual(["resize_move", { x: 130, y: 230 }]);
  owner.finish();
  owner.move(140, 240);
  await settle(2);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
});

it("holds a replacement mount behind the old final move and end acknowledgements", async () => {
  const old = start("topLeft", 100, 200);
  old.move(110, 210);
  old.finish();
  const replacement = start("bottomRight", 900, 600);
  replacement.move(950, 650);
  expect(requests).toHaveLength(1);
  await settle(0);
  expect(invoke.mock.lastCall).toEqual(["resize_move", { x: 110, y: 210 }]);
  await settle(1);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  expect(requests).toHaveLength(3);
  await settle(2);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "bottomRight", x: 900, y: 600 }]);
  await settle(3);
  expect(invoke.mock.lastCall).toEqual(["resize_move", { x: 950, y: 650 }]);
});

it("discards 120 gestures released before they can start", async () => {
  start("top", 100, 200).finish();
  for (let i = 0; i < 120; i++) {
    const pending = start("bottom", i, i);
    pending.move(i + 1, i + 1);
    pending.finish();
  }
  expect(requests).toHaveLength(1);
  await settle(0);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  await settle(1);
  expect(requests).toHaveLength(2);
});

it("retains only the latest waiting owner and ignores superseded updates", async () => {
  start("top", 100, 200).finish();
  const discarded = start("left", 300, 400);
  const latest = start("right", 500, 600);
  discarded.move(999, 999);
  discarded.finish();
  latest.move(510, 610);
  await settle(0);
  await settle(1);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "right", x: 500, y: 600 }]);
  await settle(2);
  expect(invoke.mock.lastCall).toEqual(["resize_move", { x: 510, y: 610 }]);
});

it.each(["start", "move"])("requires an end barrier after rejected %s before starting the next owner", async failed => {
  const old = start("left", 100, 200);
  if (failed === "move") {
    await settle(0);
    old.move(110, 210);
  }
  const failedIndex = requests.length - 1;
  const latest = start("right", 500, 600);
  latest.move(510, 610);
  await settle(failedIndex, true);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  expect(requests.filter(request => request.command === "resize_start")).toHaveLength(1);
  await settle(failedIndex + 1);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "right", x: 500, y: 600 }]);
  old.move(999, 999);
  old.finish();
});

it("limits failed end recovery and lets a fresh gesture retry the barrier", async () => {
  start("left", 100, 200).finish();
  await settle(0);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  await settle(1, true);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  await settle(2, true);
  expect(requests).toHaveLength(3);
  const next = start("right", 500, 600);
  next.move(510, 610);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  expect(requests).toHaveLength(4);
  await settle(3);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "right", x: 500, y: 600 }]);
});

it("holds a waiting owner behind the retry of a failed normal end", async () => {
  start("left", 100, 200).finish();
  await settle(0);
  const next = start("right", 500, 600);
  next.move(510, 610);
  await settle(1, true);
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["resize_start", "resize_end", "resize_end"]);
  await settle(2);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "right", x: 500, y: 600 }]);
});

it("honors one recovery request made by a new owner while cleanup was in flight", async () => {
  start("left", 100, 200);
  await settle(0, true);
  start("right", 500, 600);
  await settle(1, true);
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["resize_start", "resize_end", "resize_end"]);
  await settle(2);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "right", x: 500, y: 600 }]);
});

it("does not queue released replacement clicks behind a failed cleanup", async () => {
  start("left", 100, 200);
  await settle(0, true);
  expect(invoke.mock.lastCall).toEqual(["resize_end"]);
  const click = start("right", 500, 600);
  click.finish();
  await settle(1, true);
  expect(requests).toHaveLength(2);
  const next = start("bottom", 700, 800);
  await settle(2);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "bottom", x: 700, y: 800 }]);
  next.finish();
});

it("keeps an unsettled call as the barrier across arbitrarily many replacement clicks", () => {
  start("left", 100, 200).finish();
  for (let i = 0; i < 120; i++) start("right", i, i).finish();
  expect(requests).toHaveLength(1);
  expect(inFlight).toBe(1);
});

it("contains synchronous bridge exceptions and recovers before a new start", async () => {
  const normalInvoke = invoke.getMockImplementation()!;
  invoke.mockImplementationOnce(() => { throw new Error("bridge unavailable"); });
  start("left", 100, 200);
  expect(invoke.mock.calls.map(([command]) => command)).toEqual(["resize_start", "resize_end"]);
  invoke.mockImplementation(normalInvoke);
  start("right", 500, 600);
  await settle(0);
  expect(invoke.mock.lastCall).toEqual(["resize_start", { region: "right", x: 500, y: 600 }]);
});
