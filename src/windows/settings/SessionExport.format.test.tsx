// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage, UI_LANGUAGES } from "../../lib/i18n";
import type { SessionHistoryItem, TranscriptPage } from "../../lib/types";
import { SessionExport } from "./SessionExport";

const ipc = vi.hoisted(() => ({
  sessionArchiveState: vi.fn(), sessionArchiveClear: vi.fn(), sessionExport: vi.fn(),
  sessionTranscriptPage: vi.fn(), sessionHistoryList: vi.fn(), sessionHistoryPage: vi.fn(),
  sessionHistoryAudio: vi.fn(), sessionHistoryDelete: vi.fn(),
}));
const store = vi.hoisted(() => ({
  session: { isActive: false },
  settings: { retainSessionHistory: true, recordSessionAudio: false, audioInput: "system" },
  saveSettings: vi.fn(),
}));
vi.mock("../../lib/ipc", () => ({ isTauri: true, setOverlayPointerCursor: vi.fn(), ...ipc }));
vi.mock("../../lib/store", () => ({
  useStore: Object.assign((select: (state: unknown) => unknown) => select(store), { getState: () => store }),
}));

const OriginalFormatter = Intl.DateTimeFormat;
const sessionOptions = { hour: "2-digit", minute: "2-digit" } as const;
const transcriptOptions = { ...sessionOptions, second: "2-digit" } as const;
const dayOptions = { year: "numeric", month: "long", day: "numeric" } as const;
const startedAtMs = new Date(2026, 9, 10, 12, 34, 56).getTime();
let archive: { transcriptCount: number; audioBytes: number };
let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 18, 12));
  vi.clearAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  archive = { transcriptCount: 30, audioBytes: 0 };
  ipc.sessionArchiveState.mockImplementation(async () => ({ ...archive }));
  ipc.sessionHistoryList.mockResolvedValue(Array.from({ length: 20 }, (_, index) => ({
    id: `synthetic-${index}`, count: 30, hasAudio: false, limited: false,
    startedAtMs: startedAtMs + index * 60_000, endedAtMs: startedAtMs + index * 60_000 + 30_000,
  } satisfies SessionHistoryItem)));
  ipc.sessionTranscriptPage.mockResolvedValue({ total: 30, page: 0, entries: Array.from({ length: 30 }, (_, index) => ({
    index: index + 1, source: "Synthetic source", translation: "Synthetic translation", createdAtMs: startedAtMs + index * 1_000,
  })) } satisfies TranscriptPage);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  vi.restoreAllMocks(); setStoredUiLanguage("system"); vi.unstubAllGlobals(); vi.useRealTimers();
});

async function mount() {
  await act(async () => root.render(<SessionExport visible />));
  await tick(0);
}
async function tick(ms = 1_000) {
  await act(async () => { await vi.advanceTimersByTimeAsync(ms); });
}

function observeFormatters(defaultTimeZone?: () => string) {
  const instances: Array<{ options: Intl.DateTimeFormatOptions | undefined; format: ReturnType<typeof vi.fn> }> = [];
  const constructor = vi.spyOn(Intl, "DateTimeFormat").mockImplementation(function(locales, options) {
    const formatter = new OriginalFormatter(locales, defaultTimeZone ? { ...options, timeZone: defaultTimeZone() } : options);
    const format = vi.fn(formatter.format);
    Object.defineProperty(formatter, "format", { value: format });
    instances.push({ options, format });
    return formatter;
  });
  return { constructor, instances };
}

