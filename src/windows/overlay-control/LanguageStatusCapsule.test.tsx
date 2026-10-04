// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import { LanguageStatusCapsule } from "./LanguageStatusCapsule";
import { audioInputLabel } from "../../lib/audioInput";

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

it("measures collapsed content initially and after locale/status changes without measuring the expanded header", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
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
    expect(onWidthChange).toHaveBeenCalledTimes(3);
  } finally {
    await act(async () => root.unmount());
    host.remove();
    bounds.mockRestore();
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
