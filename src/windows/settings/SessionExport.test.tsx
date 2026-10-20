// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18N } from "../../lib/i18n";
import { SessionExport } from "./SessionExport";

const ipc = vi.hoisted(() => ({
  sessionArchiveState: vi.fn(), sessionArchiveClear: vi.fn(), sessionExport: vi.fn(),
  sessionTranscriptPage: vi.fn(), sessionHistoryList: vi.fn(), sessionHistoryPage: vi.fn(),
  sessionHistoryAudio: vi.fn(), sessionHistoryDelete: vi.fn(),
}));
const store = vi.hoisted(() => ({ session: { isActive: false }, settings: { retainSessionHistory: true, recordSessionAudio: false }, saveSettings: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ isTauri: true, setOverlayPointerCursor: vi.fn(), ...ipc }));
vi.mock("../../lib/store", () => {
  return { useStore: Object.assign((select: (state: unknown) => unknown) => select(store), { getState: () => store }) };
});
let host: HTMLDivElement;
let root: Root;
const page = (id = "current", index = 0) => ({ total: 61, page: index, entries: [{ index: index * 30 + 1, source: `Synthetic ${id}`, translation: `Fixture ${id}`, createdAtMs: 1_700_000_000_000 }] });
function deferred() {
  let resolve!: (value?: unknown) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
async function flush(ms = 0) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }
function button(label: string) {
  const scope = document.querySelector(".settings-confirmation") ?? host;
  const found = [...scope.querySelectorAll("button")].find((node) => node.textContent === label || node.getAttribute("aria-label") === label);
  if (!found) throw new Error(`Button missing: ${label}`);
  return found;
}
async function click(node: HTMLElement) { await act(async () => node.click()); await flush(); }
async function select(index: number) { await click(host.querySelectorAll<HTMLButtonElement>(".session-history__item")[index]); }
async function confirm() { await click(button(I18N.settings.historyDelete)); }
async function search(value: string) {
  const input = host.querySelector("input")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await flush(180);
}
beforeEach(async () => {
  vi.useFakeTimers(); vi.clearAllMocks();
  store.session.isActive = false;
  store.settings.retainSessionHistory = true;
  store.settings.recordSessionAudio = false;
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 61, audioBytes: 0 });
  ipc.sessionHistoryList.mockResolvedValue(["A", "B"].map((id, n) => ({ id, count: 61, hasAudio: false, startedAtMs: 1_700_000_000_000 + n * 3600000 })));
  ipc.sessionTranscriptPage.mockImplementation((_query, index) => Promise.resolve(page("current", index)));
  ipc.sessionHistoryPage.mockImplementation((id, _query, index) => Promise.resolve(page(id, index)));
  ipc.sessionExport.mockResolvedValue(false);
  ipc.sessionHistoryDelete.mockResolvedValue(undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });

it("refreshes the current transcript across a new session even when the count stays unchanged", async () => {
  expect(host.textContent).toContain("Synthetic current");
  const reads = ipc.sessionTranscriptPage.mock.calls.length;
  ipc.sessionTranscriptPage.mockResolvedValue(page("new-session"));
  store.session.isActive = true;
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  expect(ipc.sessionTranscriptPage).toHaveBeenCalledTimes(reads + 1);
  expect(host.textContent).toContain("Synthetic new-session");
  expect(host.textContent).not.toContain("Synthetic current");
});

it("rejects a transcript request from before the session changed", async () => {
  const old = deferred();
  ipc.sessionTranscriptPage.mockReturnValueOnce(old.promise);
  await click(button(I18N.settings.transcriptNext));
  ipc.sessionTranscriptPage.mockResolvedValue(page("new-session", 1));
  store.session.isActive = true;
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  await act(async () => old.resolve(page("old-session", 1)));
  expect(host.textContent).toContain("Synthetic new-session");
  expect(host.textContent).not.toContain("Synthetic old-session");
});

