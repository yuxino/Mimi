// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { SettingsSessionControls } from "./SettingsSessionControls";

it("requires a running session to enter immersive without starting it, and always permits exit", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const onSessionChange = vi.fn(), onImmersiveChange = vi.fn(), onConfigure = vi.fn();
  const base = { checked: false, disabled: false, status: "idle" as const, statusText: "Not started", isActive: false, isChanging: false, immersive: false, canConfigure: false, actionFailed: false, nativeShortcuts: true, desktopShortcuts: null, onSessionChange, onResume: vi.fn(), onImmersiveChange, onConfigure };
  const controls = () => [...host.querySelectorAll<HTMLButtonElement>('button[role="switch"]')];
  try {
    await act(async () => root.render(<SettingsSessionControls {...base} />));
    expect(onSessionChange).not.toHaveBeenCalled();
    expect(onImmersiveChange).not.toHaveBeenCalled();
    expect(controls()[1].disabled).toBe(true);
    await act(async () => controls()[1].click());
    expect(onSessionChange).not.toHaveBeenCalled();
    expect(onImmersiveChange).not.toHaveBeenCalled();
    await act(async () => root.render(<SettingsSessionControls {...base} isActive checked />));
    expect(controls()[1].disabled).toBe(false);
    await act(async () => controls()[1].click());
    expect(onImmersiveChange).toHaveBeenLastCalledWith(true);
    expect(onSessionChange).not.toHaveBeenCalled();
    await act(async () => root.render(<SettingsSessionControls {...base} immersive isChanging />));
    expect(controls()[1].disabled).toBe(false);
    await act(async () => controls()[1].click());
    expect(onImmersiveChange).toHaveBeenLastCalledWith(false);
    await act(async () => root.render(<SettingsSessionControls {...base} disabled canConfigure />));
    expect(controls()[0].disabled).toBe(true);
    await act(async () => host.querySelector<HTMLButtonElement>('.settings-button')!.click());
    expect(onConfigure).toHaveBeenCalledOnce();
    for (const [agent, expected] of [["Macintosh", ["⌘⇧Space", "⌘⇧M"]], ["Windows", ["Ctrl+Shift+Space", "Ctrl+Shift+M"]], ["Linux X11", ["Ctrl+Shift+Space", "Ctrl+Shift+M"]]] as const) {
      vi.spyOn(navigator, "userAgent", "get").mockReturnValue(agent);
      await act(async () => root.render(<SettingsSessionControls {...base} />));
      expect([...host.querySelectorAll("kbd")].map(key => key.textContent)).toEqual(expected);
    }
    await act(async () => root.render(<SettingsSessionControls {...base} nativeShortcuts={false} />));
    expect(host.querySelector('kbd')).toBeNull();
  } finally {
    await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  }
});

it.each(["zh", "en", "ja"] as const)("keeps a direct retry and session switch in the compact service row in %s", async (language) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage(language);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const onSessionChange = vi.fn(), onImmersiveChange = vi.fn();
  const base = { compact: true, checked: false, disabled: false, status: "error" as const, statusText: "Synthetic session error", isActive: false, isChanging: false, immersive: false, canConfigure: false, actionFailed: false, nativeShortcuts: true, desktopShortcuts: null, onSessionChange, onResume: vi.fn(), onImmersiveChange, onConfigure: vi.fn() };
  try {
    await act(async () => root.render(<SettingsSessionControls {...base} />));
    expect(host.querySelectorAll('[role="switch"]')).toHaveLength(1);
    expect(host.querySelector("kbd")).not.toBeNull();
    const retry = host.querySelector<HTMLButtonElement>('.settings-session-control__actions .settings-button')!;
    expect(retry.textContent).toBe(I18N.settings.sessionRetry);
    await act(async () => retry.click());
    expect(onSessionChange).toHaveBeenCalledExactlyOnceWith(true);
    expect(onImmersiveChange).not.toHaveBeenCalled();
    await act(async () => root.render(<SettingsSessionControls {...base} status="connecting" statusText={I18N.settings.sessionConnecting} retrying disabled checked />));
    expect(retry.textContent).toBe(I18N.settings.sessionConnecting);
    expect(retry.disabled).toBe(true);
    await act(async () => retry.click());
    expect(onSessionChange).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("en"); vi.unstubAllGlobals();
  }
});

