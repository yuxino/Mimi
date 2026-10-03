// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SettingsView } from "./SettingsView";
import permissions from "../../../src-tauri/permissions/app.toml?raw";

import type { DesktopShortcutCommands } from "../../lib/ipc";

const shortcuts = vi.hoisted(() => ({ commands: null as DesktopShortcutCommands | null | undefined }));
vi.mock("../../lib/useDesktopShortcuts", () => ({ useDesktopShortcuts: () => ({ commands: shortcuts.commands, nativeShortcuts: shortcuts.commands === null }) }));

const initial = useStore.getState();
let host: HTMLDivElement;
let root: Root;
let togglePaused: ReturnType<typeof vi.fn>;
let start: ReturnType<typeof vi.fn>;
let stop: ReturnType<typeof vi.fn>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  Element.prototype.scrollTo = vi.fn();
  shortcuts.commands = null;
  setStoredUiLanguage("en");
  window.history.replaceState(null, "", "#subtitle-settings");
  togglePaused = vi.fn().mockResolvedValue(undefined);
  start = vi.fn().mockResolvedValue(undefined);
  stop = vi.fn().mockResolvedValue(undefined);
  useStore.setState({ ...initial, togglePaused, start, stop,
    settings: { ...initial.settings, profiles: initial.settings.profiles.map((profile) => ({ ...profile, credentialState: "present" })) },
    session: { ...initial.session, status: { kind: "listening" }, isActive: true, isPaused: true },
  }, true);
  host = document.createElement("div"); document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initial, true);
  setStoredUiLanguage("system");
  window.history.replaceState(null, "", window.location.pathname);
  vi.unstubAllGlobals();
});

const resumeButton = () => host.querySelector<HTMLButtonElement>(".settings-session-resume");
const sessionSwitch = () => host.querySelector<HTMLButtonElement>('.settings-session-card [role="switch"]')!;
async function mount() { await act(async () => root.render(<SettingsView />)); }
async function nativeResume() {
  await act(async () => useStore.setState({ session: { ...useStore.getState().session, isPaused: false } }));
}

it.each(["zh", "en", "ja"] as const)("resumes an existing paused session in subtitle settings in %s without starting a new session", async (language) => {
  setStoredUiLanguage(language);
  await mount();
  for (const category of ["subtitles"]) {
    await act(async () => host.querySelector<HTMLButtonElement>(`#settings-category-${category}`)!.click());
    expect(resumeButton()?.textContent).toBe(I18N.settings.sessionResume);
    expect(sessionSwitch().getAttribute("aria-checked")).toBe("true");
    expect(host.querySelector("#settings-session-status")?.textContent).toBe(I18N.settings.sessionPaused);
  }
  await act(async () => resumeButton()!.click());
  expect(togglePaused).toHaveBeenCalledOnce();
  expect(start).not.toHaveBeenCalled();
  expect(stop).not.toHaveBeenCalled();
  // IPC resolves before the native pause event. Neither statusKind nor
  // isActive changed, so the start/stop coordinator would never release this.
  expect(resumeButton()?.disabled).toBe(false);
  expect(sessionSwitch().disabled).toBe(false);
  await nativeResume();
  expect(resumeButton()).toBeNull();
  expect(host.querySelector("#settings-session-status")?.textContent).toBe(I18N.settings.sessionListening);
  expect(sessionSwitch().getAttribute("aria-checked")).toBe("true");
});

it("blocks duplicate resume and stop requests until IPC completes, even when only isPaused changes first", async () => {
  let complete!: () => void;
  togglePaused.mockImplementationOnce(() => new Promise<void>((resolve) => { complete = resolve; }));
  await mount();
  const resume = resumeButton()!, toggle = sessionSwitch();
  await act(async () => { resume.click(); resume.click(); toggle.click(); });
  expect(togglePaused).toHaveBeenCalledOnce();
  expect(stop).not.toHaveBeenCalled();
  expect(resume.disabled).toBe(true);
  expect(resume.getAttribute("aria-busy")).toBe("true");
  expect(resume.textContent).toBe(I18N.settings.sessionResuming);
  expect(toggle.disabled).toBe(true);
  await nativeResume();
  expect(host.querySelector("#settings-session-status")?.textContent).toBe(I18N.settings.sessionListening);
  expect(toggle.disabled).toBe(true);
  await act(async () => complete());
  expect(resumeButton()).toBeNull();
  expect(toggle.disabled).toBe(false);
  await act(async () => toggle.click());
  expect(stop).toHaveBeenCalledOnce();
});

