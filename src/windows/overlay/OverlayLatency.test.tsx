// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import type { SessionStateEvent } from "../../lib/types";
import { OverlayLatency } from "./OverlayLatency";
import { formatLatency, latencyTone } from "./latencyFormat";

let host: HTMLDivElement;
let root: Root;
const session: SessionStateEvent = {
  status: { kind: "listening" }, isActive: true, isPaused: false,
  isOverlayCollapsed: false, detectedLanguage: null,
  isTranslationPending: false, isTranslationTimedOut: false,
  subtitles: { source: { text: "", isFinal: false }, translation: { text: "", isFinal: false }, history: [] },
};
beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  setStoredUiLanguage("en");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); setStoredUiLanguage("system"); });
async function render(overrides: Partial<SessionStateEvent> = {}, translationRequired = true) {
  await act(async () => root.render(<OverlayLatency session={{ ...session, ...overrides }} translationRequired={translationRequired} />));
}
const measured = { apiLatencyMs: 120, translationLatencyMs: 700, translationLatencyKind: "request" as const };

describe("overlay timing observations", () => {
  it("keeps unavailable and invalid samples distinct from a measured zero", () => {
    for (const value of [null, undefined, Number.NaN, Number.POSITIVE_INFINITY, -1]) {
      expect(formatLatency(value)).toBe("—");
      expect(latencyTone(value, "api")).toBe("neutral");
      expect(latencyTone(value, "translation")).toBe("neutral");
    }
    expect(formatLatency(0)).toBe("0 ms");
    expect(formatLatency(238)).toBe("238 ms");
    expect(formatLatency(1250)).toBe("1.3 s");
  });
  it.each([
    [0, 0, "neutral", "neutral"], [499, 999, "neutral", "neutral"],
    [500, 1_000, "warning", "warning"], [1_499, 2_999, "warning", "warning"],
    [1_500, 3_000, "slow", "slow"],
  ] as const)("grades observed API %ims and translation %ims independently", async (apiLatencyMs, translationLatencyMs, apiTone, translationTone) => {
    await render({ apiLatencyMs, translationLatencyMs, translationLatencyKind: "request" });
    const samples = host.querySelectorAll("strong");
    expect(samples[0].dataset.tone).toBe(apiTone); expect(samples[1].dataset.tone).toBe(translationTone);
    expect(samples[0].textContent).toBe(formatLatency(apiLatencyMs)); expect(samples[1].textContent).toBe(formatLatency(translationLatencyMs));
    expect(host.querySelectorAll(".overlay-latency__separator")).toHaveLength(1);
  });
  it("omits absent measurements and shows only the first samples that actually arrive", async () => {
    await render(); expect(host.firstElementChild).toBeNull();
    await render({ apiLatencyMs: 47 });
    expect(host.querySelectorAll("strong")).toHaveLength(1);
    expect(host.textContent).toBe("API47 ms");
    expect(host.querySelector(".overlay-latency__separator")).toBeNull();
    await render({ translationLatencyMs: 600, translationLatencyKind: "request" });
    expect(host.textContent).toBe("Translation600 ms");
    expect(host.querySelector(".overlay-latency__separator")).toBeNull();
    await render(measured); expect(host.querySelectorAll("strong")).toHaveLength(2);
    await render(); expect(host.firstElementChild).toBeNull();
  });
  it.each([null, undefined, Number.NaN, Number.POSITIVE_INFINITY, -1])("omits invalid timing samples %s", async value => {
    await render({ apiLatencyMs: value, translationLatencyMs: value, translationLatencyKind: "request" });
    expect(host.firstElementChild).toBeNull();
  });
  it("requires a known translation boundary rather than inferring it from the API or text route", async () => {
    for (const kind of [null, undefined, "unknown" as "request"]) {
      await render({ apiLatencyMs: 47, translationLatencyMs: 450, translationLatencyKind: kind });
      expect(host.textContent).toBe("API47 ms");
    }
    await render({ ...measured, translationLatencyMs: 0, translationLatencyKind: "follow" });
    expect(host.textContent).toContain("Translation wait0 ms");
    expect(host.querySelector('[aria-label="Translation wait: 0 ms"]')?.getAttribute("title")).toContain("translation arrived first");
  });
  it("shows actual translation work as status, even when that realtime service has no translation measurement", async () => {
    for (const work of [{ isTranslationPending: true }, { isTranslationPreviewPending: true }]) {
      await render(work);
      expect(host.querySelector('[role="status"]')?.textContent).toBe("Translating");
      expect(host.textContent).not.toContain("Translation wait");
      expect(host.querySelector(".overlay-latency__separator")).toBeNull();
    }
    await render({ ...measured, isTranslationPreviewPending: true });
    expect(host.querySelectorAll("strong")[1].textContent).toBe("Translating");
    expect(host.querySelectorAll("strong")[1].dataset.tone).toBe("neutral");
    await render(measured); expect(host.textContent).toContain("Translation700 ms");
  });
  it("retains rate-limit and retry states without labeling them as measured translation duration", async () => {
    for (const [reason, retryScheduled, text] of [
      ["rateLimited", true, "Rate limited; retrying"],
      ["rateLimited", false, "Translation rate limited"],
      ["temporarilyUnavailable", true, "Unavailable; retrying"],
      ["temporarilyUnavailable", false, "Translation unavailable"],
    ] as const) {
      await render({ ...measured, translationLatencyMs: 4_000, translationRecovery: { reason, retryAfterMs: 600, retryScheduled } });
      const status = host.querySelector('[role="status"]');
      expect(status?.textContent).toBe(text); expect(status?.getAttribute("title")).toBe(text);
      expect(status?.querySelector("strong")?.dataset.tone).toBe("neutral");
      expect(host.textContent).not.toContain("4.0 s");
    }
  });
  it("does not display translation samples or work in recognition-only sessions", async () => {
    await render({ ...measured, isTranslationPending: true }, false);
    expect(host.textContent).toBe("API120 ms");
    expect(host.querySelector(".overlay-latency__separator")).toBeNull();
    await render({ isTranslationPending: true }, false); expect(host.firstElementChild).toBeNull();
  });
  it("suppresses samples and work while paused, connecting, stopping, failed or stopped", async () => {
    for (const state of [
      { isPaused: true }, { status: { kind: "connecting" as const } },
      { status: { kind: "stopping" as const } }, { status: { kind: "error" as const, message: "unavailable" } },
      { isActive: false, status: { kind: "idle" as const } },
    ]) {
      await render({ ...measured, isTranslationPending: true, ...state }); expect(host.firstElementChild).toBeNull();
    }
  });
  it.each(["zh", "zh-TW", "en", "ja", "ko", "fr", "de"] as const)("localizes both boundaries and independent statuses in %s", async language => {
    setStoredUiLanguage(language);
    await render(measured); expect(host.textContent).toContain(I18N.overlay.translationLatency);
    await render({ ...measured, translationLatencyKind: "follow" });
    expect(host.textContent).toContain(I18N.overlay.translationFollowLatency);
    expect(host.querySelectorAll("[aria-label]")).toHaveLength(2);
    await render({ apiLatencyMs: 0, isTranslationPending: true });
    expect(host.querySelector('[role="status"]')?.textContent).toBe(I18N.overlay.translating);
    await render(); expect(host.firstElementChild).toBeNull();
  });
});
