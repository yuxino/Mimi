// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../../lib/i18n";
import { useStore } from "../../../lib/store";
import { LanguageStatusCapsule } from "./LanguageStatusCapsule";
import { audioInputLabel } from "../../../lib/audio/audioInput";
import { recordingStatusLabel } from "../../../lib/audio/recordingStatus";
import { capsuleLabels } from "./capsuleLabels";
import { OVERLAY_ACTIVITY_PHASES } from "../../../lib/types";

it.each(["zh", "zh-TW", "en", "ja", "ko", "fr", "de", "ru", "th"] as const)("names the expanded %s controls without repeating the language choices and retains status and collapse", async language => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  setStoredUiLanguage(language);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onToggle = vi.fn();
  try {
    await act(async () => root.render(<LanguageStatusCapsule
      phase="connecting"
      status={{ source: "Japanese", separator: "→", target: "Chinese" }}
      settings={useStore.getState().settings}
      audioRecording="recording"
      isPaused={false}
      isWaitingForFinalTranslation={false}
      expanded
      onToggle={onToggle}
    />));
    const header = host.querySelector(".overlay-control-header")!;
    expect(header.querySelector(".overlay-control-header__title")?.textContent).toBe(I18N.overlay.controlPanel);
    expect(header.querySelector(".overlay-control-island__summary")).toBeNull();
    expect(header.querySelector('[data-phase="connecting"]')).not.toBeNull();
    expect(header.querySelector(".overlay-control-island__phase")).not.toBeNull();
    expect(header.querySelector('[data-recording-status="recording"]')).not.toBeNull();
    const collapse = header.querySelector<HTMLButtonElement>(".overlay-control-header__collapse")!;
    expect(collapse.getAttribute("aria-label")?.endsWith(I18N.overlay.closeControls)).toBe(true);
    expect(collapse.getAttribute("aria-label")).toContain(recordingStatusLabel("recording", language));
    await act(async () => host.querySelector<HTMLElement>(".overlay-control-header__title")!.click());
    expect(onToggle).not.toHaveBeenCalled();
    await act(async () => collapse.click());
    expect(onToggle).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount());
    host.remove();
    setStoredUiLanguage("system");
    vi.unstubAllGlobals();
  }
});

it("keeps actual recording feedback in the existing control and replaces duplicate paused copy", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  setStoredUiLanguage("zh");
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const props = {
    phase: "listening" as const, status: { source: "日语", separator: "→", target: "中文" },
    settings: { ...useStore.getState().settings, recordSessionAudio: true },
    isPaused: false, isWaitingForFinalTranslation: false, expanded: false, onToggle: () => {},
  };
  try {
    await act(async () => root.render(<LanguageStatusCapsule {...props} />));
    expect(host.querySelector("[data-recording-status]")).toBeNull();
    for (const audioRecording of ["starting", "recording", "paused", "limited", "error", "off"] as const) {
      await act(async () => root.render(<LanguageStatusCapsule {...props}
        audioRecording={audioRecording} isPaused={audioRecording === "paused"} />));
      expect(host.querySelectorAll("button")).toHaveLength(1);
      const label = recordingStatusLabel(audioRecording, "zh");
      if (!label) {
        expect(host.querySelector("[data-recording-status]")).toBeNull();
        continue;
      }
      expect(host.querySelector("button")!.getAttribute("aria-label")).toContain(label);
      expect(host.querySelector("button")!.title).toContain(label);
      expect(host.querySelector("[data-recording-status]")?.getAttribute("data-recording-status")).toBe(audioRecording);
      if (audioRecording === "paused") {
        expect(host.querySelector(".overlay-control-island__phase")).toBeNull();
        expect(host.querySelector(".recording-indicator__label")).toBeNull();
        expect(host.querySelector("[data-recording-status]")?.getAttribute("aria-label")).toBe(label);
      }
    }
  } finally {
    await act(async () => root.unmount());
    host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals();
  }
});

