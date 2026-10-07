// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { ApplicationAudioPicker } from "./ApplicationAudioPicker";
import { applicationIconCache } from "../lib/applicationAudioIcons";
import { applicationAudioCopy, type ApplicationSnapshot } from "../lib/applicationAudio";
import { useStore } from "../lib/store";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../lib/ipc", async original => ({ ...await original<typeof import("../lib/ipc")>(), isTauri: true }));
const initial = useStore.getState();
const player = { id: "example.player", name: "Player", iconDataUrl: "data:image/png;base64,cGxheWVy" };
const chat = { id: "example.chat", name: "Chat", iconDataUrl: "data:image/png;base64,Y2hhdA==" };
const snapshot = { supported: true, applications: [player, chat] };
const switchTarget = vi.fn(), save = vi.fn();
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("navigator", { userAgent: "Macintosh" });
  applicationIconCache.clear();
  vi.mocked(invoke).mockReset(); vi.mocked(invoke).mockResolvedValue(snapshot);
  switchTarget.mockReset(); save.mockReset();
  useStore.setState({ ...initial, initializationStatus: "ready", switchSystemAudioTarget: switchTarget, saveSettings: save,
    settings: { ...initial.settings, systemAudioTarget: { kind: "application", id: player.id, name: player.name } },
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); useStore.setState(initial, true);
  applicationIconCache.clear(); vi.unstubAllGlobals();
});
async function mount(visible = true) { await act(async () => root.render(visible ? <ApplicationAudioPicker /> : null)); }
function trigger() { return host.querySelector<HTMLButtonElement>('button[role="combobox"]')!; }
function icon() { return trigger().querySelector('img')?.getAttribute("src"); }
function deferred() {
  let resolve!: (value: ApplicationSnapshot) => void;
  const promise = new Promise<ApplicationSnapshot>(done => { resolve = done; });
  return { promise, resolve };
}

it("restores the saved application's icon on first mount without creating choices, writes, or capture", async () => {
  await mount();
  expect(icon()).toBe(player.iconDataUrl);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("audio_applications");
  expect(document.querySelector('[role="option"]')).toBeNull();
  expect(trigger().textContent).toBe("Player");
  expect(switchTarget).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
});

it("keeps the selected icon immediately after the floating controls unmount and reopen", async () => {
  await mount(); await mount(false);
  const next = deferred(); vi.mocked(invoke).mockReturnValueOnce(next.promise);
  await mount();
  expect(icon()).toBe(player.iconDataUrl);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  // Cached icons never become a stale selectable application list.
  await act(async () => trigger().click());
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent)).toEqual([applicationAudioCopy().all]);
  expect(invoke).toHaveBeenCalledTimes(2); // The manual open shares the pending read.
  await act(async () => next.resolve(snapshot));
  expect(icon()).toBe(player.iconDataUrl);
  expect(switchTarget).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
});

it("restores an application chosen in another window without selecting it again", async () => {
  await mount();
  const next = deferred(); vi.mocked(invoke).mockReturnValueOnce(next.promise);
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    systemAudioTarget: { kind: "application", id: chat.id, name: chat.name },
  } })));
  expect(icon()).toBe(chat.iconDataUrl);
  await act(async () => next.resolve({ supported: true, applications: [chat] }));
  expect(icon()).toBe(chat.iconDataUrl);
  expect(trigger().textContent).toBe("Chat");
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(switchTarget).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
});

it("restores an old listed application's icon after another window's new target lookup fails", async () => {
  await mount();
  await act(async () => trigger().click());
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent))
    .toEqual([applicationAudioCopy().all, player.name, chat.name]);
  expect(invoke).toHaveBeenCalledTimes(2);
  vi.mocked(invoke).mockRejectedValueOnce(new Error("synthetic-private-error"));
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    systemAudioTarget: { kind: "application", id: "example.new-player", name: "New player" },
  } })));
  expect(icon()).toBeUndefined();
  expect(applicationIconCache.read().size).toBe(0);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(3); // A background failure does not retry.

  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    systemAudioTarget: { kind: "application", id: player.id, name: player.name },
  } })));
  expect(icon()).toBe(player.iconDataUrl);
  expect(trigger().textContent).toBe(player.name);
  expect(invoke).toHaveBeenCalledTimes(4);
  await mount();
  expect(invoke).toHaveBeenCalledTimes(4); // Restoring the icon does not trigger another read.
  expect(vi.mocked(invoke).mock.calls.every(([command]) => command === "audio_applications")).toBe(true);
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent))
    .toEqual([applicationAudioCopy().all, player.name, chat.name]);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(switchTarget).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
});

it("drops a stale icon silently when the saved application exits", async () => {
  await mount(); await mount(false);
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [] });
  await mount();
  expect(icon()).toBeUndefined();
  expect(trigger().querySelector('.mimi-select__icon svg')).not.toBeNull();
  expect(trigger().textContent).toBe("Player");
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(switchTarget).not.toHaveBeenCalled();
});

it("keeps background failures silent while a manual refresh still reports a safe failure", async () => {
  applicationIconCache.replace(snapshot);
  vi.mocked(invoke).mockRejectedValue(new Error("synthetic-private-error"));
  await mount();
  expect(icon()).toBeUndefined();
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>('.application-audio-picker__refresh')!.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().failed);
  expect(host.textContent).not.toContain("synthetic-private-error");
  expect(switchTarget).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
});

it("ignores an old unmounted read when a reopened panel already received newer icons", async () => {
  const old = deferred(); vi.mocked(invoke).mockReturnValueOnce(old.promise);
  await mount(); await mount(false);
  const currentIcon = "data:image/png;base64,bmV3";
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [{ ...player, iconDataUrl: currentIcon }] });
  await mount();
  expect(icon()).toBe(currentIcon);
  await act(async () => old.resolve(snapshot));
  expect(applicationIconCache.read().get(player.id)).toBe(currentIcon);
  expect(icon()).toBe(currentIcon);
});

it("restores the selected Windows application's icon without changing capture or preferences", async () => {
  vi.stubGlobal("navigator", { userAgent: "Windows NT 10.0" });
  await mount();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("audio_applications");
  expect(icon()).toBe(player.iconDataUrl);
  expect(switchTarget).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
});

it("does not automatically enumerate application icons on Linux", async () => {
  vi.stubGlobal("navigator", { userAgent: "Linux" });
  await mount();
  expect(invoke).not.toHaveBeenCalled();
  expect(icon()).toBeUndefined();
});

it("waits for initialized settings and skips automatic enumeration for all applications", async () => {
  useStore.setState({ initializationStatus: "loading" });
  await mount(); expect(invoke).not.toHaveBeenCalled();
  await act(async () => useStore.setState({ initializationStatus: "ready" }));
  expect(icon()).toBe(player.iconDataUrl);
  await mount(false); vi.mocked(invoke).mockClear();
  useStore.setState(state => ({ settings: { ...state.settings, systemAudioTarget: { kind: "system" } } }));
  await mount(); expect(invoke).not.toHaveBeenCalled();
});
