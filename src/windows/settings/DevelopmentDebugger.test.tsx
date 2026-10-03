// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { DevelopmentDebugger, type DebuggerSnapshot } from "./DevelopmentDebugger";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), createObjectURL: vi.fn(), revokeObjectURL: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../lib/ipc", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/ipc")>(), isTauri: true,
}));
vi.mock("../overlay/Timeline", () => ({ Timeline: () => null }));

const original = useStore.getState();
const OriginalURL = URL;
const caseA = "00000000-0000-4000-8000-000000000001";
const caseB = "00000000-0000-4000-8000-000000000002";
const wavBytes = new Uint8Array([82, 73, 70, 70]).buffer;
let host: HTMLDivElement;
let root: Root | null;
let report: DebuggerSnapshot;
let audioResult: () => Promise<ArrayBuffer>;

function snapshot(caseId = caseA): DebuggerSnapshot {
  return {
    trace: { available: true, enabled: false, entryLimit: 512, recorded: 4,
      evicted: 0, frontendDropped: 0, entries: [] },
    route: { provider: "alibabaCloud", audioInput: "both", subtitleDisplayMode: "bilingual" },
    audio: { enabled: false, sources: [
      { source: "system", sampleRateHz: 16_000, bytes: 32_000,
        successfulChunks: 10, failedChunks: 0, cancelledChunks: 0, droppedChunks: 0 },
      { source: "microphone", sampleRateHz: 16_000, bytes: 16_000,
        successfulChunks: 5, failedChunks: 0, cancelledChunks: 0, droppedChunks: 0 },
    ] },
    caseId, replaySnapshots: 0, privateEvents: 0, contentBytes: 0,
    contentDropped: 0, contentLimited: false, contentFailed: false,
  };
}

