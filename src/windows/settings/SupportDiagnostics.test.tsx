// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, UI_LANGUAGES, setStoredUiLanguage } from "../../lib/i18n";
import { SupportDiagnostics } from "./SupportDiagnostics";
import { diagnosticsCopy } from "./diagnosticsCopy";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), clipboard: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../../lib/ipc", () => ({ isTauri: true, setOverlayPointerCursor: vi.fn() }));
vi.mock("../../lib/diagnosticClipboard", () => ({ writeDiagnosticClipboard: mocks.clipboard }));

let host: HTMLDivElement;
let root: Root;
const report = "Mimi diagnostics\nStatus: idle\nRecent audio frames: 0";

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  mocks.invoke.mockResolvedValue(report);
  mocks.clipboard.mockImplementation(async (value: Promise<string>) => { await value; });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  setStoredUiLanguage("system");
  vi.resetAllMocks();
  vi.unstubAllGlobals();
});
async function mount() { await act(async () => root.render(<><SupportDiagnostics /><SettingsToastRegion /></>)); }
async function copy() { await act(async () => host.querySelector<HTMLButtonElement>('[data-action="copy"]')!.click()); }
async function openDetails() {
  const details = host.querySelector<HTMLDetailsElement>("details")!;
  await act(() => { details.open = true; details.dispatchEvent(new Event("toggle")); });
}

it("prepares diagnostics only after an explicit action while its page is inactive, then offers a collapsed safe report", async () => {
  await mount();
  expect(mocks.invoke).not.toHaveBeenCalled();
  expect(host.querySelector("details, summary, textarea")).toBeNull();
  await copy();
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("support_diagnostics");
  expect(mocks.clipboard).toHaveBeenCalledOnce();
  expect(host.querySelector('[role="status"]')?.textContent).toContain("Copied successfully");
  expect(host.querySelector("textarea")).toBeNull();
  expect(host.querySelector<HTMLDetailsElement>("details")?.open).toBe(false);
  expect(host.querySelector("pre")).toBeNull();
  await openDetails();
  expect(host.querySelector("pre")?.textContent).toBe(report);
});

it("offers a selectable report directly when clipboard access fails", async () => {
  mocks.clipboard.mockRejectedValue(new Error("synthetic-clipboard-denial"));
  await mount();
  await copy();
  expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe(report);
  expect(host.querySelector<HTMLTextAreaElement>("textarea")?.readOnly).toBe(true);
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not copy");
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Dismiss notification"]')!.click());
  expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe(report);
  expect(host.querySelector<HTMLDetailsElement>("details")?.open).toBe(true);
});

it("reports preparation failure without exposing a previous or invalid report", async () => {
  mocks.invoke.mockRejectedValue(new Error("synthetic-diagnostic-failure"));
  await mount();
  await copy();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not prepare diagnostics");
  expect(host.querySelector("textarea")).toBeNull();
});

it("keeps the existing reviewed public feedback action", async () => {
  mocks.invoke.mockResolvedValue({ report, requiresPaste: false });
  await mount();
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="issue"]')!.click());
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("app_open_support_issue");
  expect(host.querySelector('[role="status"]')?.textContent).toContain("Review the report");
});

