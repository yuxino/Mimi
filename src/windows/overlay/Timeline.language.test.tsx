// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { Timeline } from "./Timeline";
import type { SubtitleBlock } from "./overlayModel";

it("updates paused source labels without replacing subtitle nodes or scrolling", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const originalScrollTo = HTMLElement.prototype.scrollTo;
  HTMLElement.prototype.scrollTo = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const blocks: SubtitleBlock[] = [
    { id: "system", createdAt: 1, presentation: "history", source: "System sample", translation: null, audioSource: "system" },
    { id: "mic", createdAt: 2, presentation: "latestCommitted", source: "Microphone sample", translation: null, audioSource: "microphone" },
  ];
  try {
    setStoredUiLanguage("en");
    await act(async () => root.render(<Timeline blocks={blocks} fontSize={18} alignment="left" color="white" displayMode="original" audioInput="both" />));
    const timeline = host.firstElementChild as HTMLElement;
    const indicators = Array.from(host.querySelectorAll(".subtitle-audio-source"));
    expect(indicators.map(node => node.getAttribute("aria-label"))).toEqual(["System audio", "Microphone"]);
    timeline.scrollTop = 37;
    for (const language of ["zh-TW", "de", "fr", "ko"] as const) {
      // No new props or subtitle event: React.memo must not retain old labels.
      await act(async () => setStoredUiLanguage(language));
      expect(host.firstElementChild).toBe(timeline);
      expect(timeline.scrollTop).toBe(37);
      for (const [index, node] of indicators.entries()) {
        expect(host.querySelectorAll(".subtitle-audio-source")[index]).toBe(node);
        const expected = index === 0 ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone;
        expect(node.getAttribute("aria-label")).toBe(expected);
        expect(node.getAttribute("title")).toBe(expected);
      }
    }
  } finally {
    await act(async () => root.unmount());
    host.remove();
    setStoredUiLanguage("en");
    HTMLElement.prototype.scrollTo = originalScrollTo;
    vi.unstubAllGlobals();
  }
});
