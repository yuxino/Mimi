// @vitest-environment jsdom
import { act, Profiler } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { shareUnchangedSubtitleHistory } from "../../lib/sessionSnapshot";
import { useStore } from "../../lib/store";
import { TrayPanel } from "../tray-panel/TrayPanel";
import { OverlayControlWindow } from "../overlay-control/OverlayControlWindow";
import { SettingsView } from "./SettingsView";

it("keeps non-subtitle windows and an unsaved credential draft stable across high-frequency session snapshots", async () => {
  const original = useStore.getState();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  Element.prototype.scrollTo = vi.fn();
  setStoredUiLanguage("en");
  window.history.replaceState(null, "", "#service-profiles");
  useStore.setState({ ...original, session: {
    ...original.session, status: { kind: "idle" }, isActive: false, detectedLanguage: "en", isTranslationPending: true,
    subtitles: { source: { text: "Synthetic source", isFinal: false }, translation: { text: "Synthetic translation", isFinal: false }, history: [] },
  } }, true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const commits = { settings: 0, tray: 0, control: 0 };
  try {
    await act(async () => root.render(<>
      <Profiler id="settings" onRender={() => commits.settings++}><SettingsView /></Profiler>
      <Profiler id="tray" onRender={() => commits.tray++}><TrayPanel /></Profiler>
      <Profiler id="control" onRender={() => commits.control++}><OverlayControlWindow /></Profiler>
    </>));
    await act(async () => host.querySelector<HTMLButtonElement>(".service-row__edit")!.click());
    const replace = [...host.querySelectorAll<HTMLButtonElement>(".credential-panel button")].find((button) => button.textContent === I18N.settings.replaceCredentials)!;
    await act(async () => replace.click());
    const input = host.querySelector<HTMLInputElement>('.credential-panel input[type="password"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "synthetic-unsaved-key");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => useStore.setState({ session: { ...useStore.getState().session, status: { kind: "listening" }, isActive: true } }));
    // Locking credentials intentionally discards the field's private reveal
    // state. Subsequent subtitle snapshots must keep that locked field stable.
    const lockedInput = host.querySelector<HTMLInputElement>('.credential-panel input[type="password"]')!;
    expect(lockedInput.value).toBe("synthetic-unsaved-key");
    expect(lockedInput.disabled).toBe(true);
    const before = { ...commits };
    for (let index = 0; index < 50; index++) {
      const incoming = JSON.parse(JSON.stringify(useStore.getState().session)) as typeof original.session;
      incoming.subtitles.source.text = `Synthetic source draft ${index}`;
      incoming.subtitles.translation.text = `Synthetic translation draft ${index}`;
      incoming.apiLatencyMs = index;
      incoming.translationLatencyMs = index + 10;
      await act(async () => useStore.setState((state) => ({ session: shareUnchangedSubtitleHistory(state.session, incoming) })));
    }
    expect(commits).toEqual(before);
    expect(host.querySelector('.credential-panel input[type="password"]')).toBe(lockedInput);
    expect(lockedInput.value).toBe("synthetic-unsaved-key");
    expect(lockedInput.disabled).toBe(true);
  } finally {
    await act(async () => root.unmount()); host.remove(); useStore.setState(original, true);
    window.history.replaceState(null, "", window.location.pathname); setStoredUiLanguage("system"); vi.unstubAllGlobals();
  }
});