it.each(["zh", "zh-TW", "en", "ja", "de", "fr", "ko", "th", "ru"] as const)("keeps refresh, copy and feedback actions labeled and keyboard-reachable in %s", async (language) => {
  setStoredUiLanguage(language);
  await mount();
  expect(host.querySelector(".settings-diagnostic-privacy")).toBeNull();
  expect(host.querySelector(".settings-support-diagnostics > p.settings-caption")).toBeNull();
  const buttons = [...host.querySelectorAll<HTMLButtonElement>(".settings-support-diagnostics__actions button")];
  expect(buttons).toHaveLength(3);
  for (const button of buttons) {
    expect(button.classList.contains("settings-sidebar-action")).toBe(false);
    expect(button.classList.contains("settings-button") || button.classList.contains("settings-link")).toBe(true);
    expect(button.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
    expect(button.querySelector("span")?.textContent).toBeTruthy();
    await act(async () => button.focus());
    expect(document.activeElement).toBe(button);
  }
  expect(mocks.invoke).not.toHaveBeenCalled();
  expect(mocks.clipboard).not.toHaveBeenCalled();
});

it("keeps diagnostic preparation guarded while its compact copy action is pending", async () => {
  let complete!: (report: string) => void;
  mocks.invoke.mockImplementationOnce(() => new Promise<string>((resolve) => { complete = resolve; }));
  await mount();
  const buttons = [...host.querySelectorAll<HTMLButtonElement>(".settings-support-diagnostics__actions button")];
  const copyButton = host.querySelector<HTMLButtonElement>('[data-action="copy"]')!;
  await act(async () => { copyButton.click(); copyButton.click(); buttons[0].click(); });
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("support_diagnostics");
  expect(buttons.every((button) => button.disabled)).toBe(true);
  expect(host.querySelector("section")?.getAttribute("aria-busy")).toBe("true");
  await act(async () => complete(report));
  expect(buttons.every((button) => !button.disabled)).toBe(true);
  expect(mocks.clipboard).toHaveBeenCalledOnce();
});

it("loads a safe snapshot on entry, keeps recent events visible independently of raw data, and refreshes only on request", async () => {
  const snapshot = (status: string) => JSON.stringify({
    schema: "mimi.support.v2", session_status: status,
    service: { api_round_trip_ms: 0, translation_duration_ms: 125, translation_duration_kind: "follow" },
    last_error: { classification: { category: "rate_limit", code: "TRANSLATION_RATE_LIMITED" } },
    journal: { recent_events: Array.from({ length: 9 }, (_, index) => ({ sequence: index, elapsed_since_app_start_ms: index * 1000, kind: "status", status: "listening" })) },
  });
  mocks.invoke.mockResolvedValue(snapshot("listening"));
  await act(async () => root.render(<><SupportDiagnostics visible={false} /><SettingsToastRegion /></>));
  expect(mocks.invoke).not.toHaveBeenCalled();
  await act(async () => root.render(<><SupportDiagnostics visible /><SettingsToastRegion /></>));
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("support_diagnostics");
  expect(host.querySelectorAll(".settings-diagnostic-summary > div")).toHaveLength(1);
  expect(host.querySelector('[data-fact="measurements"]')?.textContent).toContain("0 ms");
  expect(host.querySelector('[data-fact="measurements"]')?.textContent).toContain("125 ms");
  expect(host.querySelectorAll(".settings-diagnostic-events li")).toHaveLength(6);
  expect(host.querySelectorAll(".settings-diagnostic-preview .settings-diagnostic-events li")).toHaveLength(0);
  expect(host.querySelector("pre")).toBeNull();
  expect(host.querySelector<HTMLDetailsElement>(".settings-diagnostic-preview")?.open).toBe(false);
  const details = host.querySelector<HTMLDetailsElement>(".settings-diagnostic-preview")!;
  await act(() => { details.open = true; details.dispatchEvent(new Event("toggle")); });
  expect(host.querySelectorAll(".settings-diagnostic-events li")).toHaveLength(6);
  expect(host.querySelector("pre")?.textContent).toBe(snapshot("listening"));
  expect(host.querySelector(".settings-diagnostic-events__heading .settings-help-control__description")?.textContent).toBe(diagnosticsCopy.en.eventsHelp);
  expect(host.querySelector(".settings-diagnostic-events__heading > span:not(.settings-help-control)")).toBeNull();
  expect(mocks.invoke).toHaveBeenCalledOnce();
  await act(() => { details.open = false; details.dispatchEvent(new Event("toggle")); });
  expect(host.querySelectorAll(".settings-diagnostic-events li")).toHaveLength(6);
  expect(host.querySelector("pre")).toBeNull();
  await act(async () => root.render(<><SupportDiagnostics visible /><SettingsToastRegion /></>));
  expect(mocks.invoke).toHaveBeenCalledOnce();
  mocks.invoke.mockResolvedValue(snapshot("paused"));
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="refresh"]')!.click());
  expect(mocks.invoke).toHaveBeenCalledTimes(2);
  expect(host.querySelector(".settings-diagnostic-summary")?.textContent).toContain(I18N.settings.sessionPaused);
  expect(mocks.clipboard).not.toHaveBeenCalled();
  await act(async () => root.render(<><SupportDiagnostics visible={false} /><SettingsToastRegion /></>));
  expect(mocks.invoke).toHaveBeenCalledTimes(2);
});

