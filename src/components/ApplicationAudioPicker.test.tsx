// @vitest-environment jsdom
import { act, StrictMode } from "react";
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
function refreshButton() { return host.querySelector<HTMLButtonElement>('.application-audio-picker__refresh')!; }
async function selectExternally(app: typeof player) {
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings,
    systemAudioTarget: { kind: "application", id: app.id, name: app.name },
  } })));
}
function deferred() {
  let resolve!: (value: ApplicationSnapshot) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<ApplicationSnapshot>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
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

it("shares the same target lookup across StrictMode effects and opening the picker", async () => {
  const pending = deferred(); vi.mocked(invoke).mockReturnValueOnce(pending.promise);
  await act(async () => root.render(<StrictMode><ApplicationAudioPicker /></StrictMode>));
  expect(invoke).toHaveBeenCalledOnce();
  await act(async () => trigger().click());
  expect(invoke).toHaveBeenCalledOnce();
  await act(async () => pending.resolve(snapshot));
  expect(icon()).toBe(player.iconDataUrl);
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

it.each([
  { name: "has no icon", applications: [{ id: player.id, name: player.name }], missing: false },
  { name: "has exited", applications: [], missing: true },
])("does not enumerate again after refreshing a selected application that $name", async ({ applications, missing }) => {
  await mount();
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications });
  await act(async () => refreshButton().click());
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(icon()).toBeUndefined();
  expect(trigger().textContent).toBe(missing ? `${player.name} · ${applicationAudioCopy().missing}` : player.name);
  expect(host.querySelector('[role="alert"]')?.textContent ?? null).toBe(missing ? applicationAudioCopy().unavailable : null);
  await mount();
  expect(invoke).toHaveBeenCalledTimes(2);
  expect(switchTarget).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
});

it("allows a failed refresh to retry successfully without an extra icon lookup", async () => {
  await mount();
  vi.mocked(invoke).mockRejectedValueOnce(new Error("synthetic-private-error"));
  await act(async () => refreshButton().click());
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().failed);
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [] });
  await act(async () => refreshButton().click());
  expect(invoke).toHaveBeenCalledTimes(3);
  expect(refreshButton().disabled).toBe(false);
  expect(trigger().textContent).toBe(`${player.name} · ${applicationAudioCopy().missing}`);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(applicationAudioCopy().unavailable);
});

it("keeps the current target's choices and pending refresh when an older target's refresh completes", async () => {
  await mount();
  const old = deferred(); vi.mocked(invoke).mockReturnValueOnce(old.promise);
  await act(async () => refreshButton().click());
  const currentIcon = "data:image/png;base64,Y3VycmVudA==";
  const current = { supported: true, applications: [{ ...chat, iconDataUrl: currentIcon }] };
  vi.mocked(invoke).mockResolvedValueOnce(current);
  await selectExternally(chat);
  expect(invoke).toHaveBeenCalledTimes(3);
  expect(icon()).toBe(currentIcon);
  expect(refreshButton().disabled).toBe(false);
  vi.mocked(invoke).mockResolvedValueOnce(current);
  await act(async () => refreshButton().click());
  expect(invoke).toHaveBeenCalledTimes(4);

  const pending = deferred(); vi.mocked(invoke).mockReturnValueOnce(pending.promise);
  await act(async () => trigger().click());
  expect(invoke).toHaveBeenCalledTimes(5);
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent))
    .toEqual([applicationAudioCopy().all, chat.name]);
  await act(async () => old.resolve({ supported: true, applications: [player] }));
  expect(refreshButton().getAttribute("aria-busy")).toBe("true");
  expect(icon()).toBe(currentIcon);
  expect(applicationIconCache.read().get(chat.id)).toBe(currentIcon);
  expect([...document.querySelectorAll('[role="option"]')].map(node => node.textContent))
    .toEqual([applicationAudioCopy().all, chat.name]);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await act(async () => pending.resolve(current));
  expect(refreshButton().disabled).toBe(false);
});

it("ignores a previous target's late refresh failure after an external target change", async () => {
  await mount();
  const old = deferred(); vi.mocked(invoke).mockReturnValueOnce(old.promise);
  await act(async () => refreshButton().click());
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [chat] });
  await selectExternally(chat);
  expect(invoke).toHaveBeenCalledTimes(3);
  expect(refreshButton().disabled).toBe(false);
  await act(async () => old.reject(new Error("synthetic-private-error")));
  expect(icon()).toBe(chat.iconDataUrl);
  expect(applicationIconCache.read().get(chat.id)).toBe(chat.iconDataUrl);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(host.textContent).not.toContain("synthetic-private-error");
});

it("does not accept a refresh from the first visit after an external target changes away and back", async () => {
  await mount();
  const old = deferred(); vi.mocked(invoke).mockReturnValueOnce(old.promise);
  await act(async () => refreshButton().click());
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [chat] });
  await selectExternally(chat);
  const currentIcon = "data:image/png;base64,cmV0dXJuZWQ=";
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [{ ...player, iconDataUrl: currentIcon }] });
  await selectExternally(player);
  expect(invoke).toHaveBeenCalledTimes(4);
  expect(icon()).toBe(currentIcon);
  await act(async () => old.resolve({ supported: true, applications: [] }));
  expect(icon()).toBe(currentIcon);
  expect(trigger().textContent).toBe(player.name);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(refreshButton().disabled).toBe(false);
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

it("restores Linux icons from cached live choices without refreshing discovery or capture", async () => {
  vi.stubGlobal("navigator", { userAgent: "Linux" });
  await mount();
  expect(invoke).toHaveBeenCalledExactlyOnceWith("audio_applications", { cachedOnly: true });
  expect(icon()).toBe(player.iconDataUrl);
  expect(applicationIconCache.read().get(player.id)).toBe(player.iconDataUrl);
  expect(switchTarget).not.toHaveBeenCalled(); expect(save).not.toHaveBeenCalled();
  await mount(false); await mount();
  expect(icon()).toBe(player.iconDataUrl);
  expect(vi.mocked(invoke).mock.calls.every(([, args]) => (args as { cachedOnly?: boolean })?.cachedOnly)).toBe(true);
});

it("ignores a delayed Linux cached icon after an explicit refresh returns newer artwork", async () => {
  vi.stubGlobal("navigator", { userAgent: "Linux" });
  const cached = deferred(); vi.mocked(invoke).mockReturnValueOnce(cached.promise);
  await mount();
  const currentIcon = "data:image/png;base64,bmV3";
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [{ ...player, iconDataUrl: currentIcon }] });
  await act(async () => trigger().click());
  await act(async () => cached.resolve(snapshot));
  expect(icon()).toBe(currentIcon);
  expect(applicationIconCache.read().get(player.id)).toBe(currentIcon);
  expect(invoke).toHaveBeenCalledTimes(2);
});

it("clears a Linux icon when its live stream is no longer cached on reopen", async () => {
  vi.stubGlobal("navigator", { userAgent: "Linux" });
  await mount(); await mount(false);
  vi.mocked(invoke).mockResolvedValueOnce({ supported: true, applications: [] });
  await mount();
  expect(icon()).toBeUndefined();
  expect(applicationIconCache.read().size).toBe(0);
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  expect(switchTarget).not.toHaveBeenCalled();
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
