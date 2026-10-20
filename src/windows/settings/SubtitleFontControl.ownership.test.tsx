// @vitest-environment jsdom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { installedFontFamilies, listenSessionState, listenSettingsChanged, sessionGetState, settingsGet, settingsSave } from "../../lib/ipc";
import { disposeStoreSnapshotStreams, useStore } from "../../lib/store";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import type { SettingsDraft, SettingsSnapshot } from "../../lib/types";
import { SubtitleFontControl } from "./SubtitleFontControl";
import { SettingsToastRegion } from "./SettingsToast";
import { subtitleFontChoiceOwner } from "./subtitleFontChoiceOwner";

vi.mock("../../lib/ipc", async original => ({
  ...await original<typeof import("../../lib/ipc")>(),
  installedFontFamilies: vi.fn(), settingsSave: vi.fn(), isTauri: true,
  listenSessionState: vi.fn(), listenSettingsChanged: vi.fn(), sessionGetState: vi.fn(), settingsGet: vi.fn(),
}));

const initial = useStore.getState();
let host: HTMLDivElement;
let root: Root;
let nativeSettings: SettingsSnapshot;
let writes: string[];
let emitSettings: (settings: SettingsSnapshot) => void;
const preview = vi.fn();

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

function Harness({ name = "font" }: { name?: string }) {
  const value = useStore(state => state.settings.subtitleFontFamily ?? "");
  const save = useStore(state => state.saveSettings);
  return <div data-view={name}><SubtitleFontControl value={value} onPreview={preview}
    getCurrentValue={() => useStore.getState().settings.subtitleFontFamily ?? ""}
    onChange={family => save({ subtitleFontFamily: family })} /></div>;
}
async function render(key = "first", strict = false, multiple = false) {
  const contents = <><Harness key={key} />{multiple && <Harness name="sibling" />}<SettingsToastRegion scopeKey={key} /></>;
  await act(() => root.render(strict ? <StrictMode>{contents}</StrictMode> : contents));
}
function trigger(name = "font") { return host.querySelector<HTMLButtonElement>(`[data-view="${name}"] button[role="combobox"]`)!; }
async function choose(family: string, name = "font") {
  await act(async () => trigger(name).click());
  await act(async () => [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(option => option.textContent === family)!.click());
}
async function arrow(key: "ArrowDown" | "ArrowUp" = "ArrowDown") {
  await act(() => trigger().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
}
function persist(draft: SettingsDraft) {
  nativeSettings = { ...nativeSettings, ...draft };
  emitSettings(nativeSettings);
  return nativeSettings;
}
function holdFirst(fails = false) {
  const first = deferred();
  vi.mocked(settingsSave).mockImplementationOnce(async draft => {
    writes.push(draft.subtitleFontFamily!);
    await first.promise;
    if (fails) throw new Error("synthetic-private-error");
    return persist(draft);
  });
  return first;
}

beforeEach(async () => {
  disposeStoreSnapshotStreams();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  nativeSettings = { ...initial.settings, subtitleFontFamily: "" };
  useStore.setState({ ...initial, settings: nativeSettings }, true);
  writes = [];
  preview.mockReset();
  vi.mocked(installedFontFamilies).mockReset().mockResolvedValue(["A", "B", "C"]);
  vi.mocked(settingsSave).mockReset().mockImplementation(async draft => { writes.push(draft.subtitleFontFamily!); return persist(draft); });
  vi.mocked(listenSettingsChanged).mockReset().mockImplementation(async handler => { emitSettings = handler; return vi.fn(); });
  vi.mocked(listenSessionState).mockReset().mockResolvedValue(vi.fn());
  vi.mocked(settingsGet).mockReset().mockResolvedValue(nativeSettings);
  vi.mocked(sessionGetState).mockReset().mockResolvedValue(initial.session);
  await useStore.getState().init();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  expect(subtitleFontChoiceOwner.getSnapshot()).toBeNull();
  await act(() => root.unmount()); host.remove();
  disposeStoreSnapshotStreams();
  useStore.setState(initial, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});

it.each([false, true])("a remounted C replaces old queued B through the real store/coordinator (first save fails=%s)", async fails => {
  const first = holdFirst(fails);
  await render(); await choose("A"); await arrow();
  expect(trigger().textContent).toBe("B");
  await act(() => root.render(<SettingsToastRegion scopeKey="other-page" />));
  await render("second");
  const remountedChoice = trigger().textContent;
  const remountedPreview = preview.mock.lastCall?.[0];
  await choose("C");
  await act(async () => first.resolve());
  expect(writes).toEqual(["A", "C"]);
  expect(remountedChoice).toBe("B");
  expect(remountedPreview).toBe("B");
  expect(nativeSettings.subtitleFontFamily).toBe("C");
  expect(useStore.getState().settings.subtitleFontFamily).toBe("C");
  expect(trigger().textContent).toBe("C");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("finishes B after leaving the page without a replacement choice", async () => {
  const first = holdFirst();
  await render(); await choose("A"); await arrow();
  await act(() => root.render(<SettingsToastRegion scopeKey="other-page" />));
  const calls = preview.mock.calls.length;
  await act(async () => first.resolve());
  expect(writes).toEqual(["A", "B"]);
  expect(nativeSettings.subtitleFontFamily).toBe("B");
  expect(preview).toHaveBeenCalledTimes(calls);
  await render("second");
  expect(trigger().textContent).toBe("B");
});

it("shares the latest choice and busy state between simultaneous instances", async () => {
  const first = holdFirst();
  await render("first", false, true); await choose("A"); await arrow();
  expect(trigger("sibling").textContent).toBe("B");
  expect(host.querySelectorAll('.settings-select-wrap[aria-busy="true"]')).toHaveLength(2);
  await choose("C", "sibling");
  expect(trigger().textContent).toBe("C");
  await act(async () => first.resolve());
  expect(writes).toEqual(["A", "C"]);
  expect(trigger().textContent).toBe("C");
  expect(trigger("sibling").textContent).toBe("C");
});

it("keeps a remounted ABA choice satisfied by the successful first save", async () => {
  const first = holdFirst();
  await render(); await choose("A"); await arrow();
  await act(() => root.render(<div>Other page</div>));
  await render("second"); await choose("A");
  await act(async () => first.resolve());
  expect(writes).toEqual(["A"]);
  expect(nativeSettings.subtitleFontFamily).toBe("A");
  expect(trigger().textContent).toBe("A");
});

it.each([false, true])("persists a new ABA choice after an external event supersedes the old response (leave page=%s)", async leave => {
  const first = deferred();
  vi.mocked(settingsSave).mockImplementationOnce(async draft => {
    writes.push(draft.subtitleFontFamily!);
    // Native committed A and emitted its event, but the command response is
    // delayed. A later external D must invalidate that old response in the store.
    const response = persist(draft);
    await first.promise;
    return response;
  });
  await render(); await choose("A"); await arrow();
  await act(() => { nativeSettings = { ...nativeSettings, subtitleFontFamily: "D" }; emitSettings(nativeSettings); });
  await choose("A");
  if (leave) await act(() => root.render(<div>Other page</div>));
  await act(async () => first.resolve());
  expect(writes).toEqual(["A", "A"]);
  expect(nativeSettings.subtitleFontFamily).toBe("A");
  expect(useStore.getState().settings.subtitleFontFamily).toBe("A");
  if (leave) await render("second");
  expect(trigger().textContent).toBe("A");
});

it("keeps pending previews through older/native external values and accepts idle external changes", async () => {
  const first = holdFirst();
  await render(); await choose("A"); await arrow();
  for (const family of ["", "A", "External font"]) {
    await act(() => emitSettings({ ...nativeSettings, subtitleFontFamily: family }));
    expect(trigger().textContent).toBe("B");
    expect(preview).toHaveBeenLastCalledWith("B");
  }
  await act(async () => first.resolve());
  expect(writes).toEqual(["A", "B"]);
  expect(trigger().textContent).toBe("B");
  await act(() => emitSettings({ ...nativeSettings, subtitleFontFamily: "External font" }));
  expect(trigger().textContent).toBe("External font");
  expect(preview).toHaveBeenLastCalledWith(undefined);
});

it("reports the current mounted choice's failure and rolls back to the preceding saved font", async () => {
  const first = holdFirst();
  await render(); await choose("A"); await arrow();
  await act(() => root.render(<div>Other page</div>));
  await render("second"); await choose("C");
  vi.mocked(settingsSave).mockRejectedValueOnce(new Error("synthetic-private-error"));
  await act(async () => first.resolve());
  expect(nativeSettings.subtitleFontFamily).toBe("A");
  expect(trigger().textContent).toBe("A");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(I18N.settings.settingSaveFailed(I18N.settings.subtitleFont));
  expect(document.body.textContent).not.toContain("synthetic-private-error");
});

it("restores the pending preview after StrictMode rehearses a remount", async () => {
  const first = holdFirst();
  await render(); await choose("A"); await arrow();
  await act(() => root.render(<div>Other page</div>));
  await render("second", true);
  expect(trigger().textContent).toBe("B");
  expect(preview).toHaveBeenLastCalledWith("B");
  await act(async () => first.resolve());
  expect(writes).toEqual(["A", "B"]);
});