it("clears the current transcript when retention is disabled and reloads a reenabled archive with the same count", async () => {
  store.settings.retainSessionHistory = false;
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  expect(host.textContent).not.toContain("Synthetic current");
  ipc.sessionTranscriptPage.mockResolvedValue(page("reenabled"));
  store.settings.retainSessionHistory = true;
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  expect(host.textContent).toContain("Synthetic reenabled");
});

it("preserves a selected historical transcript across current-session lifecycle and retention changes", async () => {
  await select(1);
  const reads = ipc.sessionHistoryPage.mock.calls.length;
  store.session.isActive = true;
  store.settings.retainSessionHistory = false;
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  expect(host.textContent).toContain("Synthetic A");
  expect(ipc.sessionHistoryPage).toHaveBeenCalledTimes(reads);
});

it("pauses document-hidden archive polling without treating an unfocused window as hidden", async () => {
  await act(async () => window.dispatchEvent(new Event("blur")));
  const beforeBlur = ipc.sessionArchiveState.mock.calls.length;
  await flush(2000);
  expect(ipc.sessionArchiveState).toHaveBeenCalledTimes(beforeBlur + 2);
  const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
  await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  const reads = ipc.sessionArchiveState.mock.calls.length;
  await flush(5000);
  expect(ipc.sessionArchiveState).toHaveBeenCalledTimes(reads);
  hidden.mockReturnValue(false);
  await act(async () => document.dispatchEvent(new Event("visibilitychange"))); await flush();
  expect(ipc.sessionArchiveState).toHaveBeenCalledTimes(reads + 1);
});

it("pauses archive polling while its page is hidden and refreshes immediately on return", async () => {
  await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
  const reads = ipc.sessionArchiveState.mock.calls.length;
  await flush(5000);
  expect(ipc.sessionArchiveState).toHaveBeenCalledTimes(reads);
  ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 7, audioBytes: 0 });
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  expect(ipc.sessionArchiveState).toHaveBeenCalledTimes(reads + 1);
  expect(host.querySelector(".session-transcript__heading")?.textContent).toContain(I18N.settings.transcriptCount(7));
});

it("does not start archive polling for an initially hidden page", async () => {
  await act(async () => root.render(null));
  ipc.sessionArchiveState.mockClear();
  await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
  await flush(5000);
  expect(ipc.sessionArchiveState).not.toHaveBeenCalled();
});

it("discards an archive read from the previous page visit", async () => {
  const old = deferred();
  ipc.sessionArchiveState.mockReturnValueOnce(old.promise);
  await flush(1000);
  await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
  ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 7, audioBytes: 0 });
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  await act(async () => old.resolve({ transcriptCount: 99, audioBytes: 0 }));
  expect(host.querySelector(".session-transcript__heading")?.textContent).toContain(I18N.settings.transcriptCount(7));
});

it("bounds playback reads across repeated clicks and permits retry after failure", async () => {
  const objectUrl = vi.fn(() => "blob:synthetic-A");
  vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: objectUrl, revokeObjectURL: vi.fn() }));
  try {
    ipc.sessionHistoryList.mockResolvedValue([{ id: "A", count: 1, hasAudio: true, startedAtMs: 1_700_000_000_000 }]);
    await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
    await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
    await select(1);
    const pending = deferred();
    ipc.sessionHistoryAudio.mockReturnValue(pending.promise);
    const play = button(I18N.settings.historyPlayAudio);
    await act(async () => { play.click(); play.click(); });
    await click(play);
    expect(ipc.sessionHistoryAudio).toHaveBeenCalledTimes(1);
    expect(play.disabled).toBe(true);
    expect(play.getAttribute("aria-busy")).toBe("true");
    await act(async () => pending.reject(new Error("synthetic failure")));
    expect(play.disabled).toBe(false);
    expect(host.textContent).toContain(I18N.settings.historyAudioFailed);
    ipc.sessionHistoryAudio.mockResolvedValueOnce(new ArrayBuffer(4));
    await click(play);
    expect(ipc.sessionHistoryAudio).toHaveBeenCalledTimes(2);
    expect(host.querySelector("audio")?.src).toBe("blob:synthetic-A");
    expect(host.textContent).not.toContain(I18N.settings.historyAudioFailed);
  } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
});