it("loads a fresh snapshot under StrictMode without duplicating the disposed entry effect", async () => {
  await act(async () => root.render(<><StrictMode><SupportDiagnostics visible /></StrictMode><SettingsToastRegion /></>));
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("support_diagnostics");
  expect(host.querySelector("pre")).toBeNull();
  await openDetails();
  expect(host.querySelector("pre")?.textContent).toBe(report);
  expect(host.querySelector('button[aria-busy="true"]')).toBeNull();
});

it.each([false, true])("queues one fresh entry read after a previous visit's pending diagnostics settles (failure=%s)", async failure => {
  let resolve!: (report: string) => void;
  let reject!: (error: Error) => void;
  mocks.invoke.mockImplementationOnce(() => new Promise<string>((done, fail) => { resolve = done; reject = fail; }));
  mocks.invoke.mockResolvedValue(JSON.stringify({ schema: "mimi.support.v2", session_status: "error" }));
  await act(async () => root.render(<><SupportDiagnostics visible /><SettingsToastRegion /></>));
  for (let visit = 0; visit < 2; visit += 1) {
    await act(async () => root.render(<><SupportDiagnostics visible={false} /><SettingsToastRegion /></>));
    await act(async () => root.render(<><SupportDiagnostics visible /><SettingsToastRegion /></>));
  }
  expect(mocks.invoke).toHaveBeenCalledOnce();
  await act(async () => {
    if (failure) reject(new Error("old request failed"));
    else resolve(JSON.stringify({ schema: "mimi.support.v2", session_status: "idle" }));
  });
  expect(mocks.invoke).toHaveBeenCalledTimes(2);
  expect(host.querySelector(".settings-diagnostic-summary")?.textContent).toContain(I18N.settings.sessionError);
  expect(host.querySelector(".settings-toast")).toBeNull();
});

it("discards a previous visit's report without refreshing an inactive page", async () => {
  let complete!: (report: string) => void;
  mocks.invoke.mockImplementationOnce(() => new Promise<string>(resolve => { complete = resolve; }));
  await act(async () => root.render(<><SupportDiagnostics visible /><SettingsToastRegion /></>));
  await act(async () => root.render(<><SupportDiagnostics visible={false} /><SettingsToastRegion /></>));
  await act(async () => complete(JSON.stringify({ schema: "mimi.support.v2", session_status: "idle" })));
  expect(host.querySelector(".settings-diagnostic-summary")).toBeNull();
  expect(mocks.invoke).toHaveBeenCalledOnce();
});

