import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { SUBTITLE_DISPLAY_OPTIONS } from "/src/lib/subtitles/subtitleDisplay.ts";
import { formatGeminiTranscriptForDisplay } from "/src/windows/overlay/timeline/geminiTranscriptDisplay.ts";

// These are published snapshot shapes, not simulated provider implementations.
// No endpoint or credential values are installed, and the actual browser store
// owns profile selection and display-mode changes through its real panel.
export function createCaptionModeFixture(store, ControlWindow) {
  const routes = [
    { id: "caption-doubao", name: "Doubao realtime", provider: "volcanoEngine", textTranslation: "followService", atomic: false },
    { id: "caption-alibaba", name: "Alibaba ASR + Qwen-MT Lite", provider: "alibabaCloud", alibabaRealtimeMode: "disabled", qwenMtModel: "lite", textTranslation: "followService", atomic: true },
    { id: "caption-gemini", name: "Gemini simultaneous", provider: "googleGeminiLive", speechTranslationMode: "simultaneous", textTranslation: "followService", atomic: false },
    { id: "caption-independent", name: "Independent DashScope ASR + DeepL", provider: "customDashScopeASR", textTranslation: "deepL", customSpeechSourceLanguages: ["en"], atomic: true },
  ];
  const saved = store.getState();
  const host = document.createElement("div");
  host.id = "caption-mode-controls";
  Object.assign(host.style, { position: "fixed", top: "8px", right: "8px", width: "280px", zIndex: "10000" });
  document.body.append(host);
  const root = createRoot(host);
  store.setState({ initializationStatus: "ready", settings: { ...saved.settings,
    showIntermediateSubtitles: true, languageCapabilities: undefined,
    profiles: routes.map(route => {
      const profile = { ...route, credentialState: "present", speechCredentialState: "present", textCredentialState: "present" };
      delete profile.atomic;
      return profile;
    }),
    activeProfileId: routes.at(-1).id,
  } });
  root.render(createElement(ControlWindow));
  let route, history, current, index;
  let frames = 0;
  const timeline = () => document.querySelector(".overlay-surface .overlay-timeline");
  const leaf = text => [...timeline().querySelectorAll("span")].find(node => !node.childElementCount && node.textContent === text);
  const shown = text => route.provider === "googleGeminiLive" ? formatGeminiTranscriptForDisplay(text) : text;
  const tailGap = () => timeline().scrollHeight - timeline().clientHeight - timeline().scrollTop;
  const liveButton = () => document.querySelector(".overlay-surface .overlay-return-to-live");
  function visible(node, fully = false) {
    const clip = timeline().getBoundingClientRect(), rect = node?.getBoundingClientRect();
    return rect?.width > 0 && (fully ? rect.top >= clip.top - 1 && rect.bottom <= clip.bottom + 1
      : rect.bottom > clip.top && rect.top < clip.bottom);
  }
  async function settle(stage, ready, sample) {
    const start = performance.now();
    let steady;
    while (true) {
      const now = await new Promise(requestAnimationFrame);
      frames++;
      sample?.();
      if (ready()) { steady ??= now; if (now - steady >= 200) return; } else steady = undefined;
      if (now - start > 3000) throw new Error(`Caption modes did not settle: ${JSON.stringify({ stage,
        route: route?.id, mode: store.getState().settings.subtitleDisplayMode, tailGap: tailGap(),
        reading: Boolean(liveButton()), scrollTop: timeline().scrollTop,
        scrollRange: timeline().scrollHeight - timeline().clientHeight,
        hovered: document.querySelector(".overlay-surface").matches(":hover") })}`);
    }
  }
  function pair(number) {
    return { audioSource: "system", source: `Original ${number} follows the riverside path.`,
      translation: `译文 ${number} 沿着河边的小路。`, createdAt: 1_700_010_000_000 + number * 1000 };
  }
  function advance() {
    index++;
    const base = pair(index);
    current = route.provider === "googleGeminiLive" ? { ...base,
      source: base.source + " A second sentence reaches the bridge.",
      translation: base.translation + "第二句话经过老车站，走到了桥边。" } : base;
  }
  function publish(pending = false) {
    const utteranceId = `caption-${index}`;
    // Gemini's continuous transcript events have no utterance ID. HTTP MT
    // retains the preceding confirmed raw translation beside a complete pair.
    const owner = route.provider === "googleGeminiLive" ? {} : { utteranceId };
    const content = { source: { text: current.source, ...owner, isFinal: false },
      translation: route.atomic ? { text: history.at(-1).translation, isFinal: true }
        : { text: current.translation, ...owner, isFinal: false }, history,
      previewPair: route.atomic ? { source: current.source, translation: current.translation, utteranceId } : null };
    const track = { ...content, audioSource: "system", detectedLanguage: "en",
      isTranslationPending: pending, isTranslationTimedOut: false };
    store.setState(state => ({ session: { ...state.session, atomicPreview: route.atomic,
      isActive: true, isPaused: false, status: { kind: "listening" }, detectedLanguage: "en",
      isTranslationPending: pending, isTranslationPreviewPending: false, isTranslationTimedOut: false,
      subtitles: { ...content, tracks: [track] } } }));
  }
  function lanes(mode, fully = false) {
    const source = leaf(shown(current.source)), translation = leaf(shown(current.translation));
    return (mode === "translation" ? !source : visible(source, fully))
      && (mode === "original" ? !translation : visible(translation, fully))
      && history.every(pair => (mode === "translation" ? !leaf(pair.source) : Boolean(leaf(pair.source)))
        && (mode === "original" ? !leaf(pair.translation) : Boolean(leaf(pair.translation))));
  }
  function anchor() {
    const node = [...timeline().querySelectorAll("[data-utterance-id] span")].find(node =>
      !node.childElementCount && node.closest("[data-utterance-id]").dataset.utteranceId.includes("history-") && visible(node, true));
    if (!node) throw new Error("No fully visible confirmed history anchor");
    return { text: node.textContent, id: node.closest("[data-utterance-id]").dataset.utteranceId,
      y: node.getBoundingClientRect().top, top: timeline().getBoundingClientRect().top,
      bottom: timeline().getBoundingClientRect().bottom };
  }
  async function unchanged(stage, before, action) {
    let shift = 0;
    action();
    await settle(stage, () => true, () => {
      const node = leaf(before.text), clip = timeline().getBoundingClientRect();
      if (!node || node.closest("[data-utterance-id]").dataset.utteranceId !== before.id) throw new Error(`${stage}: history identity changed`);
      shift = Math.max(shift, Math.abs(node.getBoundingClientRect().top - before.y),
        Math.abs(clip.top - before.top), Math.abs(clip.bottom - before.bottom));
      if (shift > 2) throw new Error(`${stage}: unchanged captions moved by ${shift}px`);
    });
    return shift;
  }
  return {
    routes: routes.map(({ id, name }) => ({ id, name })),
    modes: SUBTITLE_DISPLAY_OPTIONS.map(({ value, shortLabel }) => ({ value, label: shortLabel })),
    async ready() { await settle("control mount", () => Boolean(host.querySelector('button[aria-haspopup="dialog"]'))); },
    async begin(id) {
      route = routes.find(candidate => candidate.id === id);
      if (store.getState().settings.activeProfileId !== id || store.getState().settings.targetLanguage !== "zh") throw new Error("Actual profile selection did not preserve the translation target");
      history = Array.from({ length: 8 }, (_, i) => pair(i + 1));
      index = 8;
      advance();
      publish();
      await settle("profile snapshot", () => Boolean(leaf(history.at(-1).translation)));
    },
    async mode(value) {
      const option = SUBTITLE_DISPLAY_OPTIONS.find(option => option.value === value);
      await settle("actual mode selection", () => store.getState().settings.subtitleDisplayMode === value
        && [...host.querySelectorAll('.overlay-control-display button')].some(button => button.textContent === option.shortLabel && button.getAttribute("aria-pressed") === "true") && lanes(value));
    },
    async read(value) {
      await settle("history reader", () => Boolean(liveButton()) && tailGap() > 2);
      const before = anchor();
      const pendingShift = Math.max(await unchanged("reading pending", before, () => publish(true)),
        await unchanged("reading cleared", before, () => publish(false)));
      const historyShift = await unchanged("next pair while reading", before, () => {
        history = [...history, current]; advance(); publish();
      });
      if (!liveButton()) throw new Error("New captions silently left history reading");
      return { mode: value, pendingShift, historyShift, anchor: before.id };
    },
    async follow(value) {
      await settle("return to live", () => !liveButton() && lanes(value, true) && Math.abs(tailGap()) <= 2);
      const before = anchor();
      const pendingShift = Math.max(await unchanged("live pending", before, () => publish(true)),
        await unchanged("live cleared", before, () => publish(false)));
      history = [...history, current]; advance(); publish();
      await settle("next visible pair", () => !liveButton() && lanes(value, true) && Math.abs(tailGap()) <= 2);
      return { pendingShift, frames, historyCount: history.length,
        provider: route.provider, atomic: route.atomic, mode: value, sourceLanguage: store.getState().settings.sourceLanguage,
        targetLanguage: store.getState().settings.targetLanguage };
    },
    dispose() { root.unmount(); host.remove(); store.setState({ settings: saved.settings, session: saved.session, initializationStatus: saved.initializationStatus }); },
  };
}
