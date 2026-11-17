// @vitest-environment jsdom
import { act, Profiler } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { OverlaySessionDetails } from "./OverlaySessionDetails";

let host: HTMLDivElement, root: Root;
const initialState = useStore.getState();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  setStoredUiLanguage("en");
  useStore.setState({
    settings: { ...initialState.settings, activeProfileId: "sample", targetLanguage: "zh", profiles: [{
      id: "sample", name: "Daily subtitles", provider: "alibabaCloud", textTranslation: "deepL", credentialState: "present",
    }] },
    session: { ...initialState.session, status: { kind: "listening" }, isActive: true, isPaused: false,
      apiLatencyMs: 120, translationLatencyMs: 700, translationLatencyKind: "request" },
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  useStore.setState(initialState, true); setStoredUiLanguage("system"); vi.unstubAllGlobals();
});

it("keeps actual service roles and changing measurements in the panel with an explicit profile action", async () => {
  const openProfile = vi.fn();
  await act(async () => root.render(<OverlaySessionDetails onOpenProfile={openProfile} disabled={false} />));
  expect([...host.querySelectorAll("[data-provider]")].map(node => node.getAttribute("data-provider"))).toEqual(["alibabaCloud", "deepL"]);
  const profile = host.querySelector("button")!;
  expect(profile.getAttribute("aria-label")).toContain("Current configuration: Daily subtitles");
  expect(host.textContent).toContain("API120 ms");
  await act(async () => useStore.setState(state => ({ session: { ...state.session, apiLatencyMs: 80 } })));
  expect(host.textContent).toContain("API80 ms");
  await act(async () => profile.click()); expect(openProfile).toHaveBeenCalledOnce();
  await act(async () => root.render(<OverlaySessionDetails onOpenProfile={openProfile} disabled />));
  await act(async () => profile.click()); expect(openProfile).toHaveBeenCalledOnce();
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings, targetLanguage: "original" } })));
  expect(host.querySelectorAll("[data-provider]")).toHaveLength(1);
  expect(host.textContent).not.toContain("700 ms");
});

it("does not subscribe to caption text or unchanged status snapshots", async () => {
  const render = vi.fn();
  await act(async () => root.render(<Profiler id="service-details" onRender={render}>
    <OverlaySessionDetails onOpenProfile={() => {}} disabled={false} />
  </Profiler>));
  render.mockClear();
  await act(async () => {
    for (let i = 0; i < 50; i++) useStore.setState(state => ({ session: {
      ...state.session, status: { kind: "listening" },
      subtitles: { ...state.session.subtitles, source: { text: `Fixture ${i}`, isFinal: false } },
    } }));
  });
  expect(render).not.toHaveBeenCalled();
  await act(async () => useStore.setState(state => ({ session: { ...state.session, isTranslationPending: true } })));
  expect(host.querySelector('[role="status"]')?.textContent).toBe("Translating");
});
