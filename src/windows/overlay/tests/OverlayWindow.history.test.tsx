// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setStoredUiLanguage } from "../../../lib/i18n";
import { useStore } from "../../../lib/store";
import type { AudioSource, SourceSubtitleSnapshot, SubtitleSnapshot } from "../../../lib/types";
import { OverlayWindow } from "../OverlayWindow";

vi.mock("../../../lib/ipc", async original => ({
  ...await original<typeof import("../../../lib/ipc")>(),
  isTauri: false,
  hasEmbeddedOverlayControls: false,
}));
vi.mock("../status/PulseRing", () => ({ PulseRing: () => null }));
vi.mock("../interaction/ResizeHandles", () => ({ ResizeHandles: () => null }));
vi.mock("../development/DevelopmentOverlayTrace", () => ({ DevelopmentOverlayTrace: () => null }));

const initial = useStore.getState();
const ROW_HEIGHT = 100;
const VIEWPORT_HEIGHT = 80;
let root: Root, host: HTMLDivElement;
let positions: WeakMap<HTMLElement, number>;
const originalScrollTo = HTMLElement.prototype.scrollTo;
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalScrollTop = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop")!;
const originalScrollHeight = Object.getOwnPropertyDescriptor(Element.prototype, "scrollHeight")!;
const originalClientHeight = Object.getOwnPropertyDescriptor(Element.prototype, "clientHeight")!;

function isTimeline(element: Element) { return element.classList.contains("overlay-timeline"); }
function maximumTop(element: HTMLElement) { return Math.max(0, element.children.length * ROW_HEIGHT - VIEWPORT_HEIGHT); }
function top(element: HTMLElement) { return Math.min(positions.get(element) ?? 0, maximumTop(element)); }
function timeline() { return host.querySelector<HTMLDivElement>(".overlay-timeline")!; }
function rows() { return Array.from(timeline().children) as HTMLElement[]; }
function committedRows() { return rows().filter(row => /(?:^|:)history-/.test(row.dataset.utteranceId!)); }
function liveRows() { return rows().filter(row => row.dataset.utteranceId!.includes("live-")); }
function returnButton() { return host.querySelector<HTMLButtonElement>(".overlay-return-to-live"); }

function history(count = 12, dual = false): SubtitleSnapshot["history"] {
  return Array.from({ length: count }, (_, index) => ({
    createdAt: index + 1,
    audioSource: dual && index % 2 ? "microphone" : "system",
    source: `Synthetic original ${index + 1}.`,
    translation: `合成译文 ${index + 1}。`,
  }));
}

