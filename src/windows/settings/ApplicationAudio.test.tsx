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
import type { SessionStateEvent, SystemAudioTarget } from "../../lib/types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../lib/ipc", async importOriginal => ({ ...await importOriginal<typeof import("../../lib/ipc")>(), isTauri: true }));
const initial = useStore.getState();
let host: HTMLDivElement, root: Root;
const switchTarget = vi.fn();
const save = vi.fn();
const applications = [{ id: "com.example.player", name: "Player" }, { id: "com.example.chat", name: "Chat" }];
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("navigator", { userAgent: "Macintosh" });
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  vi.mocked(invoke).mockReset(); vi.mocked(invoke).mockResolvedValue({ supported: true, applications });
  save.mockReset();
  switchTarget.mockReset(); switchTarget.mockImplementation(async (target: SystemAudioTarget) => {
    useStore.setState(state => ({ settings: mergeSettingsSnapshot(state.settings, { systemAudioTarget: target }) }));
  });
  useStore.setState({ ...initial, initializationStatus: "ready", saveSettings: save, switchSystemAudioTarget: switchTarget,
    settings: { ...initial.settings, systemAudioTarget: { kind: "system" }, recordSessionAudio: true },
    session: { ...initial.session, status: { kind: "idle" }, isActive: false, isPaused: false },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals(); });
async function render() { await act(async () => root.render(<ApplicationAudio />)); }
function trigger() { return host.querySelector<HTMLButtonElement>(`button[aria-label="${applicationAudioCopy().title}"]`)!; }
function refresh() { return host.querySelector<HTMLButtonElement>(`button[aria-label="${applicationAudioCopy().refresh}"]`)!; }
async function open() { if (trigger().getAttribute("aria-expanded") !== "true") await act(async () => trigger().click()); }
async function choose(option: string) {
  await open();
  const node = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === option)!;
  expect(node).toBeDefined();
  await act(async () => node.click());
}

it.each(["en", "zh", "ja"] as const)("offers one picker and lists only after user intent in %s", async language => {
  setStoredUiLanguage(language); await render();
  expect(invoke).not.toHaveBeenCalled();
  expect(host.querySelectorAll('button[role="combobox"]')).toHaveLength(1);
  expect(trigger().textContent).toContain(applicationAudioCopy().all);
  await open();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("audio_applications");
  expect(switchTarget).not.toHaveBeenCalled();
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([applicationAudioCopy().all, "Player", "Chat"]);
  expect(document.querySelector('[role="option"] .mimi-select__icon')).toBeNull();
  await choose("Player");
  expect(switchTarget).toHaveBeenCalledExactlyOnceWith({ kind: "application", ...applications[0] });
  expect(save).not.toHaveBeenCalled();
  expect(useStore.getState().settings.recordSessionAudio).toBe(false);
});

it("keeps the popup open while applications load and never offers a placeholder as a selection", async () => {
  let resolve!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  await render(); await open();
  expect(trigger().disabled).toBe(false);
  expect(trigger().getAttribute("aria-expanded")).toBe("true");
  expect(host.querySelector('.application-audio-picker__feedback')).toBeNull();
  expect(refresh().getAttribute("aria-busy")).toBe("true");
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([applicationAudioCopy().all]);
  await act(async () => resolve({ supported: true, applications }));
  expect(trigger().getAttribute("aria-expanded")).toBe("true");
  await choose("Player");
  expect(trigger().textContent).toContain("Player");
});

it("searches applications and returns to all audio through the same picker", async () => {
  await render(); await open();
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
  await choose(applicationAudioCopy().all);
  expect(useStore.getState().settings.systemAudioTarget).toEqual({ kind: "system" });
});

