// @vitest-environment jsdom
import { SettingsToastRegion } from "./SettingsToast";
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { SupportDiagnostics } from "./SupportDiagnostics";

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

it.each(["zh", "en", "ja"] as const)("keeps refresh, copy and feedback actions labeled and keyboard-reachable in %s", async (language) => {
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
  expect(host.querySelectorAll(".settings-diagnostic-summary > div")).toHaveLength(3);
  expect(host.querySelector(".settings-diagnostic-summary")?.textContent).toContain("0 ms");
  expect(host.querySelector(".settings-diagnostic-summary")?.textContent).toContain("125 ms");
  expect(host.querySelectorAll(".settings-diagnostic-events li")).toHaveLength(6);
  expect(host.querySelectorAll(".settings-diagnostic-preview .settings-diagnostic-events li")).toHaveLength(0);
  expect(host.querySelector("pre")).toBeNull();
  expect(host.querySelector<HTMLDetailsElement>(".settings-diagnostic-preview")?.open).toBe(false);
  const details = host.querySelector<HTMLDetailsElement>(".settings-diagnostic-preview")!;
  await act(() => { details.open = true; details.dispatchEvent(new Event("toggle")); });
  expect(host.querySelectorAll(".settings-diagnostic-events li")).toHaveLength(6);
  expect(host.querySelector("pre")?.textContent).toBe(snapshot("listening"));
  expect(host.querySelector(".settings-diagnostic-events__heading .settings-help-control__description")?.textContent).toBe("Since app start");
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
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not open GitHub");
  expect(host.textContent).not.toContain(tooManyBytes);
});
