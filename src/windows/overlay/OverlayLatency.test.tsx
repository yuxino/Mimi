// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setStoredUiLanguage } from "../../lib/i18n";
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
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
async function render(overrides: Partial<SessionStateEvent> = {}) {
  await act(async () => root.render(<OverlayLatency session={{ ...session, ...overrides }} />));
}

describe("overlay timing observations", () => {
  it("omits recognition API timing for local captions while retaining actual translation timing", async () => {
    await act(async () => root.render(<OverlayLatency session={session} showApiLatency={false} translationRequired={false} />));
    expect(host.textContent).toBe("");
    await act(async () => root.render(<OverlayLatency session={{ ...session, apiLatencyMs: 999, translationLatencyMs: 240 }} showApiLatency={false} />));
    expect(host.textContent).not.toContain("API");
    expect(host.textContent).not.toContain("999");
    expect(host.querySelectorAll("strong")).toHaveLength(1);
    expect(host.querySelector("strong")?.textContent).toBe("240 ms");
    expect(host.querySelector(".overlay-latency__separator")).toBeNull();
  });
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
    [0, 0, "neutral", "neutral"],
    [499, 999, "neutral", "neutral"],
    [500, 1_000, "warning", "warning"],
    [1_499, 2_999, "warning", "warning"],
    [1_500, 3_000, "slow", "slow"],
  ] as const)("grades actual API %ims and translation %ims with their own bands", async (apiLatencyMs, translationLatencyMs, apiTone, translationTone) => {
    await render({ apiLatencyMs, translationLatencyMs, translationLatencyKind: "request" });
    const samples = host.querySelectorAll("strong");
    expect(samples[0].dataset.tone).toBe(apiTone);
    expect(samples[1].dataset.tone).toBe(translationTone);
    expect(samples[0].textContent).toBe(formatLatency(apiLatencyMs));
    expect(samples[1].textContent).toBe(formatLatency(translationLatencyMs));
  });
  it("never uses a previous slow sample to color pending, recovery, or inactive states", async () => {
    const samples = { apiLatencyMs: 2_000, translationLatencyMs: 4_000 };
    for (const state of [
      { isTranslationPending: true },
      { isTranslationPreviewPending: true },
      { translationRecovery: { reason: "rateLimited" as const, retryAfterMs: 1_000, retryScheduled: false } },
      { translationRecovery: { reason: "temporarilyUnavailable" as const, retryAfterMs: 1_000 } },
    ]) {
      await render({ ...samples, ...state });
      expect(host.querySelectorAll("strong")[0].dataset.tone).toBe("slow");
      expect(host.querySelectorAll("strong")[1].dataset.tone).toBe("neutral");
      expect(host.querySelectorAll("strong")[1].textContent).not.toBe("4.0 s");
    }
    for (const state of [
      { isPaused: true }, { status: { kind: "connecting" as const } },
      { status: { kind: "error" as const, message: "unavailable" } },
    ]) {
      await render({ ...samples, ...state });
      expect(Array.from(host.querySelectorAll("strong"), sample => sample.dataset.tone)).toEqual(["neutral", "neutral"]);
    }
  });
  it("shows missing measurements without inventing a timing", async () => {
    await render();
    expect(host.querySelectorAll("strong")[0].textContent).toBe("Pending");
    expect(host.querySelectorAll("strong")[1].textContent).toBe("Pending");
    expect(host.textContent).not.toContain("0 ms");
  });
  it("shows active work and rate-limit recovery instead of an unexplained dash", async () => {
    await render({ isTranslationPending: true });
    expect(host.textContent).toContain("Translating");
    await render({ translationRecovery: { reason: "rateLimited", retryAfterMs: 4000 } });
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Rate limited; retrying");
    await render({ translationRecovery: { reason: "temporarilyUnavailable", retryAfterMs: 600 } });
    expect(host.textContent).toContain("Unavailable; retrying");
    await render({ isPaused: true, translationRecovery: { reason: "rateLimited", retryAfterMs: 4000 } });
    expect(host.textContent).not.toContain("retrying");
  });
  it("shows actual preview work independently of waiting for a final translation", async () => {
    await render({ isTranslationPending: false, isTranslationPreviewPending: true, translationLatencyMs: 450 });
    expect(host.textContent).toContain("Translating");
    await render({ isTranslationPending: false, isTranslationPreviewPending: false, translationLatencyMs: 450 });
    expect(host.textContent).toContain("450 ms");
  });
  it("does not promise a translation measurement in recognition-only sessions", async () => {
    await act(async () => root.render(<OverlayLatency session={session} translationRequired={false} />));
    expect(host.querySelectorAll("strong")).toHaveLength(1);
    expect(host.textContent).not.toContain("Translation");
  });
  it("does not claim a retry remains scheduled after bounded preview retries exhaust", async () => {
    await render({ translationRecovery: { reason: "rateLimited", retryAfterMs: 4000, retryScheduled: false } });
    expect(host.querySelector('[role="status"]')?.textContent).toContain("Translation rate limited");
    expect(host.textContent).not.toContain("retrying");
    await render({ translationRecovery: { reason: "temporarilyUnavailable", retryAfterMs: 600, retryScheduled: false } });
    expect(host.textContent).toContain("Translation unavailable");
    await render({ translationRecovery: { reason: "rateLimited", retryAfterMs: 4000, retryScheduled: true } });
    expect(host.textContent).toContain("Rate limited; retrying");
  });
  it("distinguishes translation request time from matching-final wait", async () => {
    await render({ apiLatencyMs: 120, translationLatencyMs: 700, translationLatencyKind: "request" });
    expect(host.textContent).toContain("Translation700 ms");
    await render({ apiLatencyMs: 120, translationLatencyMs: 0, translationLatencyKind: "follow" });
    expect(host.textContent).toContain("Translation wait0 ms");
    expect(host.querySelector('[aria-label="Translation wait: 0 ms"]')?.getAttribute("title")).toContain("translation arrived first");
  });
  it("suppresses stale observations while paused, connecting, failed, or stopped", async () => {
    const sample = { apiLatencyMs: 120, translationLatencyMs: 700, translationLatencyKind: "request" as const };
    for (const state of [{ isPaused: true }, { status: { kind: "connecting" as const } }, { status: { kind: "error" as const, message: "unavailable" } }]) {
      await render({ ...sample, ...state });
      expect(host.textContent).not.toContain("120 ms");
      expect(host.textContent).not.toContain("700 ms");
    }
    await render({ ...sample, isActive: false, status: { kind: "idle" } });
    expect(host.firstElementChild).toBeNull();
  });
  it.each(["zh", "en", "ja"] as const)("localizes the visible and accessible timing labels in %s", async language => {
    setStoredUiLanguage(language);
    await render({ apiLatencyMs: 120, translationLatencyMs: 200, translationLatencyKind: "follow" });
    expect(host.textContent).toContain({ zh: "译文等待", en: "Translation wait", ja: "訳文待ち" }[language]);
    expect(host.querySelectorAll("[aria-label]").length).toBe(2);
  });
});