it.each(["zh", "en", "ja"] as const)("offers an explicit keyboard-reachable resume action with pending feedback in %s", async (language) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage(language);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const onResume = vi.fn(), onSessionChange = vi.fn();
  const base = { checked: true, disabled: false, status: "paused" as const, statusText: I18N.settings.sessionPaused, isActive: true, isChanging: false, immersive: false, canConfigure: false, actionFailed: false, nativeShortcuts: false, desktopShortcuts: null, onSessionChange, onResume, onImmersiveChange: vi.fn(), onConfigure: vi.fn() };
  try {
    for (const compact of [false, true]) {
      await act(async () => root.render(<SettingsSessionControls {...base} compact={compact} />));
      const resume = host.querySelector<HTMLButtonElement>(".settings-session-resume")!;
      expect(resume.textContent).toBe(I18N.settings.sessionResume);
      expect(resume.getAttribute("aria-describedby")).toBe("settings-session-status");
      expect(host.querySelector('[role="switch"]')?.getAttribute("aria-checked")).toBe("true");
      await act(async () => resume.focus());
      expect(document.activeElement).toBe(resume);
      await act(async () => resume.click());
      expect(onSessionChange).not.toHaveBeenCalled();
      await act(async () => root.render(<SettingsSessionControls {...base} compact={compact} resuming disabled isChanging />));
      expect(resume.textContent).toBe(I18N.settings.sessionResuming);
      expect(resume.disabled).toBe(true);
      expect(resume.getAttribute("aria-busy")).toBe("true");
      expect(resume.querySelector('[aria-hidden="true"]')).not.toBeNull();
      await act(async () => resume.click());
    }
    expect(onResume).toHaveBeenCalledTimes(2);
    await act(async () => root.render(<SettingsSessionControls {...base} resumeFailed />));
    expect(host.querySelector('[role="alert"]')?.textContent).toBe(I18N.settings.sessionResumeFailed);
    await act(async () => root.render(<SettingsSessionControls {...base} status="listening" statusText={I18N.settings.sessionListening} />));
    expect(host.querySelector(".settings-session-resume")).toBeNull();
  } finally {
    await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("en"); vi.unstubAllGlobals();
  }
});

it("keeps the permission cause and manual retry without a duplicate generic failure", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const onSessionChange = vi.fn(), onConfigure = vi.fn();
  const base = { checked: false, disabled: false, status: "error" as const, statusText: I18N.settings.sessionActionFailed,
    errorMessage: "Synthetic permission cause", permissionRequired: true, actionFailed: true, isActive: false, isChanging: false,
    immersive: false, canConfigure: false, nativeShortcuts: false, desktopShortcuts: null,
    onSessionChange, onResume: vi.fn(), onImmersiveChange: vi.fn(), onConfigure };
  try {
    await act(async () => root.render(<SettingsSessionControls {...base} />));
    expect(host.querySelectorAll('[role="alert"]')).toHaveLength(1);
    expect(host.querySelector('[role="alert"]')?.textContent).toContain(base.errorMessage);
    expect(host.querySelector('.session-error-feedback__action')?.textContent).not.toContain(I18N.settings.openSpeechSettings);
    const retry = host.querySelector<HTMLButtonElement>('.settings-session-control__actions .settings-button')!;
    expect(retry.disabled).toBe(false);
    expect(onSessionChange).not.toHaveBeenCalled();
    await act(async () => retry.click());
    expect(onSessionChange).toHaveBeenCalledExactlyOnceWith(true);
    expect(onConfigure).not.toHaveBeenCalled();
    await act(async () => root.render(<SettingsSessionControls {...base} permissionRequired={false} />));
    expect(host.querySelectorAll('[role="alert"]')).toHaveLength(2);
  } finally {
    await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();
  }
});
