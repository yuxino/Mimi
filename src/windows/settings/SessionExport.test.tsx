// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import { SessionExport } from "./SessionExport";
import { sessionWorkspaceCopy } from "./sessionWorkspaceCopy";

const ipc = vi.hoisted(() => ({
  sessionArchiveState: vi.fn(), sessionArchiveClear: vi.fn(), sessionExport: vi.fn(),
  sessionTranscriptPage: vi.fn(), sessionHistoryList: vi.fn(), sessionHistoryPage: vi.fn(),
  sessionHistoryAudio: vi.fn(), sessionArchiveAudio: vi.fn(), sessionHistoryDelete: vi.fn(), sessionHistoryRename: vi.fn(),
}));
const clipboard = vi.hoisted(() => ({ write: vi.fn() }));
const store = vi.hoisted(() => ({ session: { isActive: false }, settings: { retainSessionHistory: true, recordSessionAudio: false }, saveSettings: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ isTauri: true, setOverlayPointerCursor: vi.fn(), ...ipc }));
vi.mock("../../lib/diagnosticClipboard", () => ({ writeDiagnosticClipboard: clipboard.write }));
vi.mock("../../lib/store", () => {
  return { useStore: Object.assign((select: (state: unknown) => unknown) => select(store), { getState: () => store }) };
});
let host: HTMLDivElement;
let root: Root;
const page = (id = "current", index = 0) => ({ total: 61, page: index, entries: [{ index: index * 30 + 1, source: `Synthetic ${id}`, translation: `Fixture ${id}`, createdAtMs: 1_700_000_000_000 }] });
const catalog = (hasAudio = false) => ["A", "B"].map((id, n) => ({ id, count: 61, hasAudio, startedAtMs: 1_700_000_000_000 + n * 3600000 }));
function deletedCatalog(hasAudio = false) { ipc.sessionHistoryList.mockResolvedValue(catalog(hasAudio).filter(item => item.id !== "A")); }
function renamedCatalog(title: string) { ipc.sessionHistoryList.mockResolvedValue(catalog().map(item => item.id === "A" ? { ...item, title } : item)); }
function deferred() {
  let resolve!: (value?: unknown) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
async function flush(ms = 0) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }
function button(label: string) {
  const scope = document.querySelector(".settings-confirmation") ?? document.querySelector(".settings-action-menu") ?? host;
  const found = [...scope.querySelectorAll("button")].find((node) => node.textContent === label || node.getAttribute("aria-label") === label);
  if (!found) throw new Error(`Button missing: ${label}`);
  return found;
}
async function click(node: HTMLElement) { await act(async () => node.click()); await flush(); }
async function library() {
  const back = host.querySelector<HTMLButtonElement>(".session-workspace__back");
  if (back) await click(back);
}
async function select(index: number) {
  await library();
  const item = host.querySelectorAll<HTMLButtonElement>(".session-history__item")[index];
  if (!item) throw new Error(`Session missing: ${index}`);
  await click(item);
}
async function more(label: string) {
  const trigger = host.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]');
  if (!trigger) throw new Error("More menu missing");
  await click(trigger);
  await click(button(label));
}
async function confirm() { await more(I18N.settings.historyDelete); }
const copy = () => sessionWorkspaceCopy[effectiveUiLanguage()];
const defaultExportOptions = { format: "txt", content: "bilingual", includeTimestamps: false };
function input(label: string) {
  const found = [...document.querySelectorAll<HTMLInputElement>("input")].find(node => node.getAttribute("aria-label") === label);
  if (!found) throw new Error(`Input missing: ${label}`);
  return found;
}
async function type(input: HTMLInputElement, value: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
async function search(value: string) {
  await type(input(I18N.settings.transcriptSearch), value);
  await flush(180);
}
async function exportRecording() {
  await click(button(copy().export));
  await choose(copy().format, copy().wav);
  await click(button(copy().chooseFile));
}
async function exportTranscript() {
  await click(button(copy().export));
  await click(button(copy().chooseFile));
}
async function choose(label: string, option: string) {
  const scope = () => document.querySelector(".settings-confirmation") ?? host;
  const control = [...scope().querySelectorAll<HTMLElement>('[role="combobox"]')].find(node => node.getAttribute("aria-label") === label);
  if (!control) throw new Error(`Select missing: ${label}`);
  await click(control);
  const item = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === option);
  if (!item) throw new Error(`Option missing: ${option}`);
  await click(item);
  const current = [...scope().querySelectorAll<HTMLElement>('[role="combobox"]')].find(node => node.getAttribute("aria-label") === label);
  expect(current?.textContent).toBe(option);
}
async function render(visible = true) {
  await act(async () => root.render(<><SessionExport visible={visible} /><SettingsToastRegion /></>));
  await flush();
}
beforeEach(async () => {
  vi.useFakeTimers(); vi.clearAllMocks();
  store.session.isActive = false;
  store.settings.retainSessionHistory = true;
  store.settings.recordSessionAudio = false;
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 61, audioBytes: 0 });
  ipc.sessionArchiveClear.mockReset().mockResolvedValue(undefined);
  ipc.sessionHistoryList.mockReset().mockResolvedValue(catalog());
  ipc.sessionTranscriptPage.mockImplementation((_query, index) => Promise.resolve(page("current", index)));
  ipc.sessionHistoryPage.mockImplementation((id, _query, index) => Promise.resolve(page(id, index)));
  ipc.sessionExport.mockResolvedValue(false);
  ipc.sessionHistoryDelete.mockReset().mockImplementation(async () => deletedCatalog());
  ipc.sessionHistoryRename.mockReset().mockImplementation(async (_id: string, title: string) => renamedCatalog(title));
  ipc.sessionArchiveAudio.mockReset().mockResolvedValue(new ArrayBuffer(4));
  clipboard.write.mockReset().mockResolvedValue(undefined);
  Element.prototype.scrollIntoView = vi.fn();
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue();
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(() => {});
  host = document.createElement("div"); host.className = "settings-console__scroll"; document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  await select(0);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });

it.each([0, 1])("debounces search edits but requests the next matching page immediately (session %s)", async index => {
  await select(index);
  const read = index === 0 ? ipc.sessionTranscriptPage : ipc.sessionHistoryPage;
  read.mockClear();
  await type(input(I18N.settings.transcriptSearch), "Syn");
  await flush(100);
  await type(input(I18N.settings.transcriptSearch), "Synthetic");
  await flush(179);
  expect(read).not.toHaveBeenCalled();
  await flush(1);
  expect(read).toHaveBeenCalledOnce();
  await click(button(I18N.settings.transcriptNext));
  expect(read).toHaveBeenCalledTimes(2);
  expect(read).toHaveBeenLastCalledWith(...(index === 0 ? ["Synthetic", 1] : ["A", "Synthetic", 1]));
  expect(host.textContent).toContain(I18N.settings.transcriptPage(2, 3));
  await click(button(I18N.settings.transcriptPrevious));
  expect(read).toHaveBeenCalledTimes(3);
  expect(read).toHaveBeenLastCalledWith(...(index === 0 ? ["Synthetic", 0] : ["A", "Synthetic", 0]));
});

