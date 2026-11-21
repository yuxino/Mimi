// @vitest-environment jsdom
import { act, Profiler } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, providerDisplayName, setStoredUiLanguage } from "../../../lib/i18n";
import { useStore } from "../../../lib/store";
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
  expect(host.querySelector(".overlay-service__button")).toBeNull();
  expect(host.textContent).toContain("API80 ms");
  expect(host.textContent).not.toContain("700 ms");
});

it.each([["zh", "Alibaba Cloud"], ["en", "Daily subtitles"]] as const)(
  "omits the single built-in brand in %s and keeps timing and recovery", async (language, name) => {
    setStoredUiLanguage(language);
    useStore.setState(state => ({ settings: { ...state.settings, profiles: [{
      ...state.settings.profiles[0], name, textTranslation: "followService",
    }] } }));
    await act(async () => root.render(<OverlaySessionDetails onOpenProfile={() => {}} disabled={false} />));
    expect(host.querySelector(".overlay-service__button")).toBeNull();
    expect([...host.querySelectorAll(".overlay-latency strong")].map(node => node.textContent)).toEqual(["120 ms", "700 ms"]);

    await act(async () => useStore.setState(state => ({ session: { ...state.session,
      translationRecovery: { reason: "rateLimited", retryAfterMs: 600, retryScheduled: true },
    } })));
    expect(host.querySelector('[role="status"]')?.textContent).toBe(I18N.overlay.translationRateLimited);
    expect(host.querySelector(".overlay-service__button")).toBeNull();
  },
);

it.each(["Private recognizer", undefined])("keeps a standalone custom recognition service named %s", async speechRecognitionName => {
  useStore.setState(state => ({ settings: { ...state.settings, targetLanguage: "original", profiles: [{
    ...state.settings.profiles[0], provider: "customOpenAIASR", speechRecognitionName,
  }] } }));
  await act(async () => root.render(<OverlaySessionDetails onOpenProfile={() => {}} disabled={false} />));
  const stage = host.querySelector('[data-stage="recognition"]');
  expect(stage?.querySelector('[data-provider="customOpenAIASR"]')).not.toBeNull();
  expect(stage?.querySelector(".overlay-service__name")?.textContent).toBe(speechRecognitionName ?? providerDisplayName("customOpenAIASR"));
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
  expect(render).not.toHaveBeenCalled();
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect([...host.querySelectorAll(".overlay-latency__measurement strong")].map(node => node.textContent)).toEqual(["120 ms", "700 ms"]);
});

it("leaves streamed translation activity to the panel header", async () => {
  useStore.setState(state => ({
    settings: { ...state.settings, sourceLanguage: "auto", profiles: [{
      id: "sample", name: "Realtime", provider: "openAIRealtime", textTranslation: "followService", credentialState: "present",
    }] },
    session: { ...state.session, apiLatencyMs: null, translationLatencyMs: null,
      translationLatencyKind: null, detectedLanguage: "en", isTranslationPending: false,
      isTranslationPreviewPending: false, subtitles: {
        source: { text: "Hello world", isFinal: false }, translation: { text: "", isFinal: false }, history: [],
      },
    },
  }));
  const render = vi.fn();
  await act(async () => root.render(<Profiler id="stream-details" onRender={render}>
    <OverlaySessionDetails onOpenProfile={() => {}} disabled={false} />
  </Profiler>));
  expect(host.querySelector('[role="status"]')).toBeNull();
  const update = (text: string) => useStore.setState(state => ({ session: { ...state.session,
    subtitles: { ...state.session.subtitles, realtimePreview: {
      source: state.session.subtitles.source, translation: { text, isFinal: false },
    } },
  } }));
  await act(async () => update("你好"));
  expect(host.querySelector('[role="status"]')).toBeNull();
  expect(host.textContent).not.toContain("ms");
  render.mockClear();
  await act(async () => update("你好世界"));
  expect(render).not.toHaveBeenCalled();
  await act(async () => useStore.setState(state => ({ session: { ...state.session,
    subtitles: { ...state.session.subtitles, realtimePreview: null,
      source: { text: "Hello world", isFinal: true }, translation: { text: "你好世界", isFinal: true },
    },
  } })));
  expect(host.querySelector('[role="status"]')).toBeNull();
});