it("renders no debugger when the native production gate is unavailable", async () => {
  report.trace.available = false;
  await mount();
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("development_debug_snapshot");
  expect(host.querySelector('.development-debugger')).toBeNull();
  expect(host.querySelector('audio')).toBeNull();
});

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  useStore.setState({ ...original, session: { ...original.session, isActive: false } }, true);
  report = snapshot();
  audioResult = async () => wavBytes;
  mocks.invoke.mockReset().mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "development_debug_snapshot": return report;
      case "development_debug_audio": return audioResult();
      case "development_debug_replay": return { elapsedMs: 20, snapshot: original.session };
      case "development_debug_cases": return [caseA, caseB].map((id, index) => ({
        id, createdAtUnixMs: 1_700_000_000_000 + index, snapshots: 2, provider: "alibabaCloud",
      }));
      case "development_debug_open_case": report = snapshot(String(args?.caseId)); return;
      case "development_debug_start":
        report = { ...snapshot(), trace: { ...report.trace, enabled: true }, caseId: args?.withAudio ? caseA : null };
        return;
      case "development_debug_private_events": return Array.from({ length: 32 }, (_, index) => ({
        elapsedMs: Number(args?.offset) + index,
        event: { kind: "syntheticRequest", protocol: "synthetic", text: `Synthetic private event ${Number(args?.offset) + index}` },
      }));
      case "development_debug_trace_events": return Array.from({ length: Math.min(64, 70 - Number(args?.offset)) }, (_, index) => ({
        id: Number(args?.offset) + index + 1, elapsedMs: index,
        event: { kind: "pipeline", label: "Synthetic saved event" },
      })).reverse();
      default: throw new Error(`Unexpected debugger command: ${command}`);
    }
  });
  let nextUrl = 0;
  mocks.createObjectURL.mockReset().mockImplementation(() => `blob:synthetic-debug-${++nextUrl}`);
  mocks.revokeObjectURL.mockReset();
  vi.stubGlobal("URL", class extends OriginalURL {
    static createObjectURL = mocks.createObjectURL;
    static revokeObjectURL = mocks.revokeObjectURL;
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

afterEach(async () => {
  if (root) await act(async () => root?.unmount());
  root = null; host.remove(); useStore.setState(original, true);
  setStoredUiLanguage("system"); vi.useRealTimers(); vi.unstubAllGlobals();
});

async function mount(visible = true) { await act(async () => root!.render(<DevelopmentDebugger visible={visible} />)); }
function button(label: string, scope: ParentNode = host): HTMLButtonElement {
  const found = [...scope.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === label);
  if (!found) throw new Error(`Missing debugger button: ${label}`);
  return found;
}
async function click(label: string, scope: ParentNode = host) { await act(async () => button(label, scope).click()); }
function audioRow(source: "system" | "microphone"): HTMLElement {
  const label = source === "system" ? "System audio" : "Microphone";
  const found = [...host.querySelectorAll<HTMLElement>(".development-debugger__audio")]
    .find(row => row.firstElementChild?.textContent?.startsWith(label));
  if (!found) throw new Error(`Missing audio row: ${source}`);
  return found;
}
async function active(isActive: boolean) {
  await act(async () => useStore.setState(state => ({ session: { ...state.session, isActive } })));
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}

it("reads availability only while visible and never opts into trace, audio, private content or replay on mount", async () => {
  await mount(false);
  expect(mocks.invoke).not.toHaveBeenCalled();
  await mount();
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("development_debug_snapshot");
  await act(async () => { await vi.advanceTimersByTimeAsync(1_000); });
  expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual([
    "development_debug_snapshot", "development_debug_snapshot", "development_debug_snapshot",
  ]);
  expect(host.querySelector("audio")).toBeNull();
  expect(mocks.createObjectURL).not.toHaveBeenCalled();
  await mount(false);
  const reads = mocks.invoke.mock.calls.length;
  await act(async () => { await vi.advanceTimersByTimeAsync(1_000); });
  expect(mocks.invoke).toHaveBeenCalledTimes(reads);
});

it.each([["Start trace", false], ["Record audio and subtitles", true]] as const)(
  "opts into %s only after its explicit action", async (label, withAudio) => {
    await mount();
    await click(label);
    expect(mocks.invoke).toHaveBeenCalledWith("development_debug_start", { withAudio });
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "development_debug_start")).toHaveLength(1);
    expect(mocks.invoke.mock.calls.some(([command]) => command === "session_start")).toBe(false);
  },
);

it("binds both audio sources to the exact case and revokes replaced URLs and remaining URLs on unmount", async () => {
  await mount();
  await click("Load audio", audioRow("system"));
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_audio", { source: "system", caseId: caseA });
  expect(host.querySelector('audio[aria-label="System audio"]')?.getAttribute("src")).toBe("blob:synthetic-debug-1");
  const blob = mocks.createObjectURL.mock.calls[0][0] as Blob;
  expect(blob.type).toBe("audio/wav"); expect(blob.size).toBe(wavBytes.byteLength);
  await click("Load audio", audioRow("system"));
  expect(mocks.revokeObjectURL).toHaveBeenCalledExactlyOnceWith("blob:synthetic-debug-1");
  expect(host.querySelector('audio[aria-label="System audio"]')?.getAttribute("src")).toBe("blob:synthetic-debug-2");
  await click("Load audio", audioRow("microphone"));
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_audio", { source: "microphone", caseId: caseA });
  expect(host.querySelectorAll("audio")).toHaveLength(2);
  await act(async () => root!.unmount()); root = null;
  expect(mocks.revokeObjectURL.mock.calls.map(([url]) => url)).toEqual([
    "blob:synthetic-debug-1", "blob:synthetic-debug-2", "blob:synthetic-debug-3",
  ]);
});