function snapshot(entries: SubtitleSnapshot["history"], dual = false, live = false): SubtitleSnapshot {
  const sourceSnapshot = (audioSource: AudioSource): SourceSubtitleSnapshot => {
    const own = entries.filter(pair => (pair.audioSource ?? "system") === audioSource);
    const last = own.at(-1);
    const preview = live ? {
      utteranceId: `${audioSource}:current`,
      source: `Synthetic ${audioSource} live original.`,
      translation: `${audioSource} 实时译文。`,
    } : null;
    return {
      audioSource, history: own, detectedLanguage: "en", isTranslationPending: live, isTranslationTimedOut: false,
      source: { text: preview?.source ?? last?.source ?? "", isFinal: !live },
      translation: { text: preview?.translation ?? last?.translation ?? "", isFinal: !live },
      displayPair: preview ?? (last ? { source: last.source, translation: last.translation } : null),
      displayPairFinal: !live,
      previewPair: preview,
    };
  };
  const system = sourceSnapshot("system");
  return { ...system, history: entries, ...(dual ? { tracks: [system, sourceSnapshot("microphone")] } : {}) };
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  positions = new WeakMap();
  Object.defineProperties(Element.prototype, {
    scrollHeight: { configurable: true, get() { return isTimeline(this) ? this.children.length * ROW_HEIGHT : 22; } },
    clientHeight: { configurable: true, get() { return isTimeline(this) ? VIEWPORT_HEIGHT : 22; } },
    scrollTop: {
      configurable: true,
      get() { return isTimeline(this) ? top(this) : 0; },
      set(value: number) { if (isTimeline(this)) positions.set(this, Math.max(0, Math.min(value, maximumTop(this)))); },
    },
  });
  HTMLElement.prototype.getBoundingClientRect = function () {
    if (isTimeline(this)) return { top: 0, bottom: VIEWPORT_HEIGHT, height: VIEWPORT_HEIGHT } as DOMRect;
    if (this.parentElement && isTimeline(this.parentElement)) {
      const index = Array.from(this.parentElement.children).indexOf(this);
      const rowTop = index * ROW_HEIGHT - top(this.parentElement);
      return { top: rowTop, bottom: rowTop + ROW_HEIGHT, height: ROW_HEIGHT } as DOMRect;
    }
    return originalRect.call(this);
  };
  HTMLElement.prototype.scrollTo = function (options: ScrollToOptions | number = {}) {
    if (!isTimeline(this) || typeof options === "number") return;
    const previous = this.scrollTop;
    this.scrollTop = options.top ?? previous;
    if (this.scrollTop !== previous) {
      // Browsers emit layout/programmatic scroll asynchronously. These events
      // must not turn expansion or trimming back into user reading intent.
      queueMicrotask(() => this.dispatchEvent(new Event("scroll")));
    }
  };
  setStoredUiLanguage("en");
  useStore.setState({
    ...initial,
    session: { ...initial.session, status: { kind: "listening" }, isActive: true, atomicPreview: true,
      detectedLanguage: "en", subtitles: snapshot(history()) },
    settings: { ...initial.settings, audioInput: "system", sourceLanguage: "en", targetLanguage: "zh",
      subtitleDisplayMode: "bilingual", pulseAnimation: false, subtitleAnimation: false,
      customSubtitleCountEnabled: true, visibleSubtitleCount: 3 },
  }, true);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  useStore.setState(initial, true);
  setStoredUiLanguage("system");
  HTMLElement.prototype.scrollTo = originalScrollTo;
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  Object.defineProperties(Element.prototype, {
    scrollTop: originalScrollTop, scrollHeight: originalScrollHeight, clientHeight: originalClientHeight,
  });
  vi.unstubAllGlobals();
});

async function mount(count = 3, dual = false, live = false) {
  useStore.setState(state => ({
    settings: { ...state.settings, visibleSubtitleCount: count, audioInput: dual ? "both" : "system" },
    session: { ...state.session, subtitles: snapshot(history(12, dual), dual, live) },
  }));
  await act(async () => root.render(<OverlayWindow />));
}

async function key(key: string) {
  await act(async () => timeline().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })));
}

async function scrollToRead(top: number) {
  await act(async () => {
    timeline().dispatchEvent(new WheelEvent("wheel", { deltaY: -30, bubbles: true }));
  });
  await act(async () => {
    timeline().scrollTop = top;
    timeline().dispatchEvent(new Event("scroll"));
  });
}

async function updateHistory(count: number, dual = false, live = false) {
  await act(async () => useStore.setState(state => ({
    session: { ...state.session, subtitles: snapshot(history(count, dual), dual, live) },
  })));
}

it.each([1, 3, 10])("expands the bounded history from a %i-caption live window and keeps the reading anchor through new confirmations", async count => {
  await mount(count);
  expect(committedRows()).toHaveLength(count);
  expect(returnButton()).toBeNull();
  const current = rows().at(-1)!;
  const currentOffset = current.getBoundingClientRect().top;
  await act(async () => timeline().dispatchEvent(new WheelEvent("wheel", { deltaY: -30, bubbles: true })));
  expect(committedRows()).toHaveLength(12);
  expect(rows().at(-1)).toBe(current);
  expect(current.getBoundingClientRect().top).toBe(currentOffset);
  expect(returnButton()).not.toBeNull();

  await scrollToRead(35);
  const reading = rows()[0];
  expect(reading.dataset.utteranceId).toBe("system:history-1");
  await updateHistory(13);
  expect(committedRows()).toHaveLength(13);
  expect(rows()[0]).toBe(reading);
  expect(reading.getBoundingClientRect().top).toBe(-35);

  await act(async () => returnButton()!.click());
  expect(committedRows()).toHaveLength(count);
  expect(committedRows().at(-1)!.dataset.utteranceId).toBe("system:history-13");
  expect(returnButton()).toBeNull();
  expect(timeline().scrollTop).toBe(maximumTop(timeline()));
  await updateHistory(14);
  expect(committedRows()).toHaveLength(count);
  expect(timeline().scrollTop).toBe(maximumTop(timeline()));
});

