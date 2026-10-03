// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ApplicationAudio } from "./ApplicationAudio";
import { useStore } from "../../lib/store";
import { mergeSettingsSnapshot } from "../../lib/settingsState";
import { applicationAudioCopy } from "../../lib/applicationAudio";
import { setStoredUiLanguage } from "../../lib/i18n";
import type { SessionStateEvent } from "../../lib/types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../lib/ipc", async importOriginal => ({ ...await importOriginal<typeof import("../../lib/ipc")>(), isTauri: true }));
const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
const save = vi.fn();
const applications = [{ id: "com.example.player", name: "Player" }, { id: "com.example.chat", name: "Chat" }];
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("navigator", { userAgent: "Macintosh" });
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  vi.mocked(invoke).mockReset(); vi.mocked(invoke).mockResolvedValue({ supported: true, applications });
  save.mockReset(); save.mockImplementation(async draft => useStore.setState({ settings: mergeSettingsSnapshot(useStore.getState().settings, draft) }));
  useStore.setState({ ...initial, initializationStatus: "ready", saveSettings: save,
    settings: { ...initial.settings, systemAudioTarget: { kind: "system" }, recordSessionAudio: true },
    session: { ...initial.session, status: { kind: "idle" }, isActive: false, isPaused: false },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
async function render() { await act(async () => root.render(<ApplicationAudio />)); }
async function choose(label: string, option: string) {
  await act(async () => host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!.click());
  const node = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === option)!;
  expect(node).toBeDefined();
  await act(async () => node.click());
}
async function browse() { const text = applicationAudioCopy(); await choose(text.title, text.app); }

it.each(["en", "zh", "ja"] as const)("browses without saving or capture permission requests on initial render in %s", async language => {
  setStoredUiLanguage(language); await render();
  expect(invoke).not.toHaveBeenCalled();
  expect(host.textContent).toContain(applicationAudioCopy().all);
  await browse();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("audio_applications");
  expect(save).not.toHaveBeenCalled();
  await choose(applicationAudioCopy().application, "Player");
  expect(save).toHaveBeenCalledExactlyOnceWith({ systemAudioTarget: { kind: "application", ...applications[0] } });
  expect(useStore.getState().settings.recordSessionAudio).toBe(false);
});

it("searches applications and can return to all system audio", async () => {
  await render(); await browse();
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="Application"]')!.click());
  const input = document.querySelector<HTMLInputElement>('input[aria-label="Search applications"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "does-not-match");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(document.body.textContent).toContain(applicationAudioCopy().noMatch);
  expect(document.body.textContent).not.toContain(applicationAudioCopy().empty);
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "chat");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual(["Chat"]);
  await act(async () => document.querySelector<HTMLElement>('[role="option"]')!.click());
  await choose(applicationAudioCopy().title, applicationAudioCopy().all);
  expect(useStore.getState().settings.systemAudioTarget).toEqual({ kind: "system" });
});

it("refreshes a missing selection without silently switching to another app", async () => {
  useStore.setState({ settings: { ...useStore.getState().settings, systemAudioTarget: { kind: "application", id: "gone", name: "Old player" } } });
  await render();
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === "Refresh")!.click());
  expect(host.textContent).toContain("Old player · Unavailable");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().unavailable);
  expect(save).not.toHaveBeenCalled();
  await choose(applicationAudioCopy().application, "Player");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("reports list failures safely and allows an explicit retry and empty result", async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error("private-window-title"));
  await render(); await browse();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().failed);
  expect(host.textContent).not.toContain("private-window-title");
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [] });
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === "Refresh")!.click());
  expect(host.textContent).toContain(applicationAudioCopy().empty);
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("keeps a rejected selection reviewable and blocks duplicate saves", async () => {
  let reject!: (reason: Error) => void;
  save.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  await render(); await browse(); await choose(applicationAudioCopy().application, "Player");
  expect(host.querySelector<HTMLButtonElement>('button[aria-label="Application"]')?.disabled).toBe(true);
  expect(save).toHaveBeenCalledOnce();
  await act(async () => reject(new Error("private-error")));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().saveFailed);
  expect(useStore.getState().settings.systemAudioTarget.kind).toBe("system");
  await choose(applicationAudioCopy().application, "Chat");
  expect(useStore.getState().settings.systemAudioTarget).toMatchObject({ kind: "application", name: "Chat" });
});

it.each([
  { status: { kind: "listening" }, isActive: true, isPaused: false },
  { status: { kind: "listening" }, isActive: false, isPaused: true },
  { status: { kind: "connecting" }, isActive: false, isPaused: false },
  { status: { kind: "stopping" }, isActive: false, isPaused: false },
] satisfies Pick<SessionStateEvent, "status" | "isActive" | "isPaused">[])("locks target changes while the session owns capture: %j", async state => {
  useStore.setState({ session: { ...initial.session, ...state }, settings: { ...initial.settings, systemAudioTarget: { kind: "application", ...applications[0] } } });
  await render();
  expect(host.querySelector<HTMLButtonElement>('button[aria-label="Capture sound from"]')?.disabled).toBe(true);
  expect(host.querySelector<HTMLButtonElement>('button[aria-label="Application"]')?.disabled).toBe(true);
  expect(save).not.toHaveBeenCalled();
});

it("leaves unsupported platforms on system audio and provides help", async () => {
  vi.stubGlobal("navigator", { userAgent: "Linux" }); await render();
  expect(host.textContent).not.toContain(applicationAudioCopy().app);
  expect(host.querySelector('.settings-help-control__description')?.textContent).toContain(applicationAudioCopy().unsupported);
  expect(invoke).not.toHaveBeenCalled();
});