it("finishes a stale playback read before allowing another selection to load", async () => {
  const objectUrl = vi.fn(() => "blob:synthetic-B");
  vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: objectUrl, revokeObjectURL: vi.fn() }));
  try {
    ipc.sessionHistoryList.mockResolvedValue(["A", "B"].map(id => ({ id, count: 1, hasAudio: true, startedAtMs: 1_700_000_000_000 })));
    await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
    await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
    await select(1);
    const old = deferred();
    ipc.sessionHistoryAudio.mockReturnValueOnce(old.promise);
    await click(button(I18N.settings.historyPlayAudio));
    await select(2);
    const play = button(I18N.settings.historyPlayAudio);
    expect(play.disabled).toBe(true);
    await click(play);
    expect(ipc.sessionHistoryAudio).toHaveBeenCalledTimes(1);
    await act(async () => old.resolve(new ArrayBuffer(4)));
    expect(objectUrl).not.toHaveBeenCalled();
    expect(play.disabled).toBe(false);
    ipc.sessionHistoryAudio.mockResolvedValueOnce(new ArrayBuffer(4));
    await click(play);
    expect(ipc.sessionHistoryAudio).toHaveBeenLastCalledWith("B", "system");
    expect(host.querySelector("audio")?.src).toBe("blob:synthetic-B");
  } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
});