it("removes old audio when opening a saved case and reads the selected case's audio", async () => {
  await mount();
  await click("Load audio", audioRow("system"));
  await click("List saved cases");
  const select = host.querySelector<HTMLSelectElement>('select[aria-label="Saved cases"]')!;
  await act(async () => { select.value = caseB; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await click("Open case");
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_open_case", { caseId: caseB });
  expect(host.querySelector("code")?.textContent).toBe(caseB);
  expect(host.querySelector("audio")).toBeNull();
  expect(mocks.revokeObjectURL).toHaveBeenCalledWith("blob:synthetic-debug-1");
  await click("Load audio", audioRow("system"));
  expect(mocks.invoke).toHaveBeenLastCalledWith("development_debug_snapshot");
  expect(mocks.invoke.mock.calls.filter(([command]) => command === "development_debug_audio").map(([, args]) => args))
    .toEqual([{ source: "system", caseId: caseA }, { source: "system", caseId: caseB }]);
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  expect(host.querySelector('audio[aria-label="System audio"]')?.getAttribute("src")).toBe("blob:synthetic-debug-2");
});

it("removes loaded players immediately on capture start and keeps recording and listening unavailable during capture", async () => {
  await mount();
  await click("Load audio", audioRow("system"));
  await click("Load audio", audioRow("microphone"));
  await active(true);
  expect(host.querySelector("audio")).toBeNull();
  expect(mocks.revokeObjectURL.mock.calls.map(([url]) => url)).toEqual(["blob:synthetic-debug-1", "blob:synthetic-debug-2"]);
  const protectedButtons = [...host.querySelectorAll<HTMLButtonElement>("button")]
    .filter(node => node.textContent === "Stop subtitles to listen");
  expect(protectedButtons).toHaveLength(3);
  expect(protectedButtons.every(node => node.disabled)).toBe(true);
  const calls = mocks.invoke.mock.calls.length;
  await act(async () => { for (const node of protectedButtons) node.click(); });
  expect(mocks.invoke).toHaveBeenCalledTimes(calls);
  await active(false);
  expect(host.querySelector("audio")).toBeNull();
  expect(mocks.createObjectURL).toHaveBeenCalledTimes(2);
});

it("discards an audio read that completes after capture has started", async () => {
  const pending = deferred<ArrayBuffer>(); audioResult = () => pending.promise;
  await mount();
  await click("Load audio", audioRow("system"));
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_audio", { source: "system", caseId: caseA });
  await active(true);
  await act(async () => pending.resolve(wavBytes));
  expect(host.querySelector("audio")).toBeNull();
  expect(mocks.createObjectURL).not.toHaveBeenCalled();
  expect(mocks.invoke.mock.calls.some(([command]) => command === "session_stop" || command === "development_debug_start")).toBe(false);
});

it("does not revive a pending audio player after capture starts and stops before the read finishes", async () => {
  const pending = deferred<ArrayBuffer>(); audioResult = () => pending.promise;
  await mount();
  await click("Load audio", audioRow("system"));
  await active(true);
  await active(false);
  await act(async () => pending.resolve(wavBytes));
  expect(host.querySelector("audio")).toBeNull();
  expect(mocks.createObjectURL).not.toHaveBeenCalled();
});

it("cleans current audio and discards pending audio when polling observes a different case", async () => {
  await mount();
  await click("Load audio", audioRow("system"));
  const pending = deferred<ArrayBuffer>(); audioResult = () => pending.promise;
  await click("Load audio", audioRow("microphone"));
  report = snapshot(caseB);
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  expect(host.querySelector("code")?.textContent).toBe(caseB);
  expect(host.querySelector("audio")).toBeNull();
  expect(mocks.revokeObjectURL).toHaveBeenCalledExactlyOnceWith("blob:synthetic-debug-1");
  await act(async () => pending.resolve(wavBytes));
  expect(mocks.createObjectURL).toHaveBeenCalledOnce();
  expect(host.querySelector("audio")).toBeNull();
});

it("discards an audio read after unmount instead of creating an unreachable URL", async () => {
  const pending = deferred<ArrayBuffer>(); audioResult = () => pending.promise;
  await mount();
  await click("Load audio", audioRow("system"));
  const calls = mocks.invoke.mock.calls.length;
  await act(async () => root!.unmount()); root = null;
  await act(async () => pending.resolve(wavBytes));
  expect(mocks.createObjectURL).not.toHaveBeenCalled();
  expect(mocks.invoke).toHaveBeenCalledTimes(calls);
});

it("keeps all evidence-loss counts and storage limits in a visible alert alongside the total", async () => {
  report = { ...report, trace: { ...report.trace, evicted: 1, frontendDropped: 2 },
    contentDropped: 3, audio: { ...report.audio, limited: true, failedStorage: true,
      sources: report.audio.sources!.map(source => ({ ...source, droppedChunks: source.source === "system" ? 4 : 0 })) } };
  await mount();
  const metric = [...host.querySelectorAll(".development-debugger__metrics > div")]
    .find(node => node.querySelector("dt")?.textContent === "Missing evidence");
  expect(metric?.querySelector("dd")?.textContent).toBe("10");
  const alert = host.querySelector<HTMLElement>('[role="alert"]')!;
  expect(alert.textContent).toBe("Missing evidence: evicted=1 frontend=2 content=3 audio=4 limited=true failed=true");
  expect(alert.hidden).toBe(false); expect(alert.closest("details")).toBeNull();
});

it("distinguishes memory eviction from complete durable evidence and retains old-case loss semantics", async () => {
  report = { ...report, trace: { ...report.trace, recorded: 2_720, evicted: 672 },
    tracePersistenceEnabled: true, persistedTraceEntries: 2_720, traceDropped: 0 };
  await mount();
  const metrics = [...host.querySelectorAll(".development-debugger__metrics > div")];
  expect(metrics.find(node => node.querySelector("dt")?.textContent === "Missing evidence")?.querySelector("dd")?.textContent).toBe("0");
  expect(metrics.find(node => node.querySelector("dt")?.textContent === "Saved events")?.querySelector("dd")?.textContent).toBe("2720");
  expect(host.querySelector('[role="alert"]')).toBeNull();
  report = { ...report, tracePersistenceEnabled: false };
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("evicted=672");
});

it("reports durable queue loss without counting its content-loss subset twice", async () => {
  report = { ...report, trace: { ...report.trace, evicted: 100, frontendDropped: 1 },
    tracePersistenceEnabled: true, persistedTraceEntries: 70, traceDropped: 2, contentDropped: 3 };
  await mount();
  const metric = [...host.querySelectorAll(".development-debugger__metrics > div")]
    .find(node => node.querySelector("dt")?.textContent === "Missing evidence");
  expect(metric?.querySelector("dd")?.textContent).toBe("4");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("content=3 audio=0 savedTrace=2");
});

it("loads bounded saved-event pages for the exact case, orders IDs and resets on case change", async () => {
  report = { ...report, tracePersistenceEnabled: true, persistedTraceEntries: 70 };
  await mount();
  expect(mocks.invoke.mock.calls.some(([command]) => command === "development_debug_trace_events")).toBe(false);
  const trace = host.querySelector<HTMLElement>(".development-debugger__trace")!;
  await click("Load saved events", trace);
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_trace_events", { caseId: caseA, offset: 0, limit: 64 });
  const summaries = [...trace.querySelectorAll(".development-debugger__events summary")];
  expect(summaries[0].firstElementChild?.textContent).toBe("#1");
  expect(summaries.at(-1)?.firstElementChild?.textContent).toBe("#64");
  await click("Next", trace);
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_trace_events", { caseId: caseA, offset: 64, limit: 64 });
  expect(trace.textContent).toContain("65–70 / 70");
  expect(button("Next", trace).disabled).toBe(true);
  await click("Previous", trace);
  expect(trace.textContent).toContain("1–64 / 70");
  report = { ...snapshot(caseB), tracePersistenceEnabled: true, persistedTraceEntries: 70 };
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  expect(trace.querySelectorAll(".development-debugger__events summary")).toHaveLength(0);
  expect(button("Recent events", trace).disabled).toBe(true);
  await click("Load saved events", trace);
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_trace_events", { caseId: caseB, offset: 0, limit: 64 });
});

it("discards a saved-event page that resolves after the selected case changes", async () => {
  report = { ...report, tracePersistenceEnabled: true, persistedTraceEntries: 70 };
  const result = deferred<Array<{ id: number; elapsedMs: number; event: { kind: string; label: string } }>>();
  const originalInvoke = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) => command === "development_debug_trace_events" ? result.promise : originalInvoke(command, args));
  await mount();
  await click("Load saved events");
  report = { ...snapshot(caseB), tracePersistenceEnabled: true, persistedTraceEntries: 70 };
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  await act(async () => result.resolve([{ id: 1, elapsedMs: 1, event: { kind: "pipeline", label: "Obsolete case entry" } }]));
  expect(host.textContent).not.toContain("Obsolete case entry");
  expect(host.querySelectorAll(".development-debugger__events summary")).toHaveLength(0);
});