it("shows loading until a search returns an empty result", async () => {
  const pending = deferred();
  ipc.sessionTranscriptPage.mockReturnValueOnce(pending.promise);
  await type(input(I18N.settings.transcriptSearch), "Missing");
  expect(host.textContent).toContain(I18N.settings.transcriptLoading);
  expect(host.textContent).not.toContain(I18N.settings.transcriptNoMatches);
  await flush(180);
  expect(host.textContent).toContain(I18N.settings.transcriptLoading);
  await act(async () => pending.resolve({ total: 0, page: 0, entries: [] }));
  expect(host.textContent).toContain(I18N.settings.transcriptNoMatches);
});

it("retries an unchanged search immediately and rejects its late reply after clearing the query", async () => {
  ipc.sessionTranscriptPage.mockRejectedValueOnce(new Error("synthetic failure"));
  await search("Synthetic");
  expect(host.textContent).toContain(I18N.settings.transcriptReadFailed);
  const pending = deferred();
  ipc.sessionTranscriptPage.mockReturnValueOnce(pending.promise);
  const reads = ipc.sessionTranscriptPage.mock.calls.length;
  await click(button(I18N.settings.retryUpdate));
  expect(ipc.sessionTranscriptPage).toHaveBeenCalledTimes(reads + 1);
  expect(button(I18N.settings.retryUpdate).disabled).toBe(true);
  await type(input(I18N.settings.transcriptSearch), "");
  await flush();
  expect(ipc.sessionTranscriptPage).toHaveBeenLastCalledWith("", 0);
  expect(host.textContent).toContain("Synthetic current");
  await act(async () => pending.resolve(page("stale")));
  expect(host.textContent).not.toContain("Synthetic stale");
  expect(host.textContent).not.toContain(I18N.settings.transcriptReadFailed);
});

it("recovers an archive read failure even when the next status is identical", async () => {
  ipc.sessionArchiveState.mockRejectedValueOnce(new Error("synthetic metadata failure"));
  await flush(1000);
  expect(host.textContent).toContain(I18N.settings.sessionArchiveReadFailed);
  expect(button(copy().export).disabled).toBe(true);
  await flush(1000);
  expect(host.textContent).not.toContain(I18N.settings.sessionArchiveReadFailed);
  expect(button(copy().export).disabled).toBe(false);
  expect(host.textContent).toContain("Synthetic current");
});

it("opens the library without reading content and restores its scroll and selected entry after reading", async () => {
  await act(() => root.render(null));
  ipc.sessionTranscriptPage.mockClear(); ipc.sessionHistoryPage.mockClear();
  await render();
  expect(host.querySelector(".session-transcript")).toBeNull();
  expect(host.querySelectorAll(".session-history__item")).toHaveLength(3);
  expect(ipc.sessionTranscriptPage).not.toHaveBeenCalled();
  expect(ipc.sessionHistoryPage).not.toHaveBeenCalled();
  host.scrollTop = 230;
  host.querySelector(".session-workspace__catalog")!.scrollTop = 72;
  await select(1);
  expect(host.scrollTop).toBe(0);
  expect(document.activeElement).toBe(host.querySelector(".session-transcript__heading h3"));
  expect(host.querySelector(".session-history")).toBeNull();
  expect(host.querySelectorAll(".session-workspace__reader-actions button")).toHaveLength(2);
  expect(host.querySelector('.session-workspace__reader-actions [aria-haspopup="menu"]')).not.toBeNull();
  expect(host.querySelector(".session-workspace__reader-actions")?.textContent).not.toContain(I18N.settings.historyDelete);
  await library();
  expect(host.scrollTop).toBe(230);
  expect(host.querySelector(".session-workspace__catalog")!.scrollTop).toBe(72);
  const selected = host.querySelector('[data-history-id="A"]');
  expect(selected?.getAttribute("aria-pressed")).toBe("true");
  expect(document.activeElement).toBe(selected);
});