it.each(["zh", "zh-TW", "en", "ja", "ko", "fr", "de", "ru", "th"] as const)("keeps the %s language pair above one progress label and retains the opening cue", async language => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  setStoredUiLanguage(language);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const onToggle = vi.fn();
  const settings = { ...useStore.getState().settings, sourceLanguage: "auto" as const, targetLanguage: "zh" as const };
  const props = { settings, status: { source: "Auto", separator: "→", target: "Chinese" },
    isPaused: false, isWaitingForFinalTranslation: false, expanded: false, onToggle,
  };
  try {
    await act(async () => root.render(<LanguageStatusCapsule {...props} phase="listening" />));
    const languageRow = host.querySelector(".overlay-control-island__summary")!;
    const labels = capsuleLabels(settings, "translating", language);
    expect(languageRow.textContent).toBe(`${labels.source}→${labels.target}`);
    const pulse = host.querySelector("[data-pulse-style]");
    expect(pulse).not.toBeNull();
    expect(host.querySelector(".overlay-control-island > svg")).not.toBeNull();
    for (const phase of ["translating", "listening", "translating"] as const) {
      await act(async () => root.render(<LanguageStatusCapsule {...props} phase={phase} />));
      expect(host.querySelector(".overlay-control-island__summary")).toBe(languageRow);
      expect(languageRow.textContent).toBe(`${labels.source}→${labels.target}`);
      expect(host.querySelector("[data-pulse-style]")).toBe(pulse);
      const progress = host.querySelectorAll(".overlay-control-island__phase");
      expect(progress).toHaveLength(phase === "translating" ? 1 : 0);
      if (phase === "translating") {
        expect(progress[0].closest(".overlay-control-island__detail")).not.toBeNull();
        expect(progress[0].textContent).toBe(labels.phase);
        expect(host.querySelector("button")?.getAttribute("aria-label")).toContain(OVERLAY_ACTIVITY_PHASES.translating.accessibilityLabel);
      }
    }
    await act(async () => host.querySelector("button")!.click());
    expect(onToggle).toHaveBeenCalledOnce();
  } finally {
    await act(async () => root.unmount()); host.remove();
    setStoredUiLanguage("system"); vi.unstubAllGlobals();
  }
});

it.each([
  { label: "explicit off", reduced: false, pulseAnimation: false },
  { label: "inherited reduced motion", reduced: true, pulseAnimation: null },
  { label: "explicit on with reduced motion", reduced: true, pulseAnimation: true },
])("keeps the readable compact indicator in sync with all styles and $label", async ({ reduced, pulseAnimation }) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: reduced, addEventListener() {}, removeEventListener() {} }));
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    for (const pulseStyle of ["syllable", "ribbon", "syllable"] as const) {
      for (const expanded of [false, true]) {
        await act(async () => root.render(<LanguageStatusCapsule
          phase="listening"
          status={{ source: "Japanese", separator: "→", target: "Chinese" }}
          settings={{ ...useStore.getState().settings, pulseStyle, pulseAnimation }}
          isPaused={false}
          isWaitingForFinalTranslation={false}
          expanded={expanded}
          onToggle={() => {}}
        />));
        const indicator = host.querySelector<HTMLElement>("[data-pulse-style]")!;
        expect(indicator.dataset.pulseStyle).toBe(pulseStyle);
        expect(indicator.dataset.clock).toBe((pulseAnimation ?? !reduced) ? "running" : "paused");
        expect(indicator.style.width).toBe("24px");
      }
    }
  } finally {
    await act(async () => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  }
});

it.each(["zh", "en", "ja"] as const)("identifies selected inputs and prioritizes lifecycle feedback in %s", async language => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  setStoredUiLanguage(language);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    for (const audioInput of ["system", "microphone", "both"] as const) {
      const props = {
        phase: "listening" as const, status: { source: "Auto", separator: "→", target: "Chinese" },
        settings: { ...useStore.getState().settings, audioInput },
        isPaused: false, isWaitingForFinalTranslation: false, expanded: false, onToggle: () => {},
      };
      await act(async () => root.render(<LanguageStatusCapsule {...props} />));
      const sources = host.querySelector('[role="img"]')!;
      if (audioInput === "system") expect(sources).toBeNull();
      else {
        expect(sources.getAttribute("aria-label")).toBe(audioInputLabel(audioInput));
        expect(sources.querySelectorAll("svg")).toHaveLength(1);
      }
      expect(host.querySelector("button")!.title).toContain(audioInputLabel(audioInput));
      expect(host.querySelector("button")!.getAttribute("aria-label")).toContain(audioInputLabel(audioInput));

      await act(async () => root.render(<LanguageStatusCapsule {...props} phase="connecting" isPaused isWaitingForFinalTranslation />));
      const connecting = host.querySelector('.overlay-control-island__phase')!.textContent;
      expect(connecting).toBe({ en: "Connecting", zh: "连接中", ja: "接続中" }[language]);
      await act(async () => root.render(<LanguageStatusCapsule {...props} phase="connecting" isStopping />));
      expect(host.querySelector('.overlay-control-island__phase')!.textContent)
        .toBe({ en: "Stopping", zh: "停止中", ja: "終了中" }[language]);
      await act(async () => root.render(<LanguageStatusCapsule {...props} isPaused />));
      expect(host.querySelector('.overlay-control-island__phase')!.textContent)
        .toBe({ en: "Paused", zh: "暂停", ja: "一時停止" }[language]);
    }
  } finally {
    await act(async () => root.unmount());
    host.remove(); setStoredUiLanguage("system"); vi.unstubAllGlobals();
  }
});