it("does not restore an older case when an earlier poll completes after opening a new case", async () => {
  await mount(); await click("List saved cases");
  const oldPoll = deferred<DebuggerSnapshot>(); const oldReport = report;
  const originalInvoke = mocks.invoke.getMockImplementation()!;
  let delayNextPoll = true;
  mocks.invoke.mockImplementation((command, args) => {
    if (command === "development_debug_snapshot" && delayNextPoll) {
      delayNextPoll = false; return oldPoll.promise;
    }
    return originalInvoke(command, args);
  });
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  const select = host.querySelector<HTMLSelectElement>('select[aria-label="Saved cases"]')!;
  await act(async () => { select.value = caseB; select.dispatchEvent(new Event("change", { bubbles: true })); });
  await click("Open case");
  expect(host.querySelector("code")?.textContent).toBe(caseB);
  await act(async () => oldPoll.resolve(oldReport));
  expect(host.querySelector("code")?.textContent).toBe(caseB);
});

it("explains a full case store without displaying arbitrary error content", async () => {
  const originalInvoke = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command, args) => {
    if (command === "development_debug_start") throw "development_evidence_storage_limit";
    return originalInvoke(command, args);
  });
  await mount(); await click("Record audio and subtitles");
  expect(host.querySelector('[role="status"]')?.textContent).toBe("Local case storage is full. Export and remove cases you no longer need.");
  mocks.invoke.mockImplementation(async (command, args) => {
    if (command === "development_debug_start") throw "unexpected-private-service-content";
    return originalInvoke(command, args);
  });
  await click("Record audio and subtitles");
  expect(host.querySelector('[role="status"]')?.textContent).toBe("Operation failed. Try again.");
  expect(host.textContent).not.toContain("unexpected-private-service-content");
});