it("accepts reports up to 16KiB of UTF-8 and rejects oversized reads or feedback reports without displaying them", async () => {
  const fullReport = "x".repeat(16_384);
  mocks.invoke.mockResolvedValue(fullReport);
  await mount();
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="refresh"]')!.click());
  expect(host.querySelector("pre")).toBeNull();
  await openDetails();
  expect(host.querySelector("pre")?.textContent).toBe(fullReport);
  expect(host.querySelector(".settings-diagnostic-summary")).toBeNull();
  const tooManyBytes = "診".repeat(6_000);
  mocks.invoke.mockResolvedValue(tooManyBytes);
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="refresh"]')!.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not prepare diagnostics");
  expect(host.querySelector("pre")?.textContent).toBe(fullReport);
  mocks.invoke.mockResolvedValue({ report: tooManyBytes, requiresPaste: true });
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="issue"]')!.click());
  expect(host.querySelector(".settings-toast")?.textContent).toContain("Could not open GitHub");
  expect(host.textContent).not.toContain(tooManyBytes);
});


it.each([
  [{}, false, false],
  [{ api_round_trip_ms: 0 }, true, false],
  [{ translation_duration_ms: 0, translation_duration_kind: "request" }, false, true],
  [{ translation_duration_ms: 100, translation_duration_kind: "unknown" }, false, false],
  [{ api_round_trip_ms: 47, translation_duration_ms: 100, translation_duration_kind: "follow" }, true, true],
] as const)("shows only measured diagnostic values for %j", async (service, api, translation) => {
  mocks.invoke.mockResolvedValue(JSON.stringify({ schema: "mimi.support.v2", session_status: "listening", service }));
  await act(async () => root.render(<><SupportDiagnostics visible /><SettingsToastRegion /></>));
  const summary = host.querySelector('[data-fact="measurements"]')!;
  expect(summary.textContent?.includes(I18N.overlay.apiLatency)).toBe(api);
  expect(summary.textContent?.includes("translation_duration_kind" in service && service.translation_duration_kind === "follow" ? I18N.overlay.translationFollowLatency : I18N.overlay.translationLatency)).toBe(translation);
});


it.each(["idle", "connecting", "stopping", "paused", "error"])("hides stale diagnostic timings in %s reports", async session_status => {
  mocks.invoke.mockResolvedValue(JSON.stringify({ schema: "mimi.support.v2", session_status,
    service: { api_round_trip_ms: 47, translation_duration_ms: 100, translation_duration_kind: "request" } }));
  await act(async () => root.render(<><SupportDiagnostics visible /><SettingsToastRegion /></>));
  expect(host.querySelectorAll(".settings-diagnostic-summary > div")).toHaveLength(1);
  expect(host.querySelector('[data-fact="measurements"] strong')).toBeNull();
  expect(host.querySelector('[data-fact="measurements"] dd')?.textContent).toBe(diagnosticsCopy.en.notObserved);
});

const safeReport = (changes: Record<string, unknown> = {}) => JSON.stringify({
  schema: "mimi.support.v2", session_status: "listening",
  snapshot: { elapsed_since_app_start_ms: 12000, audio_recent_window_ms: 2000 },
  capture: { observation: { pcm_data_recent: true, sound_recent: false, observation_age_ms: 0 } },
  service: { translation_pending: false, translation_preview_pending: false, translation_timed_out: false },
  journal: { recent_events: [] }, ...changes,
});

it.each(["idle", "paused"])("does not add unrelated setup advice for a %s session", async session_status => {
  mocks.invoke.mockResolvedValue(safeReport({ session_status }));
  await act(async () => root.render(<SupportDiagnostics visible onOpenAudio={vi.fn()} onOpenServices={vi.fn()} />));
  expect(host.querySelector(".diagnostics-next-step")).toBeNull();
  expect(host.querySelector(".settings-diagnostic-status")?.textContent).toBe(session_status === "idle" ? I18N.settings.sessionIdle : I18N.settings.sessionPaused);
});

it("renders silent input as an observation and never invokes a probe or capture action", async () => {
  mocks.invoke.mockResolvedValue(safeReport());
  const openAudio = vi.fn();
  await act(async () => root.render(<SupportDiagnostics visible onOpenAudio={openAudio} />));
  expect(host.querySelector('[data-fact="audio"] dd')?.textContent).toBe(diagnosticsCopy.en.quiet);
  expect(host.querySelector('[data-action="open-audio"]')).toBeNull();
  expect(host.querySelector('[data-fact="records"] dd')?.textContent).not.toContain("0");
  expect(host.querySelector(".diagnostics-updated time")).not.toBeNull();
  expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("support_diagnostics");
  expect(openAudio).not.toHaveBeenCalled();
});