it.each([1, 3, 10])("keeps both source tails and their row identities outside the %i-confirmation limit", async count => {
  await mount(count, true, true);
  expect(committedRows()).toHaveLength(count);
  expect(liveRows()).toHaveLength(2);
  const tails = liveRows();
  expect(tails.map(row => row.dataset.utteranceId)).toEqual([
    "system:live-utterance-system:current", "microphone:live-utterance-microphone:current",
  ]);
  await key("Home");
  expect(committedRows()).toHaveLength(12);
  expect(timeline().scrollTop).toBe(0);
  await updateHistory(13, true, true);
  expect(committedRows()).toHaveLength(13);
  expect(timeline().scrollTop).toBe(0);
  expect(liveRows()).toEqual(tails);
  await key("End");
  expect(committedRows()).toHaveLength(count);
  expect(liveRows()).toEqual(tails);
  expect(returnButton()).toBeNull();
  expect(timeline().scrollTop).toBe(maximumTop(timeline()));
});

it("keeps a reader through display/count changes and lets an explicit same-frame return win over the mode anchor", async () => {
  await mount(3);
  await scrollToRead(35);
  const reading = rows()[0];
  await act(async () => useStore.setState(state => ({
    settings: { ...state.settings, visibleSubtitleCount: 1, subtitleDisplayMode: "original" },
  })));
  expect(committedRows()).toHaveLength(12);
  expect(rows()[0]).toBe(reading);
  expect(timeline().scrollTop).toBe(35);
  await act(async () => {
    useStore.setState(state => ({ settings: { ...state.settings, subtitleDisplayMode: "translation" } }));
    returnButton()!.click();
  });
  expect(committedRows()).toHaveLength(1);
  expect(returnButton()).toBeNull();
  expect(timeline().scrollTop).toBe(maximumTop(timeline()));
  await updateHistory(13);
  expect(committedRows()).toHaveLength(1);
  expect(returnButton()).toBeNull();
});

it("returns to the limited live window after downward scrolling reaches the tail", async () => {
  await mount(3);
  await scrollToRead(35);
  expect(committedRows()).toHaveLength(12);
  await act(async () => {
    timeline().dispatchEvent(new WheelEvent("wheel", { deltaY: 50, bubbles: true }));
    timeline().scrollTop = maximumTop(timeline());
    timeline().dispatchEvent(new Event("scroll"));
  });
  expect(committedRows()).toHaveLength(3);
  expect(returnButton()).toBeNull();
  expect(timeline().scrollTop).toBe(maximumTop(timeline()));
});

it("clears the reading owner with empty content and starts the next captions in live mode", async () => {
  await mount(1);
  await key("Home");
  expect(committedRows()).toHaveLength(12);
  await act(async () => useStore.setState(state => ({
    session: { ...state.session, subtitles: { source: { text: "", isFinal: false }, translation: { text: "", isFinal: false }, history: [] } },
  })));
  expect(host.querySelector(".overlay-timeline")).toBeNull();
  expect(returnButton()).toBeNull();
  await updateHistory(4);
  expect(committedRows()).toHaveLength(1);
  expect(returnButton()).toBeNull();
});

it("keeps all bounded history in the unrestricted setting and restores the limit after leaving reading", async () => {
  await mount(3);
  await act(async () => useStore.setState(state => ({
    settings: { ...state.settings, customSubtitleCountEnabled: false },
  })));
  expect(committedRows()).toHaveLength(12);
  await key("Home");
  await act(async () => useStore.setState(state => ({
    settings: { ...state.settings, customSubtitleCountEnabled: true },
  })));
  expect(committedRows()).toHaveLength(12);
  await key("End");
  expect(committedRows()).toHaveLength(3);
});