describe("history operation state", () => {
  it.each([false, null, true, "failure"])("retains query, page and transcript after export outcome %s", async (outcome) => {
    await select(1); await search("Synthetic"); await click(button(I18N.settings.transcriptNext)); await flush(180);
    if (outcome === "failure") ipc.sessionExport.mockRejectedValueOnce(new Error("synthetic failure"));
    else ipc.sessionExport.mockResolvedValueOnce(outcome);
    const reads = ipc.sessionHistoryPage.mock.calls.length;
    await click(button(I18N.settings.exportTranscript));
    expect(host.textContent).toContain("Synthetic A");
    expect(host.querySelector("input")!.value).toBe("Synthetic");
    expect(host.textContent).toContain(I18N.settings.transcriptPage(2, 3));
    expect(ipc.sessionHistoryPage).toHaveBeenLastCalledWith("A", "Synthetic", 1);
    expect(ipc.sessionHistoryPage).toHaveBeenCalledTimes(reads);
    expect(ipc.sessionExport).toHaveBeenCalledWith("transcript", "A");
    if (outcome === true) {
      expect(host.querySelector(".settings-toast")?.textContent).toContain(I18N.settings.sessionExportSaved);
      await flush(3000);
      expect(host.querySelector(".settings-toast")).toBeNull();
    }
    if (outcome === "failure") expect(host.textContent).toContain(I18N.settings.sessionExportFailed);
  });
  it("retains current transcript and repeated selection", async () => {
    await select(0); await click(button(I18N.settings.exportTranscript)); await flush(5000);
    expect(host.textContent).toContain("Synthetic current");
  });
  it("keeps a pending transcript read valid during canceled export", async () => {
    const read = deferred(); ipc.sessionHistoryPage.mockReturnValueOnce(read.promise);
    await select(1); await click(button(I18N.settings.exportTranscript));
    await act(async () => read.resolve(page("A")));
    expect(host.textContent).toContain("Synthetic A");
  });
  it("dispatches deletion once even for two clicks in the same event turn and preserves B", async () => {
    const pending = deferred(); ipc.sessionHistoryDelete.mockReturnValueOnce(pending.promise);
    await select(1); await confirm();
    const deleteButton = button(I18N.settings.historyDelete);
    await act(async () => { deleteButton.click(); deleteButton.click(); });
    expect(ipc.sessionHistoryDelete).toHaveBeenCalledTimes(1);
    expect(deleteButton.disabled).toBe(true);
    expect(host.querySelector('[aria-busy="true"]')).not.toBeNull();
    await click(button(I18N.settings.cancel)); await select(2); await act(async () => pending.resolve()); await flush();
    expect(host.textContent).toContain("Synthetic B");
    expect(host.querySelectorAll(".session-history__item")).toHaveLength(2);
    expect(host.querySelector(".is-selected")!.textContent).not.toBe(I18N.settings.historyCurrent);
  });
  it("reports delete failure, permits retry and returns to current after confirmed success", async () => {
    await select(1); await confirm(); ipc.sessionHistoryDelete.mockRejectedValueOnce(new Error("fixture"));
    await click(button(I18N.settings.historyDelete));
    expect(document.querySelector(".settings-confirmation .settings-toast")?.textContent).toBe(I18N.settings.historyDeleteFailed);
    expect(host.querySelector(".settings-feedback")).toBeNull();
    expect(button(I18N.settings.historyDelete).disabled).toBe(false);
    await click(button(I18N.settings.historyDelete));
    expect(ipc.sessionHistoryDelete).toHaveBeenCalledTimes(2);
    expect(host.textContent).toContain("Synthetic current");
    expect(host.textContent).not.toContain(I18N.settings.historyDeleteFailed);
  });
  it.each(["delete", "export"])("ignores stale %s feedback after navigating to B", async (kind) => {
    const pending = deferred(); await select(1);
    if (kind === "delete") { ipc.sessionHistoryDelete.mockReturnValueOnce(pending.promise); await confirm(); await click(button(I18N.settings.historyDelete)); }
    else { ipc.sessionExport.mockReturnValueOnce(pending.promise); await click(button(I18N.settings.exportTranscript)); }
    if (kind === "delete") await click(button(I18N.settings.cancel));
    await select(2); await act(async () => pending.reject(new Error("fixture"))); await flush();
    expect(host.textContent).toContain("Synthetic B");
    expect(host.textContent).not.toContain(I18N.settings.historyDeleteFailed);
    expect(host.textContent).not.toContain(I18N.settings.sessionExportFailed);
  });
  it("removes a deleted selection after leaving and returning during deletion", async () => {
    const pending = deferred(); await select(1); await confirm(); ipc.sessionHistoryDelete.mockReturnValueOnce(pending.promise);
    await click(button(I18N.settings.historyDelete));
    await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
    expect(document.querySelector(".settings-confirmation")).toBeNull();
    await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
    await act(async () => pending.resolve()); await flush();
    expect(host.querySelectorAll(".session-history__item")).toHaveLength(2);
    expect(host.querySelector(".is-selected")?.textContent).toContain(I18N.settings.historyCurrent);
    expect(host.textContent).not.toContain("Synthetic A");
    expect(host.textContent).toContain("Synthetic current");
    expect(host.querySelector(".settings-toast")).toBeNull();
  });
  it("does not restore a deleted session from a list captured before deletion finished", async () => {
    const deletion = deferred(); const listing = deferred();
    await select(1); await confirm(); ipc.sessionHistoryDelete.mockReturnValueOnce(deletion.promise);
    await click(button(I18N.settings.historyDelete));
    await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
    ipc.sessionHistoryList.mockReturnValueOnce(listing.promise);
    await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
    await act(async () => deletion.resolve()); await flush();
    await act(async () => listing.resolve(["A", "B"].map((id, n) => ({ id, count: 61, hasAudio: false, startedAtMs: 1_700_000_000_000 + n * 3600000 }))));
    await flush();
    expect(host.querySelectorAll(".session-history__item")).toHaveLength(2);
    expect(host.textContent).not.toContain("Synthetic A");
    await select(1);
    expect(ipc.sessionHistoryPage).toHaveBeenLastCalledWith("B", "", 0);
  });
  it("discards pending transcript and playback for the deleted selection", async () => {
    const objectUrl = vi.fn(() => "blob:synthetic-A");
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: objectUrl, revokeObjectURL: vi.fn() }));
    try {
      ipc.sessionHistoryList.mockResolvedValue(["A", "B"].map((id, n) => ({ id, count: 61, hasAudio: true, startedAtMs: 1_700_000_000_000 + n * 3600000 })));
      await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
      await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
      const transcript = deferred(); const playback = deferred(); const deletion = deferred(); const currentRead = deferred();
      ipc.sessionHistoryPage.mockReturnValueOnce(transcript.promise);
      await select(1);
      ipc.sessionHistoryAudio.mockReturnValueOnce(playback.promise);
      await click(button(I18N.settings.historyPlayAudio));
      await confirm(); ipc.sessionHistoryDelete.mockReturnValueOnce(deletion.promise);
      await click(button(I18N.settings.historyDelete));
      ipc.sessionTranscriptPage.mockReturnValueOnce(currentRead.promise);
      await act(async () => {
        deletion.resolve();
        transcript.resolve(page("A"));
        playback.resolve(new ArrayBuffer(4));
      });
      await flush();
      expect(host.textContent).not.toContain("Synthetic A");
      expect(host.querySelector("audio")).toBeNull();
      expect(objectUrl).not.toHaveBeenCalled();
      await act(async () => currentRead.resolve(page("current")));
      expect(host.textContent).toContain("Synthetic current");
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });
  it.each([false, true])("preserves B's recording when deletion finishes after navigation (playback pending: %s)", async (playbackPending) => {
    const objectUrl = vi.fn(() => "blob:synthetic-B");
    const revokeObjectUrl = vi.fn();
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: objectUrl, revokeObjectURL: revokeObjectUrl }));
    try {
      ipc.sessionHistoryList.mockResolvedValue(["A", "B"].map((id, n) => ({ id, count: 61, hasAudio: true, startedAtMs: 1_700_000_000_000 + n * 3600000 })));
      await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
      await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
      const deletion = deferred(); const playback = deferred();
      await select(1); await confirm(); ipc.sessionHistoryDelete.mockReturnValueOnce(deletion.promise);
      await click(button(I18N.settings.historyDelete)); await click(button(I18N.settings.cancel)); await select(2);
      ipc.sessionHistoryAudio.mockReturnValueOnce(playbackPending ? playback.promise : Promise.resolve(new ArrayBuffer(4)));
      await click(button(I18N.settings.historyPlayAudio));
      await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
      await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
      await act(async () => deletion.resolve()); await flush();
      if (playbackPending) await act(async () => playback.resolve(new ArrayBuffer(4)));
      expect(host.querySelectorAll(".session-history__item")).toHaveLength(2);
      expect(host.textContent).toContain("Synthetic B");
      expect(host.querySelector("audio")?.src).toBe("blob:synthetic-B");
      expect(revokeObjectUrl).not.toHaveBeenCalled();
      expect(host.querySelector(".settings-toast")).toBeNull();
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });
  it("keeps the reopened selection and suppresses stale feedback when deletion fails", async () => {
    const deletion = deferred(); await select(1); await confirm(); ipc.sessionHistoryDelete.mockReturnValueOnce(deletion.promise);
    await click(button(I18N.settings.historyDelete));
    await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
    await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
    await act(async () => deletion.reject(new Error("synthetic deletion failure"))); await flush();
    expect(host.querySelectorAll(".session-history__item")).toHaveLength(3);
    expect(host.textContent).toContain("Synthetic A");
    expect(host.querySelector(".settings-toast")).toBeNull();
  });
  it("retains transcript after clear failure, then hides content after successful clear", async () => {
    ipc.sessionArchiveClear.mockRejectedValueOnce(new Error("fixture"));
    await click(button(I18N.settings.clearSessionArchive));
    expect(host.textContent).toContain("Synthetic current");
    ipc.sessionArchiveClear.mockImplementationOnce(async () => {
      ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 0, audioBytes: 0 });
    });
    await click(button(I18N.settings.clearSessionArchive)); await flush();
    expect(host.textContent).not.toContain("Synthetic current");
    expect(host.textContent).toContain(I18N.settings.sessionArchiveCleared);
  });
  it("ignores successful export feedback after changing selection", async () => {
    const pending = deferred(); ipc.sessionExport.mockReturnValueOnce(pending.promise);
    await select(1); await click(button(I18N.settings.exportTranscript)); await select(2);
    await act(async () => pending.resolve(true)); await flush();
    expect(host.textContent).toContain("Synthetic B");
    expect(host.textContent).not.toContain(I18N.settings.sessionExportSaved);
  });
  it("canceling delete confirmation makes no deletion request", async () => {
    await select(1); await confirm(); await click(button(I18N.settings.historyCancel));
    expect(ipc.sessionHistoryDelete).not.toHaveBeenCalled();
    expect(host.textContent).toContain("Synthetic A");
  });
  it("uses a modal for history deletion and blocks navigation until canceled", async () => {
    await select(1); await confirm();
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain(I18N.settings.historyDeleteConfirm);
    expect(host.querySelector(".session-history__confirm")).toBeNull();
    await select(2);
    expect(host.textContent).toContain("Synthetic A");
    await click(button(I18N.settings.cancel)); await select(2);
    expect(host.textContent).toContain("Synthetic B");
    expect(ipc.sessionHistoryDelete).not.toHaveBeenCalled();
  });
  it("ignores pending export completion after closing and reopening", async () => {
    const pending = deferred(); ipc.sessionExport.mockReturnValueOnce(pending.promise);
    await click(button(I18N.settings.exportTranscript));
    await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
    await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
    await act(async () => pending.resolve(true)); await flush();
    expect(host.textContent).not.toContain(I18N.settings.sessionExportSaved);
    expect(host.textContent).toContain("Synthetic current");
  });
  it("shows read failure and recovers through a new query", async () => {
    ipc.sessionHistoryPage.mockRejectedValueOnce(new Error("fixture")); await select(1);
    expect(host.textContent).toContain(I18N.settings.transcriptReadFailed);
    await search("Synthetic"); expect(host.textContent).toContain("Synthetic A");
  });
});