it("shows a supported permission failure with a navigation action, and leaves user stops neutral", async () => {
  const openAudio = vi.fn();
  mocks.invoke.mockResolvedValue(safeReport({ session_status: "error", last_error: {
    classification: { category: "permission" }, observed_ago_ms: 3000,
  } }));
  await act(async () => root.render(<SupportDiagnostics visible onOpenAudio={openAudio} />));
  expect(host.querySelector(".diagnostics-last-error")?.textContent).toContain(diagnosticsCopy.en.permission);
  await act(() => host.querySelector<HTMLButtonElement>('[data-action="open-audio"]')!.click());
  expect(openAudio).toHaveBeenCalledOnce();
  expect(mocks.invoke).toHaveBeenCalledOnce();
  mocks.invoke.mockResolvedValue(safeReport({ session_status: "error", last_error: {
    classification: { category: "user_stopped" }, observed_ago_ms: 0,
  } }));
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="refresh"]')!.click());
  expect(host.querySelector(".diagnostics-last-error.is-current")).toBeNull();
  expect(host.querySelector('[data-action="open-audio"]')).toBeNull();
});

it("shows a past error with the later recovery instead of an active failure or service action", async () => {
  mocks.invoke.mockResolvedValue(safeReport({
    last_error: { classification: { category: "authentication" }, observed_ago_ms: 6000 },
    recent_recovery: { action: "recovered", observed_ago_ms: 1000 },
  }));
  await act(async () => root.render(<SupportDiagnostics visible onOpenServices={vi.fn()} />));
  expect(host.querySelector(".diagnostics-last-error")?.textContent).toContain(diagnosticsCopy.en.recoveredAfter);
  expect(host.querySelector(".diagnostics-last-error.is-current")).toBeNull();
  expect(host.querySelector('[data-action="open-services"]')).toBeNull();
});

it("shows current recovery from the snapshot and only offers service navigation when retry is not scheduled", async () => {
  const openServices = vi.fn();
  const current = (retryScheduled: boolean) => safeReport({ service: { translation_recovery: { reason: "rateLimited", retryScheduled } } });
  mocks.invoke.mockResolvedValue(current(true));
  await act(async () => root.render(<SupportDiagnostics visible onOpenServices={openServices} />));
  expect(host.querySelector('[data-fact="translation"] dd')?.textContent).toBe(I18N.overlay.translationRateLimited);
  expect(host.querySelector('[data-action="open-services"]')).toBeNull();
  mocks.invoke.mockResolvedValue(current(false));
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="refresh"]')!.click());
  expect(host.querySelector('[data-fact="translation"] dd')?.textContent).toBe(I18N.overlay.translationLimited);
  await act(() => host.querySelector<HTMLButtonElement>('[data-action="open-services"]')!.click());
  expect(openServices).toHaveBeenCalledOnce();
});

it("expands bounded events without fetching and keeps raw diagnostics separately collapsed", async () => {
  mocks.invoke.mockResolvedValue(safeReport({ journal: { events_omitted: 4, recent_events: Array.from({ length: 24 }, (_, sequence) => ({
    sequence, elapsed_since_app_start_ms: sequence * 100, kind: "status", status: "listening",
  })) } }));
  await act(async () => root.render(<SupportDiagnostics visible />));
  expect(host.querySelectorAll(".settings-diagnostic-events li")).toHaveLength(6);
  expect(host.querySelector(".settings-diagnostic-events__time")?.textContent).toContain("ago");
  const expand = host.querySelector<HTMLButtonElement>('[aria-controls="diagnostic-recent-events"]')!;
  await act(() => expand.click());
  expect(expand.getAttribute("aria-expanded")).toBe("true");
  expect(host.querySelectorAll(".settings-diagnostic-events li")).toHaveLength(24);
  expect(host.querySelector(".diagnostics-event-footer")?.textContent).toContain(`${diagnosticsCopy.en.omitted}: 4`);
  expect(host.querySelector<HTMLDetailsElement>(".settings-diagnostic-preview")?.open).toBe(false);
  expect(mocks.invoke).toHaveBeenCalledOnce();
});