it("keeps the same reader's query, page and reading mode when returning from the library", async () => {
  await select(1); await search("Synthetic"); await choose(copy().reading, copy().translation);
  await click(button(I18N.settings.transcriptNext)); await flush(180);
  await library();
  await select(1); await flush(180);
  expect(input(I18N.settings.transcriptSearch).value).toBe("Synthetic");
  expect(host.textContent).toContain(I18N.settings.transcriptPage(2, 3));
  expect(host.querySelector(".session-transcript__pair")?.textContent).toContain("Fixture A");
  expect(host.querySelector(".session-transcript__pair")?.textContent).not.toContain("Synthetic A");
});

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
    expect(host.textContent).toContain(copy().playFailed);
    ipc.sessionHistoryAudio.mockResolvedValueOnce(new ArrayBuffer(4));
    await click(play);
    expect(ipc.sessionHistoryAudio).toHaveBeenCalledTimes(2);
    expect(host.querySelector("audio")?.src).toBe("blob:synthetic-A");
    expect(host.textContent).not.toContain(copy().playFailed);
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
    const play = button(copy().loadingAudio);
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
    await exportTranscript();
    expect(host.textContent).toContain("Synthetic A");
    expect(input(I18N.settings.transcriptSearch).value).toBe("Synthetic");
    expect(host.textContent).toContain(I18N.settings.transcriptPage(2, 3));
    expect(ipc.sessionHistoryPage).toHaveBeenLastCalledWith("A", "Synthetic", 1);
    expect(ipc.sessionHistoryPage).toHaveBeenCalledTimes(reads);
    expect(ipc.sessionExport).toHaveBeenCalledWith("transcript", "A", undefined, defaultExportOptions);
    if (outcome === true) {
      expect(host.querySelector(".settings-toast")?.textContent).toContain(I18N.settings.sessionExportSaved);
      await flush(3000);
      expect(host.querySelector(".settings-toast")).toBeNull();
    }
    if (outcome === "failure") expect(host.textContent).toContain(I18N.settings.sessionExportFailed);
  });
  it("retains current transcript and repeated selection", async () => {
    await select(0); await exportTranscript(); await flush(5000);
    expect(host.textContent).toContain("Synthetic current");
  });
  it("keeps a pending transcript read valid during canceled export", async () => {
    const read = deferred(); ipc.sessionHistoryPage.mockReturnValueOnce(read.promise);
    await select(1); await exportTranscript();
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
    expect(host.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')?.disabled).toBe(true);
    await click(button(I18N.settings.cancel)); await select(2);
    deletedCatalog();
    await act(async () => pending.resolve()); await flush();
    expect(host.textContent).toContain("Synthetic B");
    await library();
    expect(host.querySelectorAll(".session-history__item")).toHaveLength(2);
    expect(host.querySelector(".is-selected")!.textContent).not.toBe(I18N.settings.historyCurrent);
  });
  it("reports delete failure, permits retry and returns to the library after confirmed success", async () => {
    await select(1); await confirm(); ipc.sessionHistoryDelete.mockRejectedValueOnce(new Error("fixture"));
    await click(button(I18N.settings.historyDelete));
    expect(document.querySelector(".settings-confirmation .settings-toast")?.textContent).toBe(I18N.settings.historyDeleteFailed);
    expect(host.querySelector(".settings-feedback")).toBeNull();
    expect(button(I18N.settings.historyDelete).disabled).toBe(false);
    await click(button(I18N.settings.historyDelete));
    expect(ipc.sessionHistoryDelete).toHaveBeenCalledTimes(2);
    expect(host.querySelector(".session-transcript")).toBeNull();
    await select(0);
    expect(host.textContent).toContain("Synthetic current");
    expect(host.textContent).not.toContain(I18N.settings.historyDeleteFailed);
  });
  it.each(["delete", "export"])("ignores stale %s feedback after navigating to B", async (kind) => {
    const pending = deferred(); await select(1);
    if (kind === "delete") { ipc.sessionHistoryDelete.mockReturnValueOnce(pending.promise); await confirm(); await click(button(I18N.settings.historyDelete)); }
    else { ipc.sessionExport.mockReturnValueOnce(pending.promise); await exportTranscript(); }
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
    deletedCatalog();
    await act(async () => pending.resolve()); await flush();
    expect(host.querySelectorAll(".session-history__item")).toHaveLength(2);
    expect(host.querySelector(".is-selected")?.textContent).toContain(I18N.settings.historyCurrent);
    expect(host.textContent).not.toContain("Synthetic A");
    await select(0);
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
    deletedCatalog();
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
      deletedCatalog(true);
      await act(async () => {
        deletion.resolve();
        transcript.resolve(page("A"));
        playback.resolve(new ArrayBuffer(4));
      });
      await flush();
      expect(host.textContent).not.toContain("Synthetic A");
      expect(host.querySelector("audio")).toBeNull();
      expect(objectUrl).not.toHaveBeenCalled();
      await select(0);
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
      deletedCatalog(true);
      await act(async () => deletion.resolve()); await flush();
      if (playbackPending) await act(async () => playback.resolve(new ArrayBuffer(4)));
      expect(host.textContent).toContain("Synthetic B");
      expect(host.querySelector("audio")?.src).toBe("blob:synthetic-B");
      expect(revokeObjectUrl).not.toHaveBeenCalled();
      expect(host.querySelector(".settings-toast")).toBeNull();
      await library();
      expect(host.querySelectorAll(".session-history__item")).toHaveLength(2);
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });
  it("keeps the reopened selection and suppresses stale feedback when deletion fails", async () => {
    const deletion = deferred(); await select(1); await confirm(); ipc.sessionHistoryDelete.mockReturnValueOnce(deletion.promise);
    await click(button(I18N.settings.historyDelete));
    await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
    await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
    await act(async () => deletion.reject(new Error("synthetic deletion failure"))); await flush();
    expect(host.textContent).toContain("Synthetic A");
    expect(host.querySelector(".settings-toast")).toBeNull();
    await click(button(I18N.settings.cancel));
    await library();
    expect(host.querySelectorAll(".session-history__item")).toHaveLength(3);
    expect(host.querySelector('[aria-pressed="true"]')?.getAttribute("data-history-id")).toBe("A");
  });
  it("retains transcript after clear failure, then hides content after successful clear", async () => {
    ipc.sessionArchiveClear.mockRejectedValueOnce(new Error("fixture"));
    await more(I18N.settings.clearSessionArchive);
    expect(host.textContent).toContain("Synthetic current");
    ipc.sessionArchiveClear.mockImplementationOnce(async () => {
      ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 0, audioBytes: 0 });
    });
    await more(I18N.settings.clearSessionArchive); await flush();
    expect(host.textContent).not.toContain("Synthetic current");
    expect(host.textContent).toContain(I18N.settings.sessionArchiveCleared);
  });
  it("ignores successful export feedback after changing selection", async () => {
    const pending = deferred(); ipc.sessionExport.mockReturnValueOnce(pending.promise);
    await select(1); await exportTranscript(); await select(2);
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
    await click(host.querySelector<HTMLButtonElement>(".session-workspace__back")!);
    expect(host.querySelector(".session-history")).toBeNull();
    expect(host.textContent).toContain("Synthetic A");
    await click(button(I18N.settings.cancel)); await select(2);
    expect(host.textContent).toContain("Synthetic B");
    expect(ipc.sessionHistoryDelete).not.toHaveBeenCalled();
  });
  it("ignores pending export completion after closing and reopening", async () => {
    const pending = deferred(); ipc.sessionExport.mockReturnValueOnce(pending.promise);
    await exportTranscript();
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
  it("retries a failed transcript page in place without changing its query, page or reading mode", async () => {
    await select(1); await search("Synthetic"); await choose(copy().reading, copy().translation);
    ipc.sessionHistoryPage.mockRejectedValueOnce(new Error("private transcript read failure"));
    await click(button(I18N.settings.transcriptNext)); await flush(180);
    expect(host.textContent).toContain(I18N.settings.transcriptReadFailed);
    const pending = deferred();
    ipc.sessionHistoryPage.mockReturnValueOnce(pending.promise);
    const reads = ipc.sessionHistoryPage.mock.calls.length;
    await click(button(I18N.settings.retryUpdate)); await flush(180);
    expect(button(I18N.settings.retryUpdate).disabled).toBe(true);
    await click(button(I18N.settings.retryUpdate));
    expect(ipc.sessionHistoryPage).toHaveBeenCalledTimes(reads + 1);
    expect(ipc.sessionHistoryPage).toHaveBeenLastCalledWith("A", "Synthetic", 1);
    await act(async () => pending.resolve(page("A", 1)));
    expect(host.textContent).not.toContain(I18N.settings.transcriptReadFailed);
    expect(host.textContent).not.toContain("private transcript read failure");
    expect(input(I18N.settings.transcriptSearch).value).toBe("Synthetic");
    expect(host.textContent).toContain(I18N.settings.transcriptPage(2, 3));
    expect(host.querySelector(".session-transcript__pair")?.textContent).toContain("Fixture A");
    expect(host.querySelector(".session-transcript__pair")?.textContent).not.toContain("Synthetic A");
  });
  it("retries a failed history listing in place and preserves its filters", async () => {
    await library();
    await type(input(copy().search), "Review");
    await choose(copy().filter, copy().text);
    ipc.sessionHistoryList.mockRejectedValueOnce(new Error("private history read failure"));
    await render(false); await render();
    expect(host.textContent).toContain(I18N.settings.historyReadFailed);
    const pending = deferred();
    ipc.sessionHistoryList.mockReturnValueOnce(pending.promise);
    const reads = ipc.sessionHistoryList.mock.calls.length;
    await click(button(I18N.settings.retryUpdate));
    expect(button(I18N.settings.retryUpdate).disabled).toBe(true);
    await click(button(I18N.settings.retryUpdate));
    expect(ipc.sessionHistoryList).toHaveBeenCalledTimes(reads + 1);
    await act(async () => pending.resolve(catalog().map(item => ({ ...item, title: `Review ${item.id}` }))));
    expect(host.textContent).not.toContain(I18N.settings.historyReadFailed);
    expect(host.textContent).not.toContain("private history read failure");
    expect(input(copy().search).value).toBe("Review");
    expect(host.querySelector('[role="combobox"]')?.textContent).toBe(copy().text);
    expect(host.querySelector('[data-history-id="A"]')?.textContent).toContain("Review A");
  });
  it("keeps a failed catalog refresh visible after opening an existing session and returning", async () => {
    await library();
    ipc.sessionHistoryList.mockRejectedValueOnce(new Error("private catalog refresh failure"));
    await render(false); await render();
    expect(host.textContent).toContain(I18N.settings.historyReadFailed);
    const reads = ipc.sessionHistoryList.mock.calls.length;
    await select(1); await library();
    expect(ipc.sessionHistoryList).toHaveBeenCalledTimes(reads);
    expect(host.textContent).toContain(I18N.settings.historyReadFailed);
    expect(button(I18N.settings.retryUpdate).disabled).toBe(false);
    await click(button(I18N.settings.retryUpdate));
    expect(host.textContent).not.toContain(I18N.settings.historyReadFailed);
  });
  it.each(["rename", "delete"] as const)("does not dismiss a catalog-read failure when an unrelated %s fails", async action => {
    await library();
    ipc.sessionHistoryList.mockRejectedValueOnce(new Error("private catalog refresh failure"));
    await render(false); await render();
    await select(1);
    if (action === "rename") {
      await more(copy().rename);
      ipc.sessionHistoryRename.mockRejectedValueOnce(new Error("private rename failure"));
      await click(button(copy().save));
    } else {
      await confirm();
      expect(document.querySelector(".settings-confirmation")?.textContent).not.toContain(I18N.settings.historyReadFailed);
      ipc.sessionHistoryDelete.mockRejectedValueOnce(new Error("private delete failure"));
      await click(button(I18N.settings.historyDelete));
    }
    await click(button(I18N.settings.cancel));
    await library();
    expect(host.textContent).toContain(I18N.settings.historyReadFailed);
    expect(button(I18N.settings.retryUpdate).disabled).toBe(false);
  });
  it.each(["current", "saved"] as const)("keeps a failed %s transcript retry available until a read succeeds", async kind => {
    const read = kind === "current" ? ipc.sessionTranscriptPage : ipc.sessionHistoryPage;
    read.mockRejectedValueOnce(new Error("first private read failure"));
    if (kind === "current") await click(button(I18N.settings.transcriptNext));
    else await select(1);
    read.mockRejectedValueOnce(new Error("second private read failure"));
    await click(button(I18N.settings.retryUpdate));
    expect(host.textContent).toContain(I18N.settings.transcriptReadFailed);
    expect(host.textContent).not.toContain("private read failure");
    expect(button(I18N.settings.retryUpdate).disabled).toBe(false);
    await click(button(I18N.settings.retryUpdate));
    expect(host.textContent).not.toContain(I18N.settings.transcriptReadFailed);
    expect(host.textContent).toContain(kind === "current" ? "Synthetic current" : "Synthetic A");
  });
  it("keeps an initially failed history list retryable without presenting an empty library", async () => {
    await act(() => root.render(null));
    ipc.sessionHistoryList.mockRejectedValueOnce(new Error("first private listing failure"));
    await render();
    expect(host.textContent).toContain(I18N.settings.historyReadFailed);
    expect(host.textContent).not.toContain(I18N.settings.historyEmpty);
    ipc.sessionHistoryList.mockRejectedValueOnce(new Error("second private listing failure"));
    await click(button(I18N.settings.retryUpdate));
    expect(host.textContent).toContain(I18N.settings.historyReadFailed);
    expect(button(I18N.settings.retryUpdate).disabled).toBe(false);
    await click(button(I18N.settings.retryUpdate));
    expect(host.textContent).not.toContain(I18N.settings.historyReadFailed);
    expect(host.querySelector('[data-history-id="A"]')).not.toBeNull();
  });
  it.each(["resolve", "reject"] as const)("ignores an old transcript retry that later %s after selecting another session", async outcome => {
    ipc.sessionHistoryPage.mockRejectedValueOnce(new Error("initial read failure"));
    await select(1);
    const pending = deferred();
    ipc.sessionHistoryPage.mockReturnValueOnce(pending.promise);
    await click(button(I18N.settings.retryUpdate));
    await select(2);
    await act(async () => {
      if (outcome === "resolve") pending.resolve(page("outdated-A"));
      else pending.reject(new Error("private old retry failure"));
    });
    expect(host.textContent).toContain("Synthetic B");
    expect(host.textContent).not.toContain("outdated-A");
    expect(host.textContent).not.toContain(I18N.settings.transcriptReadFailed);
  });
  it.each(["resolve", "reject"] as const)("ignores an old history retry that later %s after reopening the page", async outcome => {
    await library();
    ipc.sessionHistoryList.mockRejectedValueOnce(new Error("initial listing failure"));
    await render(false); await render();
    const pending = deferred();
    ipc.sessionHistoryList.mockReturnValueOnce(pending.promise);
    await click(button(I18N.settings.retryUpdate));
    await render(false);
    expect(button(I18N.settings.retryUpdate).disabled).toBe(true);
    const reads = ipc.sessionHistoryList.mock.calls.length;
    await click(button(I18N.settings.retryUpdate));
    expect(ipc.sessionHistoryList).toHaveBeenCalledTimes(reads);
    ipc.sessionHistoryList.mockResolvedValue(catalog().map(item => ({ ...item, title: `Current ${item.id}` })));
    await render();
    await act(async () => {
      if (outcome === "resolve") pending.resolve(catalog().map(item => ({ ...item, title: `Outdated ${item.id}` })));
      else pending.reject(new Error("private old listing failure"));
    });
    expect(host.querySelector('[data-history-id="A"]')?.textContent).toContain("Current A");
    expect(host.textContent).not.toContain("Outdated");
    expect(host.textContent).not.toContain(I18N.settings.historyReadFailed);
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
  await choose(I18N.settings.recordingSource, I18N.settings.audioInputMicrophone);
  await act(async () => pending.resolve(new ArrayBuffer(4)));
  expect(objectUrl).not.toHaveBeenCalled();
  ipc.sessionHistoryAudio.mockResolvedValueOnce(new ArrayBuffer(4));
  await click(button(I18N.settings.historyPlayAudio));
  expect(ipc.sessionHistoryAudio).toHaveBeenLastCalledWith("dual", "microphone");
  expect(host.querySelector("audio")?.src).toBe("blob:synthetic-microphone");
  await exportRecording();
  expect(ipc.sessionExport).toHaveBeenLastCalledWith("audio", "dual", "microphone", undefined);
  vi.unstubAllGlobals();
});

it("automatically uses the actual single microphone recording instead of assuming system audio", async () => {
  ipc.sessionHistoryList.mockResolvedValue([{ id: "mic", count: 1, hasAudio: true, audioSources: ["microphone"], startedAtMs: 1_700_000_000_000 }]);
  await act(async () => root.render(<><SessionExport visible={false} /><SettingsToastRegion /></>));
  await act(async () => root.render(<><SessionExport visible /><SettingsToastRegion /></>)); await flush();
  await select(1);
  expect([...host.querySelectorAll('[role="combobox"]')].some(node => node.getAttribute("aria-label") === I18N.settings.recordingSource)).toBe(false);
  await exportRecording();
  expect(ipc.sessionExport).toHaveBeenLastCalledWith("audio", "mic", "microphone", undefined);
});

describe("session workspace actions", () => {
  it("uses the single export dialog for WAV, with independent source choice and no text controls", async () => {
    ipc.sessionHistoryList.mockResolvedValue([{ id: "dual", count: 1, hasAudio: true,
      audioSources: ["system", "microphone"], startedAtMs: 1_700_000_000_000 }]);
    await render(false); await render(); await select(1);
    await click(button(copy().export));
    await choose(copy().format, copy().wav);
    const dialog = document.querySelector(".settings-confirmation")!;
    expect(dialog.querySelector(`[role="combobox"][aria-label="${copy().content}"]`)).toBeNull();
    expect(dialog.querySelector(`[role="switch"][aria-label="${copy().timestamps}"]`)).toBeNull();
    await choose(I18N.settings.recordingSource, I18N.settings.audioInputMicrophone);
    expect(ipc.sessionHistoryAudio).not.toHaveBeenCalled();
    await click(button(I18N.settings.cancel));
    expect(ipc.sessionExport).not.toHaveBeenCalled();
    await click(button(copy().export));
    await choose(copy().format, copy().wav);
    await click(button(copy().chooseFile));
    expect(ipc.sessionExport).toHaveBeenCalledExactlyOnceWith("audio", "dual", "microphone", undefined);
  });

  it("offers only the available format for an audio-only saved session", async () => {
    ipc.sessionHistoryList.mockResolvedValue([{ id: "audio", count: 0, hasAudio: true,
      audioSources: ["microphone"], startedAtMs: 1_700_000_000_000 }]);
    await render(false); await render(); await select(1);
    expect(host.querySelector(".session-transcript__toolbar")).toBeNull();
    await click(button(copy().export));
    await click(document.querySelector<HTMLElement>(`.settings-confirmation [role="combobox"][aria-label="${copy().format}"]`)!);
    expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([copy().wav]);
    await click(document.querySelector<HTMLElement>('[role="option"]')!);
    expect(document.querySelectorAll(".settings-confirmation [role=combobox]")).toHaveLength(1);
    await click(button(copy().chooseFile));
    expect(ipc.sessionExport).toHaveBeenCalledExactlyOnceWith("audio", "audio", "microphone", undefined);
  });

  it("loads current recording only on explicit play and clears it when recording is disabled", async () => {
    const createUrl = vi.fn(() => "blob:synthetic-current");
    const revoke = vi.fn();
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: createUrl, revokeObjectURL: revoke }));
    try {
      store.settings.recordSessionAudio = true;
      ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 61, audioBytes: 32_000, audioSources: ["microphone"] });
      await render(); await flush(1000);
      expect(ipc.sessionArchiveAudio).not.toHaveBeenCalled();
      await click(button(I18N.settings.historyPlayAudio));
      expect(ipc.sessionArchiveAudio).toHaveBeenCalledExactlyOnceWith("microphone");
      expect(ipc.sessionHistoryAudio).not.toHaveBeenCalled();
      const audio = host.querySelector("audio")!;
      expect(audio.src).toBe("blob:synthetic-current");
      store.settings.recordSessionAudio = false;
      await render();
      expect(audio.getAttribute("src")).toBeNull();
      expect(revoke).toHaveBeenCalledExactlyOnceWith("blob:synthetic-current");
      expect(host.querySelector("audio")?.getAttribute("src") ?? null).toBeNull();
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });

  it.each(["loaded", "pending"] as const)("retires the %s current recording after clearing its archive", async state => {
    const createUrl = vi.fn(() => "blob:cleared-current"), revoke = vi.fn();
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: createUrl, revokeObjectURL: revoke }));
    try {
      store.settings.recordSessionAudio = true;
      ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 61, audioBytes: 32_000 });
      const read = deferred();
      if (state === "pending") ipc.sessionArchiveAudio.mockReturnValueOnce(read.promise);
      await render(); await flush(1000);
      await click(button(I18N.settings.historyPlayAudio));
      const previousAudio = host.querySelector("audio")!;
      const pause = vi.mocked(HTMLMediaElement.prototype.pause);
      const load = vi.mocked(HTMLMediaElement.prototype.load);
      pause.mockClear(); load.mockClear();
      ipc.sessionArchiveClear.mockImplementationOnce(async () => {
        ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 0, audioBytes: 0 });
      });
      await more(I18N.settings.clearSessionArchive);
      expect(ipc.sessionArchiveClear).toHaveBeenCalledOnce();
      if (state === "pending") {
        await act(() => read.resolve(new ArrayBuffer(4)));
        expect(createUrl).not.toHaveBeenCalled();
      } else {
        expect(revoke).toHaveBeenCalledExactlyOnceWith("blob:cleared-current");
        expect(pause.mock.contexts).toContain(previousAudio);
        expect(load.mock.contexts).toContain(previousAudio);
        expect(previousAudio.getAttribute("src")).toBeNull();
      }
      expect(host.querySelector("audio")).toBeNull();
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });

  it("preserves current playback when clearing the archive fails", async () => {
    const revoke = vi.fn();
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: vi.fn(() => "blob:retained-current"), revokeObjectURL: revoke }));
    try {
      store.settings.recordSessionAudio = true;
      ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 61, audioBytes: 32_000 });
      await render(); await flush(1000);
      await click(button(I18N.settings.historyPlayAudio));
      const audio = host.querySelector("audio")!;
      const pause = vi.mocked(HTMLMediaElement.prototype.pause);
      pause.mockClear();
      ipc.sessionArchiveClear.mockRejectedValueOnce(new Error("synthetic clear failure"));
      await more(I18N.settings.clearSessionArchive);
      expect(host.querySelector("audio")).toBe(audio);
      expect(audio.src).toBe("blob:retained-current");
      expect(pause).not.toHaveBeenCalled();
      expect(revoke).not.toHaveBeenCalled();
      expect(host.textContent).toContain(I18N.settings.sessionExportFailed);
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });

  it.each(["saved", "new current"] as const)("does not clear %s playback after an older current clear finishes", async destination => {
    const revoke = vi.fn();
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: vi.fn(() => "blob:new-owner"), revokeObjectURL: revoke }));
    try {
      store.settings.recordSessionAudio = true;
      ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 61, audioBytes: 32_000 });
      ipc.sessionHistoryList.mockResolvedValue(catalog(true));
      await render(false); await render();
      const clear = deferred();
      ipc.sessionArchiveClear.mockReturnValueOnce(clear.promise);
      await more(I18N.settings.clearSessionArchive);
      if (destination === "saved") {
        await select(1);
        ipc.sessionHistoryAudio.mockResolvedValueOnce(new ArrayBuffer(4));
      } else {
        store.session.isActive = true; await render();
        store.session.isActive = false; await render();
      }
      await click(button(I18N.settings.historyPlayAudio));
      const audio = host.querySelector("audio")!;
      expect(audio.src).toBe("blob:new-owner");
      const pause = vi.mocked(HTMLMediaElement.prototype.pause);
      pause.mockClear();
      await act(() => clear.resolve());
      await flush();
      expect(host.querySelector("audio")).toBe(audio);
      expect(audio.src).toBe("blob:new-owner");
      expect(pause).not.toHaveBeenCalled();
      expect(revoke).not.toHaveBeenCalled();
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });

  it.each(["library", "hidden"] as const)("discards a pending recording read after leaving the reader for %s", async destination => {
    const createUrl = vi.fn(() => "blob:stale");
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: createUrl, revokeObjectURL: vi.fn() }));
    try {
      ipc.sessionHistoryList.mockResolvedValue([{ id: "A", count: 1, hasAudio: true, startedAtMs: 1_700_000_000_000 }]);
      await render(false); await render(); await select(1);
      const pending = deferred(); ipc.sessionHistoryAudio.mockReturnValueOnce(pending.promise);
      await click(button(I18N.settings.historyPlayAudio));
      if (destination === "library") await library();
      else await render(false);
      await act(() => pending.resolve(new ArrayBuffer(4)));
      expect(createUrl).not.toHaveBeenCalled();
      expect(host.querySelector("audio")).toBeNull();
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });

  it("allows saved-session exports during capture while current exports and playback remain disabled", async () => {
    ipc.sessionArchiveState.mockResolvedValue({ transcriptCount: 61, audioBytes: 32_000 });
    ipc.sessionHistoryList.mockResolvedValue([{ id: "A", count: 61, hasAudio: true, startedAtMs: 1_700_000_000_000 }]);
    store.session.isActive = true;
    await render();
    expect(button(copy().export).disabled).toBe(true);
    await click(button(copy().export));
    expect(ipc.sessionExport).not.toHaveBeenCalled();
    await select(1);
    expect(button(copy().export).disabled).toBe(false);
    expect(button(I18N.settings.historyPlayAudio).disabled).toBe(true);
    await click(button(I18N.settings.historyPlayAudio));
    expect(ipc.sessionHistoryAudio).not.toHaveBeenCalled();
    await exportTranscript();
    expect(ipc.sessionExport).toHaveBeenLastCalledWith("transcript", "A", undefined, defaultExportOptions);
    await exportRecording();
    expect(ipc.sessionExport).toHaveBeenLastCalledWith("audio", "A", "system", undefined);
    await select(0);
    expect(button(copy().export).disabled).toBe(true);
  });

  it.each(["resolve", "reject"] as const)("discards a pending playback %s after a session starts and permits a fresh stopped read", async outcome => {
    const createUrl = vi.fn(() => "blob:synthetic-stopped");
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: createUrl, revokeObjectURL: vi.fn() }));
    try {
      ipc.sessionHistoryList.mockResolvedValue([{ id: "A", count: 61, hasAudio: true, startedAtMs: 1_700_000_000_000 }]);
      await render(false); await render(); await select(1);
      const pending = deferred();
      ipc.sessionHistoryAudio.mockReturnValueOnce(pending.promise);
      await click(button(I18N.settings.historyPlayAudio));
      store.session.isActive = true;
      await render();
      await act(async () => {
        if (outcome === "resolve") pending.resolve(new ArrayBuffer(4));
        else pending.reject(new Error("old playback failure"));
      });
      expect(createUrl).not.toHaveBeenCalled();
      expect(host.querySelector("audio")?.getAttribute("src")).toBeNull();
      expect(host.textContent).not.toContain(copy().playFailed);
      expect(button(I18N.settings.historyPlayAudio).disabled).toBe(true);
      store.session.isActive = false;
      await render();
      ipc.sessionHistoryAudio.mockResolvedValueOnce(new ArrayBuffer(4));
      await click(button(I18N.settings.historyPlayAudio));
      expect(ipc.sessionHistoryAudio).toHaveBeenCalledTimes(2);
      expect(host.querySelector("audio")?.src).toBe("blob:synthetic-stopped");
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });

  it.each(["active", "selection", "source", "hidden", "unmount"] as const)("stops already buffered playback when its owner changes: %s", async change => {
    vi.stubGlobal("URL", Object.assign(class extends URL {}, { createObjectURL: () => "blob:synthetic-playing", revokeObjectURL: vi.fn() }));
    try {
      ipc.sessionHistoryList.mockResolvedValue(["A", "B"].map(id => ({
        id, count: 61, hasAudio: true, audioSources: ["system", "microphone"], startedAtMs: 1_700_000_000_000,
      })));
      await render(false); await render(); await select(1);
      ipc.sessionHistoryAudio.mockResolvedValueOnce(new ArrayBuffer(4));
      await click(button(I18N.settings.historyPlayAudio));
      const audio = host.querySelector("audio")!;
      expect(audio.src).toBe("blob:synthetic-playing");
      const pause = vi.mocked(HTMLMediaElement.prototype.pause);
      const load = vi.mocked(HTMLMediaElement.prototype.load);
      pause.mockClear(); load.mockClear();
      if (change === "active") { store.session.isActive = true; await render(); }
      else if (change === "selection") await select(2);
      else if (change === "source") await choose(I18N.settings.recordingSource, I18N.settings.audioInputMicrophone);
      else if (change === "hidden") await render(false);
      else await act(async () => root.render(null));
      expect(pause.mock.contexts.filter(element => element === audio)).toHaveLength(1);
      expect(audio.getAttribute("src")).toBeNull();
      expect(load).toHaveBeenCalledOnce();
      expect(load.mock.contexts[0]).toBe(audio);
    } finally { await act(() => root.render(null)); vi.unstubAllGlobals(); }
  });

  it("retains a failed rename draft and retries without changing the transcript query or page", async () => {
    await select(1); await search("Synthetic"); await click(button(I18N.settings.transcriptNext)); await flush(180);
    await more(copy().rename);
    const name = document.querySelector<HTMLInputElement>(".session-workspace__name-field input")!;
    await type(name, "  Product review  ");
    ipc.sessionHistoryRename.mockRejectedValueOnce(new Error("synthetic rename failure"));
    await click(button(copy().save));
    expect(ipc.sessionHistoryRename).toHaveBeenLastCalledWith("A", "Product review");
    expect(document.querySelector(".settings-confirmation")?.textContent).toContain(copy().renameFailed);
    expect(name.value).toBe("  Product review  ");
    expect(button(copy().save).disabled).toBe(false);
    await click(button(copy().save));
    expect(ipc.sessionHistoryRename).toHaveBeenCalledTimes(2);
    expect(document.querySelector(".settings-confirmation")).toBeNull();
    expect(host.querySelector(".session-transcript__heading")?.textContent).toContain("Product review");
    expect(input(I18N.settings.transcriptSearch).value).toBe("Synthetic");
    expect(host.textContent).toContain(I18N.settings.transcriptPage(2, 3));
    expect(ipc.sessionHistoryPage).toHaveBeenLastCalledWith("A", "Synthetic", 1);
  });

  it.each(["resolve", "reject"] as const)("keeps B selected when A's pending rename %s finishes", async outcome => {
    await select(1); await more(copy().rename);
    await type(document.querySelector<HTMLInputElement>(".session-workspace__name-field input")!, "Updated A");
    const pending = deferred();
    ipc.sessionHistoryRename.mockReturnValueOnce(pending.promise);
    await click(button(copy().save));
    await click(button(I18N.settings.cancel));
    await select(2);
    const selectedB = host.querySelector(".session-transcript__heading h3")?.textContent;
    expect(selectedB).toBeTruthy();
    if (outcome === "resolve") renamedCatalog("Updated A");
    await act(async () => {
      if (outcome === "resolve") pending.resolve();
      else pending.reject(new Error("old rename failure"));
    });
    await flush();
    expect(host.textContent).toContain("Synthetic B");
    expect(document.querySelector(".settings-confirmation")).toBeNull();
    expect(host.textContent).not.toContain(copy().renamed);
    expect(host.textContent).not.toContain(copy().renameFailed);
    expect(host.querySelector(".session-transcript__heading h3")?.textContent).toBe(selectedB);
    expect(ipc.sessionHistoryPage).toHaveBeenLastCalledWith("B", "", 0);
    if (outcome === "resolve") {
      await select(1);
      expect(host.querySelector(".session-transcript__heading")?.textContent).toContain("Updated A");
    }
  });

  it.each(["rename", "delete"] as const)("refreshes newly archived B after %s completes without accepting an older list", async kind => {
    const [a, b] = catalog();
    ipc.sessionHistoryList.mockResolvedValue([a]);
    store.session.isActive = true;
    await render();
    await select(1);
    const mutation = deferred(), stoppedListing = deferred();
    if (kind === "rename") {
      await more(copy().rename);
      await type(document.querySelector<HTMLInputElement>(".session-workspace__name-field input")!, "Updated A");
      ipc.sessionHistoryRename.mockReturnValueOnce(mutation.promise);
      await click(button(copy().save));
    } else {
      await confirm();
      ipc.sessionHistoryDelete.mockReturnValueOnce(mutation.promise);
      await click(button(I18N.settings.historyDelete));
    }
    ipc.sessionHistoryList.mockReturnValueOnce(stoppedListing.promise);
    store.session.isActive = false;
    await render();
    const readsBeforeCompletion = ipc.sessionHistoryList.mock.calls.length;
    ipc.sessionHistoryList.mockResolvedValue(kind === "rename" ? [{ ...a, title: "Updated A" }, b] : [b]);
    await act(async () => mutation.resolve());
    await flush();
    expect(ipc.sessionHistoryList.mock.calls.length).toBeGreaterThan(readsBeforeCompletion);
    await act(async () => stoppedListing.resolve([{ ...a, title: "Outdated A" }]));
    await flush();
    await library();
    expect(host.querySelector('[data-history-id="B"]')).not.toBeNull();
    expect(host.textContent).not.toContain("Outdated A");
    if (kind === "rename") expect(host.querySelector('[data-history-id="A"]')?.textContent).toContain("Updated A");
    else expect(host.querySelector('[data-history-id="A"]')).toBeNull();
    await select(kind === "rename" ? 2 : 1);
    expect(host.textContent).toContain("Synthetic B");
    expect(ipc.sessionHistoryPage).toHaveBeenLastCalledWith("B", "", 0);
  });

  it("does not overwrite a successful rename with a listing requested before it completed", async () => {
    await select(1); await more(copy().rename);
    await type(document.querySelector<HTMLInputElement>(".session-workspace__name-field input")!, "Updated A");
    const rename = deferred(), listing = deferred();
    ipc.sessionHistoryRename.mockReturnValueOnce(rename.promise);
    await click(button(copy().save));
    await render(false);
    ipc.sessionHistoryList.mockReturnValueOnce(listing.promise);
    await render();
    renamedCatalog("Updated A");
    await act(async () => rename.resolve());
    await act(async () => listing.resolve([
      { id: "A", title: "Outdated A", count: 61, hasAudio: false, startedAtMs: 1_700_000_000_000 },
      { id: "B", count: 61, hasAudio: false, startedAtMs: 1_700_003_600_000 },
    ]));
    await flush();
    expect(host.querySelector(".session-transcript__heading")?.textContent).toContain("Updated A");
    expect(host.textContent).not.toContain("Outdated A");
    await click(button(I18N.settings.cancel));
    await library();
    expect(host.querySelectorAll(".session-history__item")).toHaveLength(3);
  });

  it("filters history by title and content while preserving the selected transcript", async () => {
    ipc.sessionHistoryList.mockResolvedValue([
      { id: "A", title: "Alpha recording", count: 0, hasAudio: true, startedAtMs: 1_700_000_000_000 },
      { id: "B", title: "Beta notes", count: 61, hasAudio: false, startedAtMs: 1_700_003_600_000 },
    ]);
    await render(false); await render(); await select(2); await search("Synthetic"); await library();
    const reads = ipc.sessionHistoryPage.mock.calls.length;
    await type(input(copy().search), "ALPHA");
    expect(host.querySelectorAll(".session-history__group .session-history__item")).toHaveLength(1);
    expect(host.querySelector(".session-history__group")?.textContent).toContain("Alpha recording");
    expect(host.querySelector(".session-transcript")).toBeNull();
    await choose(copy().filter, copy().text);
    expect(host.textContent).toContain(copy().noMatches);
    expect(host.querySelector(".session-transcript")).toBeNull();
    await type(input(copy().search), "");
    expect(host.querySelectorAll(".session-history__group .session-history__item")).toHaveLength(1);
    expect(host.querySelector(".session-history__group")?.textContent).toContain("Beta notes");
    await choose(copy().filter, copy().audio);
    expect(host.querySelector(".session-history__group")?.textContent).toContain("Alpha recording");
    expect(ipc.sessionHistoryPage).toHaveBeenCalledTimes(reads);
    await choose(copy().filter, copy().all);
    await select(2); await flush(180);
    expect(host.textContent).toContain("Synthetic B");
    expect(input(I18N.settings.transcriptSearch).value).toBe("Synthetic");
  });

  it.each(["bilingual", "source", "translation"] as const)("copies only the passage content selected by the %s reading mode", async mode => {
    if (mode !== "bilingual") await choose(copy().reading, copy()[mode]);
    await click(button(`${copy().copy} #1`));
    expect(clipboard.write).toHaveBeenCalledOnce();
    await expect(clipboard.write.mock.calls[0][0]).resolves.toBe(
      mode === "source" ? "Synthetic current" : mode === "translation" ? "Fixture current" : "Synthetic current\nFixture current",
    );
    const text = host.querySelector(".session-transcript__pair")?.textContent ?? "";
    expect(text.includes("Synthetic current")).toBe(mode !== "translation");
    expect(text.includes("Fixture current")).toBe(mode !== "source");
    expect(host.querySelector(".settings-toast")?.textContent).toContain(copy().copied);
  });

  it("offers the exact chosen passage for manual copy after clipboard failure, then permits retry", async () => {
    await choose(copy().reading, copy().translation);
    clipboard.write.mockRejectedValueOnce(new Error("synthetic clipboard failure"));
    await click(button(`${copy().copy} #1`));
    const fallback = document.querySelector<HTMLTextAreaElement>(".session-workspace__manual-copy")!;
    expect(fallback.value).toBe("Fixture current");
    expect(fallback.readOnly).toBe(true);
    expect(document.querySelector(".settings-confirmation")?.textContent).toContain(copy().copyFailed);
    expect(host.textContent).not.toContain(copy().copied);
    await click(button(I18N.settings.cancel));
    await click(button(`${copy().copy} #1`));
    expect(clipboard.write).toHaveBeenCalledTimes(2);
    expect(document.querySelector(".settings-confirmation")).toBeNull();
    expect(host.querySelector(".settings-toast")?.textContent).toContain(copy().copied);
  });

  it("does not reopen failed passage copying after changing the selected session", async () => {
    const pending = deferred();
    clipboard.write.mockReturnValueOnce(pending.promise);
    await click(button(`${copy().copy} #1`));
    await select(1);
    await act(async () => pending.reject(new Error("old clipboard failure")));
    expect(document.querySelector(".session-workspace__manual-copy")).toBeNull();
    expect(host.textContent).toContain("Synthetic A");
  });

  it("exports the selected full session with chosen options and closes the dialog before the native picker completes", async () => {
    await select(1); await search("Synthetic");
    await choose(copy().reading, copy().source);
    await click(button(copy().export));
    expect(document.querySelector(".session-workspace__export-scope")?.textContent).toBe(copy().whole);
    expect(ipc.sessionExport).not.toHaveBeenCalled();
    const content = [...document.querySelectorAll('[role="combobox"]')].find(node => node.getAttribute("aria-label") === copy().content);
    expect(content?.textContent).toBe(copy().source);
    await choose(copy().format, copy().markdown);
    await choose(copy().content, copy().translation);
    await click(document.querySelector<HTMLElement>(`[role="switch"][aria-label="${copy().timestamps}"]`)!);
    const pending = deferred();
    ipc.sessionExport.mockReturnValueOnce(pending.promise);
    await click(button(copy().chooseFile));
    expect(document.querySelector(".settings-confirmation")).toBeNull();
    expect(ipc.sessionExport).toHaveBeenCalledExactlyOnceWith("transcript", "A", undefined,
      { format: "markdown", content: "translation", includeTimestamps: true });
    expect(input(I18N.settings.transcriptSearch).value).toBe("Synthetic");
    expect(ipc.sessionHistoryPage).toHaveBeenLastCalledWith("A", "Synthetic", 0);
    await act(async () => pending.resolve(false));
    expect(host.textContent).not.toContain(I18N.settings.sessionExportSaved);
    expect(host.textContent).toContain("Synthetic A");
  });

  it("canceling export never opens a native picker and a later export belongs to the new selection", async () => {
    await select(1); await click(button(copy().export));
    await choose(copy().format, copy().markdown);
    await click(button(I18N.settings.cancel));
    expect(ipc.sessionExport).not.toHaveBeenCalled();
    await select(2); await exportTranscript();
    expect(ipc.sessionExport).toHaveBeenCalledExactlyOnceWith("transcript", "B", undefined,
      { format: "markdown", content: "bilingual", includeTimestamps: false });
    expect(host.textContent).toContain("Synthetic B");
  });

  it("blocks a current-session export if capture starts while its options dialog is open", async () => {
    await click(button(copy().export));
    store.session.isActive = true;
    await render();
    expect(button(copy().chooseFile).disabled).toBe(true);
    await click(button(copy().chooseFile));
    expect(ipc.sessionExport).not.toHaveBeenCalled();
    await click(button(I18N.settings.cancel));
    await select(1);
    await exportTranscript();
    expect(ipc.sessionExport).toHaveBeenCalledExactlyOnceWith("transcript", "A", undefined, defaultExportOptions);
  });
});
