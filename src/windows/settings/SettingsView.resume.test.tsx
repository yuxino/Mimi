// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { SettingsView } from "./SettingsView";
import type { SessionStateEvent } from "../../lib/types";
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

async function mount() { await act(async () => root.render(<SettingsView />)); }

function expectNoHeaderSessionControls() {
  expect(host.querySelector(".settings-session-card, #settings-session-title, #settings-session-status, .settings-session-resume")).toBeNull();
  expect(host.querySelector(".settings-page-header [role=switch], .settings-page-header kbd")).toBeNull();
}

it("keeps session controls out of the shared settings header across lifecycle changes", async () => {
  await mount();
  const states: Pick<SessionStateEvent, "status" | "isActive" | "isPaused">[] = [
    { status: { kind: "idle" }, isActive: false, isPaused: false },
    { status: { kind: "connecting" }, isActive: false, isPaused: false },
    { status: { kind: "listening" }, isActive: true, isPaused: false },
    { status: { kind: "listening" }, isActive: true, isPaused: true },
    { status: { kind: "stopping" }, isActive: true, isPaused: false },
    { status: { kind: "error", message: "synthetic-session-error" }, isActive: false, isPaused: false },
  ];
  for (const state of states) {
    await act(() => useStore.setState({ session: { ...initial.session, ...state } }));
    expectNoHeaderSessionControls();
  }
  // Category/localization coverage belongs to SettingsNavigation.test.tsx.
  await act(() => host.querySelector<HTMLButtonElement>("#settings-category-service")!.click());
  expectNoHeaderSessionControls();
  expect(togglePaused).not.toHaveBeenCalled();
  expect(start).not.toHaveBeenCalled();
  expect(stop).not.toHaveBeenCalled();
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