it("keeps the previous snapshot identified after a failed refresh and clears that state on retry", async () => {
  mocks.invoke.mockResolvedValue(safeReport());
  await act(async () => root.render(<SupportDiagnostics visible />));
  const updated = host.querySelector("time")?.getAttribute("dateTime");
  mocks.invoke.mockRejectedValue(new Error("private failure"));
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="refresh"]')!.click());
  expect(host.querySelector(".diagnostics-read-state")?.textContent).toBe(diagnosticsCopy.en.staleRead);
  expect(host.querySelector("time")?.getAttribute("dateTime")).toBe(updated);
  expect(host.textContent).not.toContain("private failure");
  mocks.invoke.mockResolvedValue(safeReport({ session_status: "paused" }));
  await act(async () => host.querySelector<HTMLButtonElement>('[data-action="refresh"]')!.click());
  expect(host.querySelector(".diagnostics-read-state")).toBeNull();
  expect(host.querySelector(".settings-diagnostic-status")?.textContent).toBe(I18N.settings.sessionPaused);
});

it.each(UI_LANGUAGES)("renders a runtime recognition failure with a neutral status and local category in %s", async language => {
  setStoredUiLanguage(language);
  const expected = {
    zh: ["字幕发生错误", "语音识别失败"],
    "zh-TW": ["字幕發生錯誤", "語音辨識失敗"],
    en: ["Subtitle error", "Speech recognition failed"],
    ja: ["字幕でエラーが発生しました", "音声認識に失敗"],
    de: ["Untertitelfehler", "Spracherkennung fehlgeschlagen"],
    ko: ["자막 오류", "음성 인식 실패"],
    fr: ["Erreur des sous-titres", "Échec de la reconnaissance vocale"],
    th: ["คำบรรยายเกิดข้อผิดพลาด", "การรู้จำเสียงไม่สำเร็จ"],
    ru: ["Ошибка субтитров", "Ошибка распознавания речи"],
  }[language];
  const classification = { category: "recognition", phase: "asr_recognition", code: "APPLE_SPEECH_RECOGNITION_FAILED", description: "private-native-description" };
  mocks.invoke.mockResolvedValue(JSON.stringify({
    schema: "mimi.support.v2", session_status: "error", last_error: { classification },
    journal: { recent_events: [
      { sequence: 1, elapsed_since_app_start_ms: 100, kind: "status", status: "listening" },
      { sequence: 2, elapsed_since_app_start_ms: 707_700, kind: "failure", classification },
      { sequence: 3, elapsed_since_app_start_ms: 707_701, kind: "status", status: "error" },
    ] },
  }));
  await act(async () => root.render(<SupportDiagnostics visible />));
  expect(host.querySelector(".settings-diagnostic-status")?.textContent).toBe(expected[0]);
  expect(host.querySelector(".diagnostics-last-error strong")?.textContent).toBe(expected[1]);
  const events = [...host.querySelectorAll(".settings-diagnostic-events li")].map(event => event.textContent);
  expect(events).toEqual([`0.1 s${I18N.settings.sessionListening}`, `707.7 s${expected[1]}`, `707.7 s${expected[0]}`]);
  expect(host.textContent).not.toContain("private-native-description");
  expect(host.textContent).not.toContain("APPLE_SPEECH_RECOGNITION_FAILED");
});