it("unlocks a failed resume, offers retry, and clears its sanitized failure on a pause-only native transition", async () => {
  togglePaused.mockRejectedValueOnce(new Error("private raw IPC failure"));
  await mount();
  await act(async () => resumeButton()!.click());
  expect(resumeButton()?.disabled).toBe(false);
  expect(sessionSwitch().disabled).toBe(false);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.sessionResumeFailed);
  expect(host.textContent).not.toContain("private raw IPC failure");
  await act(async () => resumeButton()!.click());
  expect(togglePaused).toHaveBeenCalledTimes(2);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  togglePaused.mockRejectedValueOnce(new Error("another private failure"));
  await act(async () => resumeButton()!.click());
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await nativeResume();
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(resumeButton()).toBeNull();
});

it("does not revive a late resume failure after a tray or shortcut already resumed the session", async () => {
  let reject!: (reason: Error) => void;
  togglePaused.mockImplementationOnce(() => new Promise<void>((_resolve, fail) => { reject = fail; }));
  await mount();
  await act(async () => resumeButton()!.click());
  await nativeResume();
  await act(async () => reject(new Error("late rejected command")));
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(resumeButton()).toBeNull();
  expect(sessionSwitch().disabled).toBe(false);
});

it("keeps a pending stop authoritative and cannot resume a paused snapshot after stop was requested", async () => {
  await mount();
  const resume = resumeButton()!, toggle = sessionSwitch();
  await act(async () => { toggle.click(); resume.click(); });
  expect(stop).toHaveBeenCalledOnce();
  expect(togglePaused).not.toHaveBeenCalled();
  expect(toggle.disabled).toBe(true);
  await act(async () => useStore.setState({ session: { ...useStore.getState().session, status: { kind: "idle" }, isActive: false, isPaused: false } }));
  expect(resumeButton()).toBeNull();
  expect(toggle.disabled).toBe(false);
});

it.each(["rateLimited", "temporarilyUnavailable"] as const)("does not claim a scheduled retry when translation recovery %s is settled", async (reason) => {
  useStore.setState({ session: { ...useStore.getState().session, isPaused: false, translationRecovery: { reason, retryAfterMs: 0, retryScheduled: false } } });
  await mount();
  expect(host.querySelector("#settings-session-status")?.textContent).toBe(reason === "rateLimited" ? I18N.overlay.translationLimited : I18N.overlay.translationUnavailable);
});

it("permits the settings window to invoke the existing pause/resume command", () => {
  const settingsPermission = permissions.split("[[permission]]").find((entry) => entry.includes('identifier = "app-settings"'))!;
  expect(settingsPermission).toContain('"session_toggle_paused"');
});

it.each(["zh", "en", "ja"] as const)("offers the exact Linux commands in a General dialog and restores entry focus on close in %s", async language => {
  setStoredUiLanguage(language);
  const commands: DesktopShortcutCommands = {
    toggleSession: "'/opt/Mimi Preview.AppImage' --toggle-session",
    toggleImmersive: "'/opt/Mimi Preview.AppImage' --toggle-immersive",
    cycleSubtitleDisplay: "'/opt/Mimi Preview.AppImage' --cycle-subtitle-display",
  };
  shortcuts.commands = commands;
  await mount();
  expect(host.querySelector("#subtitle-settings-panel .settings-shortcut-setup, #service-profiles-panel .settings-shortcut-setup")).toBeNull();
  await act(() => host.querySelector<HTMLButtonElement>("#settings-category-general")!.click());
  const entry = host.querySelector<HTMLButtonElement>("#application-settings-panel .settings-shortcut-setup button")!;
  expect(entry.textContent).toBe(I18N.settings.systemShortcutSetup);
  await act(() => { entry.focus(); entry.click(); });
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
  expect(dialog.getAttribute("aria-modal")).toBe("true");
  expect(document.getElementById(dialog.getAttribute("aria-labelledby")!)?.textContent).toBe(I18N.settings.systemShortcutSetup);
  expect([...dialog.querySelectorAll("code")].map(code => code.textContent)).toEqual([commands.toggleSession, commands.toggleImmersive, commands.cycleSubtitleDisplay]);
  expect(dialog.textContent).toContain(I18N.settings.systemShortcutInstructions);
  expect(dialog.querySelectorAll(".settings-confirmation__actions button")).toHaveLength(1);
  const close = dialog.querySelector<HTMLButtonElement>(".settings-confirmation__cancel")!;
  expect(close.textContent).toBe(I18N.settings.closeDialog);
  await act(() => close.click());
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(document.activeElement).toBe(entry);
  await act(() => entry.click());
  await act(() => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(document.activeElement).toBe(entry);
  expect(start).not.toHaveBeenCalled();
  expect(stop).not.toHaveBeenCalled();
  expect(togglePaused).not.toHaveBeenCalled();
});

it.each([null, undefined])("does not advertise the Linux command entry when commands are %s", async commands => {
  shortcuts.commands = commands;
  await mount();
  await act(() => host.querySelector<HTMLButtonElement>("#settings-category-general")!.click());
  expect(host.querySelector(".settings-shortcut-setup")).toBeNull();
  expect(document.querySelector('[role="dialog"]')).toBeNull();
});