it.each(["Windows NT 10.0", "Macintosh", "Linux x86_64"])("measures collapsed content and preserves platform-specific DPI behavior on %s", async platform => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("devicePixelRatio", 1);
  const userAgent = vi.spyOn(navigator, "userAgent", "get").mockReturnValue(platform);
  const windowsDpiReports = platform.startsWith("Windows") ? 1 : 0;
  const callbacks: (() => void)[] = [];
  const disconnect = vi.fn();
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { callbacks.push(callback); }
    observe() {}
    disconnect = disconnect;
  });
  let width = 148.2;
  const bounds = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockImplementation(() => ({ width } as DOMRect));
  const onWidthChange = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const props = {
    phase: "listening" as const,
    status: { source: "Automatic", separator: "→", target: "Chinese" },
    settings: { ...useStore.getState().settings, sourceLanguage: "auto" as const, targetLanguage: "zh" as const },
    isPaused: false,
    isWaitingForFinalTranslation: false,
    expanded: false,
    onToggle: () => {},
    onWidthChange,
  };
  try {
    setStoredUiLanguage("zh");
    await act(async () => root.render(<LanguageStatusCapsule {...props} />));
    expect(onWidthChange).toHaveBeenCalledExactlyOnceWith(149);
    expect(callbacks).toHaveLength(1);
    width = 148.8;
    callbacks[0]();
    expect(onWidthChange).toHaveBeenCalledOnce();

    vi.stubGlobal("devicePixelRatio", 1.5);
    window.dispatchEvent(new Event("resize"));
    callbacks[0]();
    expect(onWidthChange).toHaveBeenCalledTimes(1 + windowsDpiReports);

    setStoredUiLanguage("ja");
    width = 198.4;
    await act(async () => root.render(<LanguageStatusCapsule {...props} phase="error" />));
    expect(host.textContent).toContain("エラー");
    callbacks[0]();
    expect(onWidthChange).toHaveBeenLastCalledWith(199);

    setStoredUiLanguage("en");
    width = 245.1;
    await act(async () => root.render(<LanguageStatusCapsule {...props} isPaused />));
    expect(host.textContent).toContain("Paused");
    callbacks[0]();
    expect(onWidthChange).toHaveBeenLastCalledWith(246);

    width = 280;
    await act(async () => root.render(<LanguageStatusCapsule {...props} expanded />));
    expect(disconnect).toHaveBeenCalledOnce();
    expect(callbacks).toHaveLength(1);
    expect(onWidthChange).toHaveBeenCalledTimes(3 + windowsDpiReports);
    callbacks[0]();
    window.dispatchEvent(new Event("resize"));
    expect(onWidthChange).toHaveBeenCalledTimes(3 + windowsDpiReports);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    bounds.mockRestore();
    userAgent.mockRestore();
    setStoredUiLanguage("system");
    vi.unstubAllGlobals();
  }
});

it.each(["zh", "en", "ja"] as const)("keeps %s visible and accessible capsule labels free of translation mode names", async (language) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  setStoredUiLanguage(language);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    for (const translationMode of ["lowLatency", "highQuality", "turbo"] as const) {
      for (const expanded of [false, true]) {
        await act(async () => root.render(<LanguageStatusCapsule
          phase="listening"
          status={{ source: "Japanese", separator: "→", target: "Chinese" }}
          settings={{ ...useStore.getState().settings, translationMode }}
          isPaused={false}
          isWaitingForFinalTranslation={false}
          expanded={expanded}
          onToggle={() => {}}
        />));
        const button = host.querySelector("button")!;
        const allLabels = `${button.textContent} ${button.title} ${button.getAttribute("aria-label")}`;
        expect(allLabels).not.toMatch(/Turbo|极速|最速|低延迟|低遅延|高质量|高品質|Live|Quality/i);
        expect(host.querySelector(".overlay-control-island__divider, .overlay-control-island__mode")).toBeNull();
        expect(button.querySelector("svg")).not.toBeNull();
      }
    }
  } finally {
    await act(async () => root.unmount());
    host.remove();
    setStoredUiLanguage("system");
    vi.unstubAllGlobals();
  }
});