it("jumps directly to a case-bound snapshot and rejects out-of-range positions", async () => {
  report = { ...report, replaySnapshots: 52 };
  await mount();
  const input = host.querySelector<HTMLInputElement>('input[aria-label="Snapshot number"]')!;
  const change = async (value: string) => act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await change("42"); await click("Go");
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_replay", { index: 41, caseId: caseA });
  expect(input.value).toBe("42");
  await change("53"); expect(button("Go").disabled).toBe(true);
  await change("0"); expect(button("Go").disabled).toBe(true);
});

it("loads private events only on request in bounded pages tied to the current case", async () => {
  report = { ...report, privateEvents: 65 };
  await mount();
  expect(mocks.invoke.mock.calls.some(([command]) => command === "development_debug_private_events")).toBe(false);
  const section = [...host.querySelectorAll(".development-debugger__section")]
    .find(node => node.querySelector("h3")?.textContent === "Recognition and translation request content")!;
  await click("Inspect content", section);
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_private_events", { caseId: caseA, offset: 0 });
  expect(section.querySelectorAll("details")).toHaveLength(32);
  await click("Next", section);
  expect(mocks.invoke).toHaveBeenCalledWith("development_debug_private_events", { caseId: caseA, offset: 32 });
  expect(section.querySelectorAll("details")).toHaveLength(32);
  expect(section.textContent).not.toContain("Synthetic private event 0");
});