it("selects a separate recording for playback and export, and discards stale playback after switching sources", async () => {
  const objectUrl = vi.fn(() => "blob:synthetic-microphone");
  vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: objectUrl, revokeObjectURL: vi.fn() }));
  Element.prototype.scrollIntoView = vi.fn();
  ipc.sessionHistoryList.mockResolvedValue([{ id: "dual", count: 1, hasAudio: true, audioSources: ["system", "microphone"], startedAtMs: 1_700_000_000_000 }]);
  await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  await select(1);
  const pending = deferred();
  ipc.sessionHistoryAudio.mockReturnValueOnce(pending.promise);
  await click(button(I18N.settings.historyPlayAudio));
  expect(ipc.sessionHistoryAudio).toHaveBeenCalledWith("dual", "system");
  await click(host.querySelector<HTMLElement>('[role="combobox"]')!);
  await click(document.querySelectorAll<HTMLElement>('[role="option"]')[1]);
  await act(async () => pending.resolve(new ArrayBuffer(4)));
  expect(objectUrl).not.toHaveBeenCalled();
  ipc.sessionHistoryAudio.mockResolvedValueOnce(new ArrayBuffer(4));
  await click(button(I18N.settings.historyPlayAudio));
  expect(ipc.sessionHistoryAudio).toHaveBeenLastCalledWith("dual", "microphone");
  expect(host.querySelector("audio")?.src).toBe("blob:synthetic-microphone");
  await click(button(I18N.settings.exportAudio));
  expect(ipc.sessionExport).toHaveBeenLastCalledWith("audio", "dual", "microphone");
  vi.unstubAllGlobals();
});

it("automatically uses the actual single microphone recording instead of assuming system audio", async () => {
  ipc.sessionHistoryList.mockResolvedValue([{ id: "mic", count: 1, hasAudio: true, audioSources: ["microphone"], startedAtMs: 1_700_000_000_000 }]);
  await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  await select(1);
  expect(host.querySelector('[role="combobox"]')).toBeNull();
  await click(button(I18N.settings.exportAudio));
  expect(ipc.sessionExport).toHaveBeenLastCalledWith("audio", "mic", "microphone");
});