it("bounds formatter work on unchanged archive polls without remounting history or delaying new state", async () => {
  await mount();
  const observer = observeFormatters();
  const historyButton = host.querySelector(".session-history__group button");
  const article = host.querySelector("article");
  const reads = ipc.sessionArchiveState.mock.calls.length;
  for (let index = 0; index < 5; index++) {
    observer.constructor.mockClear(); observer.instances.length = 0;
    await tick();
    expect(observer.constructor).toHaveBeenCalledTimes(3);
    expect(observer.instances.find(item => item.options?.year)?.format).toHaveBeenCalledOnce();
    expect(observer.instances.find(item => item.options?.hour && !item.options.second)?.format).toHaveBeenCalledTimes(20);
    expect(observer.instances.find(item => item.options?.second)?.format).toHaveBeenCalledTimes(30);
    expect(host.querySelector(".session-history__group button")).toBe(historyButton);
    expect(host.querySelector("article")).toBe(article);
  }
  expect(ipc.sessionArchiveState).toHaveBeenCalledTimes(reads + 5);
  expect(ipc.sessionTranscriptPage).toHaveBeenCalledOnce();
  archive.transcriptCount = 31;
  ipc.sessionTranscriptPage.mockResolvedValue({ total: 31, page: 0, entries: [{
    index: 31, source: "Synthetic new source", translation: "Synthetic new translation", createdAtMs: startedAtMs,
  }] } satisfies TranscriptPage);
  await tick(); await tick(0);
  expect(ipc.sessionTranscriptPage).toHaveBeenCalledTimes(2);
  expect(host.querySelector(".session-transcript__heading")?.textContent).toContain(I18N.settings.transcriptCount(31));
  expect(host.textContent).toContain("Synthetic new source");
});

it.each(UI_LANGUAGES)("preserves distinct date, session time and transcript time formats when switching to %s", async language => {
  await mount();
  const historyButton = host.querySelector(".session-history__group button");
  const article = host.querySelector("article");
  setStoredUiLanguage(language);
  await tick();
  expect(host.querySelector(".session-history__group h4")?.textContent).toBe(new OriginalFormatter(language, dayOptions).format(startedAtMs));
  expect(historyButton?.querySelector("span")?.textContent).toBe(new OriginalFormatter(language, sessionOptions).format(startedAtMs));
  expect(article?.querySelector("time")?.textContent).toBe(new OriginalFormatter(language, transcriptOptions).format(startedAtMs));
  expect(article?.querySelector("time")?.dateTime).toBe(new Date(startedAtMs).toISOString());
  expect(host.querySelector(".session-history__group button")).toBe(historyButton);
  expect(host.querySelector("article")).toBe(article);
});

it("recalculates Today and Yesterday after local midnight on the existing poll", async () => {
  const now = new Date(2026, 9, 18, 23, 59, 59, 500);
  vi.setSystemTime(now);
  ipc.sessionHistoryList.mockResolvedValue([
    { id: "today", count: 1, hasAudio: false, startedAtMs: new Date(2026, 9, 18, 12).getTime() },
    { id: "yesterday", count: 1, hasAudio: false, startedAtMs: new Date(2026, 9, 17, 12).getTime() },
  ]);
  await mount();
  const headings = () => [...host.querySelectorAll(".session-history__group h4")].map(node => node.textContent);
  expect(headings()).toEqual([I18N.settings.historyToday, I18N.settings.historyYesterday]);
  await tick();
  expect(headings()).toEqual([
    I18N.settings.historyYesterday,
    new OriginalFormatter("en", dayOptions).format(new Date(2026, 9, 17, 12)),
  ]);
});

it("uses fresh default-time-zone formatters on a later poll", async () => {
  let timeZone = "UTC";
  observeFormatters(() => timeZone);
  await mount();
  const sessionTime = () => host.querySelector(".session-history__group button span")?.textContent;
  const transcriptTime = () => host.querySelector("article time")?.textContent;
  expect(sessionTime()).toBe(new OriginalFormatter("en", { ...sessionOptions, timeZone }).format(startedAtMs));
  timeZone = "Asia/Tokyo";
  await tick();
  expect(sessionTime()).toBe(new OriginalFormatter("en", { ...sessionOptions, timeZone }).format(startedAtMs));
  expect(transcriptTime()).toBe(new OriginalFormatter("en", { ...transcriptOptions, timeZone }).format(startedAtMs));
  expect(host.querySelector(".session-history__group h4")?.textContent).toBe(new OriginalFormatter("en", { ...dayOptions, timeZone }).format(startedAtMs));
});

it("does not construct date formatters for an empty archive and history", async () => {
  archive.transcriptCount = 0;
  ipc.sessionHistoryList.mockResolvedValue([]);
  const observer = observeFormatters();
  await mount();
  await tick();
  expect(observer.constructor).not.toHaveBeenCalled();
});