it("shows an unavailable saved app without making it selectable or changing it silently", async () => {
  useStore.setState({ settings: { ...useStore.getState().settings, systemAudioTarget: { kind: "application", id: "gone", name: "Old player" } } });
  await render();
  expect(trigger().textContent).toContain("Old player");
  await act(async () => refresh().click());
  expect(trigger().textContent).toContain("Old player · Unavailable");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().unavailable);
  expect(switchTarget).not.toHaveBeenCalled();
  await open();
  expect([...document.querySelectorAll('[role="option"]')].some(node => node.textContent?.includes("Old player"))).toBe(false);
  await choose("Player");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("reports list failures safely and allows refresh to an empty result with All applications still available", async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error("private-window-title"));
  await render(); await open();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().failed);
  expect(host.textContent).not.toContain("private-window-title");
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [] });
  await act(async () => refresh().click());
  expect(refresh().title).toBe(applicationAudioCopy().empty);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([applicationAudioCopy().all]);
});

it("does not mark a new selection from another window missing using this window's old list", async () => {
  await render(); await open();
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    systemAudioTarget: { kind: "application", id: "example.new-player", name: "New player" },
  } })));
  expect(trigger().textContent).toContain("New player");
  expect(trigger().textContent).not.toContain(applicationAudioCopy().missing);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(invoke).toHaveBeenCalledOnce();
  await act(async () => refresh().click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().unavailable);
});

it("clears a failed switch after another window successfully changes the selected application", async () => {
  switchTarget.mockRejectedValueOnce("application_audio_unavailable");
  await render(); await choose("Player");
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    systemAudioTarget: { kind: "application", ...applications[1] },
  } })));
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(trigger().textContent).toContain("Chat");
});

it("keeps the error when the backend saves the new target but cannot reconnect it", async () => {
  switchTarget.mockImplementationOnce(async (systemAudioTarget: SystemAudioTarget) => {
    useStore.setState(state => ({ settings: { ...state.settings, systemAudioTarget } }));
    throw new Error("application_audio_unavailable");
  });
  await render(); await choose("Player");
  expect(trigger().textContent).toContain("Player");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().unavailable);
});

it("ignores an enumeration that was started for a different target", async () => {
  let resolve!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  await render(); await open();
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    systemAudioTarget: { kind: "application", id: "example.new-player", name: "New player" },
  } })));
  await act(async () => resolve({ supported: true, applications }));
  expect(trigger().textContent).toContain("New player");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("blocks repeated switches without optimistic selection and recovers from a rejected switch", async () => {
  let reject!: (reason: Error) => void;
  switchTarget.mockImplementationOnce(() => new Promise((_, fail) => { reject = fail; }));
  await render(); await choose("Player");
  expect(trigger().disabled).toBe(true);
  expect(trigger().textContent).toContain(applicationAudioCopy().all);
  await act(async () => trigger().click());
  expect(switchTarget).toHaveBeenCalledOnce();
  await act(async () => reject(new Error("private-error")));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().saveFailed);
  expect(useStore.getState().settings.systemAudioTarget.kind).toBe("system");
  await choose("Chat");
  expect(useStore.getState().settings.systemAudioTarget).toMatchObject({ kind: "application", name: "Chat" });
});

it.each([false, true])("changes the application during a session and preserves paused=%s", async isPaused => {
  const session = { ...initial.session, status: { kind: "listening" } as const, isActive: !isPaused, isPaused };
  useStore.setState({ session });
  await render(); await choose("Player");
  expect(switchTarget).toHaveBeenCalledExactlyOnceWith({ kind: "application", ...applications[0] });
  expect(useStore.getState().session).toBe(session);
  expect(save).not.toHaveBeenCalled();
});

it.each([
  { status: { kind: "connecting" }, isActive: false, isPaused: false },
  { status: { kind: "stopping" }, isActive: false, isPaused: false },
] satisfies Pick<SessionStateEvent, "status" | "isActive" | "isPaused">[])("locks the target during transition: %j", async state => {
  useStore.setState({ session: { ...initial.session, ...state } });
  await render();
  expect(trigger().disabled).toBe(true);
  expect(refresh().disabled).toBe(true);
  expect(switchTarget).not.toHaveBeenCalled();
});

it("leaves unsupported platforms on system audio without invoking capture APIs", async () => {
  vi.stubGlobal("navigator", { userAgent: "Linux" }); await render();
  expect(trigger().disabled).toBe(true);
  expect(host.querySelector('.application-audio-picker')?.getAttribute("title")).toContain(applicationAudioCopy().unsupported);
  expect(invoke).not.toHaveBeenCalled();
});
