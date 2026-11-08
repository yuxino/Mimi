// @vitest-environment jsdom
import { act } from "react";
import { SettingsToastRegion } from "./SettingsToast";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ApplicationAudio } from "./ApplicationAudio";
import { useStore } from "../../lib/store";
import { mergeSettingsSnapshot } from "../../lib/settingsState";
import { applicationIconCache } from "../../lib/applicationAudioIcons";
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
describe.each(["Macintosh", "Linux"] as const)("shared settings picker: %s", platform => {
function text() { return applicationAudioCopy(); }
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("navigator", { userAgent: platform });
  Element.prototype.scrollIntoView = vi.fn();
  setStoredUiLanguage("en");
  applicationIconCache.clear();
  vi.mocked(invoke).mockReset(); vi.mocked(invoke).mockImplementation(async (_command, args) => ({
    supported: true,
    applications: args && "cachedOnly" in args && args.cachedOnly
      ? [useStore.getState().settings.systemAudioTarget].filter(target => target.kind === "application")
      : applications,
  }));
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
async function render() { await act(async () => root.render(<><ApplicationAudio /><SettingsToastRegion /></>)); }
function trigger() { return host.querySelector<HTMLButtonElement>(`button[aria-label="${text().title}"]`)!; }
function refresh() { return host.querySelector<HTMLButtonElement>(`button[aria-label="${text().refresh}"]`)!; }
async function open() { if (trigger().getAttribute("aria-expanded") !== "true") await act(async () => trigger().click()); }
async function choose(option: string) {
  await open();
  const node = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(node => node.textContent === option)!;
  expect(node).toBeDefined();
  await act(async () => node.click());
}

it.each(["en", "zh", "zh-TW", "ja", "de", "fr", "ko", "th", "ru"] as const)("offers one picker and lists only after user intent in %s", async language => {
  setStoredUiLanguage(language); await render();
  expect(invoke).not.toHaveBeenCalled();
  expect(host.querySelectorAll('button[role="combobox"]')).toHaveLength(1);
  expect(trigger().textContent).toContain(text().all);
  await open();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("audio_applications");
  expect(switchTarget).not.toHaveBeenCalled();
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([text().all, "Player", "Chat"]);
  expect(document.querySelectorAll('[role="option"] .mimi-select__icon[aria-hidden="true"] svg')).toHaveLength(3);
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
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([text().all]);
  const input = document.querySelector<HTMLInputElement>('.mimi-select__search')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "play");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => resolve({ supported: true, applications }));
  expect(trigger().getAttribute("aria-expanded")).toBe("true");
  expect(document.activeElement).toBe(input);
  expect(input.value).toBe("play");
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual(["Player"]);
  await choose("Player");
  expect(trigger().textContent).toContain("Player");
});

it("shows local application icons in the list and selection without saving icons in the audio target", async () => {
  const iconDataUrl = "data:image/png;base64,c3ludGhldGljLWFwcC1pY29u";
  vi.mocked(invoke).mockResolvedValue({ supported: true, applications: [
    { ...applications[0], iconDataUrl }, { ...applications[1], iconDataUrl: null },
  ] });
  await render(); await open();
  const rows = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(rows[0].querySelector('.mimi-select__icon svg')).not.toBeNull();
  expect(rows[1].querySelector('.mimi-select__icon[aria-hidden="true"] img')?.getAttribute("src")).toBe(iconDataUrl);
  expect(rows[1].querySelector('img')?.getAttribute("alt")).toBe("");
  expect(rows[2].querySelector('.mimi-select__icon svg')).not.toBeNull();
  await choose("Player");
  expect(trigger().querySelector('img')?.getAttribute("src")).toBe(iconDataUrl);
  expect(switchTarget).toHaveBeenCalledExactlyOnceWith({ kind: "application", ...applications[0] });
  expect(useStore.getState().settings.systemAudioTarget).toEqual({ kind: "application", ...applications[0] });
});

it("uses neutral fallback icons for unreadable images and never loads remote application images", async () => {
  vi.mocked(invoke).mockResolvedValue({ supported: true, applications: [
    { ...applications[0], iconDataUrl: "data:image/png;base64,invalid" },
    { ...applications[1], iconDataUrl: "https://example.invalid/app-icon.png" },
  ] });
  await render(); await open();
  const rows = [...document.querySelectorAll<HTMLElement>('[role="option"]')];
  expect(rows[2].querySelector('img')).toBeNull();
  expect(rows[2].querySelector('.mimi-select__icon svg')).not.toBeNull();
  await act(async () => rows[1].querySelector('img')!.dispatchEvent(new Event("error")));
  expect(rows[1].querySelector('img')).toBeNull();
  expect(rows[1].querySelector('.mimi-select__icon svg')).not.toBeNull();
  await choose("Player");
  await act(async () => trigger().querySelector('img')!.dispatchEvent(new Event("error")));
  expect(trigger().querySelector('.mimi-select__icon svg')).not.toBeNull();
  expect(trigger().textContent).toBe("Player");
});

it("searches applications and returns to all audio through the same picker", async () => {
  await render(); await open();
  const input = document.querySelector<HTMLInputElement>('.mimi-select__search')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "does-not-match");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(document.body.textContent).toContain(text().noMatch);
  expect(document.body.textContent).not.toContain(text().empty);
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "chat");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual(["Chat"]);
  await act(async () => document.querySelector<HTMLElement>('[role="option"]')!.click());
  await choose(text().all);
  expect(useStore.getState().settings.systemAudioTarget).toEqual({ kind: "system" });
});

it("shows an unavailable saved app without making it selectable or changing it silently", async () => {
  useStore.setState({ settings: { ...useStore.getState().settings, systemAudioTarget: { kind: "application", id: "gone", name: "Old player" } } });
  await render();
  expect(trigger().textContent).toContain("Old player");
  expect(trigger().querySelector('.mimi-select__icon svg')).not.toBeNull();
  await act(async () => refresh().click());
  expect(trigger().textContent).toContain("Old player · Unavailable");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(text().unavailable);
  expect(switchTarget).not.toHaveBeenCalled();
  await open();
  expect([...document.querySelectorAll('[role="option"]')].some(node => node.textContent?.includes("Old player"))).toBe(false);
  await choose("Player");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("reports list failures safely and allows refresh to an empty result with All applications still available", async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error("private-window-title"));
  await render(); await open();
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(text().failed);
  expect(host.textContent).not.toContain("private-window-title");
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [] });
  await act(async () => refresh().click());
  expect(refresh().title).toBe(text().empty);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([text().all]);
});

it("does not mark a new selection from another window missing using this window's old list", async () => {
  await render(); await open();
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    systemAudioTarget: { kind: "application", id: "example.new-player", name: "New player" },
  } })));
  expect(trigger().textContent).toContain("New player");
  expect(trigger().textContent).not.toContain(text().missing);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(2); // Linux only reads already discovered choices.
  await act(async () => refresh().click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(text().unavailable);
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
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(text().unavailable);
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
  expect(trigger().textContent).toContain(text().all);
  await act(async () => trigger().click());
  expect(switchTarget).toHaveBeenCalledOnce();
  await act(async () => reject(new Error("private-error")));
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(text().saveFailed);
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

if (platform === "Linux") it("shows a retired saved target using only native cached choices", async () => {
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [] });
  useStore.setState({ settings: { ...initial.settings,
    systemAudioTarget: { kind: "application", id: "retired-token", name: "Player · stream" },
  } });
  await render();
  expect(trigger().textContent).toContain(text().missing);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(text().unavailable);
  expect(switchTarget).not.toHaveBeenCalled();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("audio_applications", { cachedOnly: true });
});

it("leaves unsupported platforms on system audio without invoking capture APIs", async () => {
  vi.stubGlobal("navigator", { userAgent: "unsupported-platform" });
  await render();
  expect(trigger().disabled).toBe(true);
  expect(host.querySelector('.application-audio-picker')?.getAttribute("title")).toContain(text().unsupported);
  expect(invoke).not.toHaveBeenCalled();
});

it("honors the existing native list capability without new settings fields", async () => {
  vi.mocked(invoke).mockResolvedValueOnce({ supported: false, applications: [] });
  await render(); await open();
  expect(trigger().disabled).toBe(true);
  expect(refresh().disabled).toBe(true);
  expect(switchTarget).not.toHaveBeenCalled();
});

if (platform === "Linux") it("does not let a delayed cached read replace an explicit refresh", async () => {
  let complete!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  useStore.setState({ settings: { ...initial.settings, systemAudioTarget: { kind: "application", ...applications[0] } } });
  await render(); await open();
  await act(async () => complete({ supported: true, applications: [] }));
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(invoke).toHaveBeenNthCalledWith(1, "audio_applications", { cachedOnly: true });
  expect(invoke).toHaveBeenNthCalledWith(2, "audio_applications");
});

});
