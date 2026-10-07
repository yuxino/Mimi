// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { I18N, setStoredUiLanguage } from "../../lib/i18n";
import { useStore } from "../../lib/store";
import type { SubtitleSnapshot } from "../../lib/types";
import { OverlayWindow } from "./OverlayWindow";

vi.mock("../../lib/ipc", async (original) => ({
  ...await original<typeof import("../../lib/ipc")>(),
  isTauri: true,
  listenOverlayPointerMotion: () => Promise.resolve(() => {}),
}));
vi.mock("./PulseRing", () => ({ PulseRing: () => null }));
vi.mock("./ResizeHandles", () => ({ ResizeHandles: () => null }));

const original = useStore.getState();
const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalScrollTo = HTMLElement.prototype.scrollTo;
const originalClientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
const originalScrollHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollHeight");
const empty: SubtitleSnapshot = {
  source: { text: "", isFinal: false }, translation: { text: "", isFinal: false }, history: [], previewPair: null,
};
const confirmed = { source: "Synthetic original sentence. ".repeat(10), translation: "合成测试译文。".repeat(10), createdAt: 1 };
const subtitles: SubtitleSnapshot = {
  ...empty, source: { text: confirmed.source, isFinal: true },
  translation: { text: confirmed.translation, isFinal: true }, history: [confirmed],
};
let host: HTMLDivElement, root: Root;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("innerWidth", 640); vi.stubGlobal("innerHeight", 136);
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  HTMLElement.prototype.getBoundingClientRect = () => ({ top: 0, bottom: 240, height: 240 } as DOMRect);
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true,
    get(this: HTMLElement) { return Number.parseFloat(this.style.height) || 51; } });
  Object.defineProperty(HTMLElement.prototype, "scrollHeight", { configurable: true, get() { return 300; } });
  HTMLElement.prototype.scrollTo = function (options?: ScrollToOptions | number, y?: number) {
    this.scrollTop = Math.min(typeof options === "number" ? y ?? 0 : options?.top ?? 0, this.scrollHeight - this.clientHeight);
  };
  setStoredUiLanguage("en");
  useStore.setState({ ...original,
    session: { ...original.session, subtitles, status: { kind: "listening" }, isActive: true, isPaused: false,
      isOverlayCollapsed: false, isTranslationPending: false, detectedLanguage: "en" },
    settings: { ...original.settings, subtitleDisplayMode: "bilingual", sourceLanguage: "en", targetLanguage: "zh",
      subtitleBlendsWithBackground: false, isOverlayLocked: false, pulseAnimation: false, subtitleAnimation: false },
    clearSubtitles: async () => useStore.setState(state => ({ session: { ...state.session, subtitles: empty } })),
  }, true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); useStore.setState(original, true); setStoredUiLanguage("system");
  HTMLElement.prototype.getBoundingClientRect = originalRect; HTMLElement.prototype.scrollTo = originalScrollTo;
  for (const [key, descriptor] of [["clientHeight", originalClientHeight], ["scrollHeight", originalScrollHeight]] as const) {
    if (descriptor) Object.defineProperty(HTMLElement.prototype, key, descriptor);
    else Reflect.deleteProperty(HTMLElement.prototype, key);
  }
  vi.unstubAllGlobals();
});

async function mount() { await act(async () => root.render(<OverlayWindow />)); }
function timeline() { return host.querySelector<HTMLDivElement>('[tabindex="0"]')!; }
function returnButton() { return host.querySelector<HTMLButtonElement>(".overlay-return-to-live"); }
async function readHistory() {
  await act(async () => timeline().dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));
}

it("keeps deliberate history reading through mode and pause changes, then returns to compact live following", async () => {
  await mount(); expect(returnButton()).toBeNull();
  const element = timeline();
  await readHistory();
  expect(returnButton()?.textContent).toBe("Back to live");
  expect(element.querySelector("[aria-label]")).toBeNull();
  for (const subtitleDisplayMode of ["translation", "original", "bilingual"] as const) {
    await act(async () => useStore.setState(state => ({ settings: { ...state.settings, subtitleDisplayMode } })));
    expect(timeline()).toBe(element);
    expect(returnButton()).not.toBeNull();
    expect(element.querySelector("[aria-label]")).toBeNull();
  }
  for (const isPaused of [true, false]) {
    await act(async () => useStore.setState(state => ({ session: { ...state.session, isPaused } })));
    expect(returnButton()).not.toBeNull();
    expect(element.querySelector("[aria-label]")).toBeNull();
  }
  await act(async () => returnButton()!.click());
  expect(returnButton()).toBeNull();
  expect(document.activeElement).toBe(element);
  expect(element.scrollTop).toBe(249);
  expect(element.querySelectorAll("[aria-label]")).toHaveLength(2);
  expect(element.textContent).toContain(confirmed.source);
  expect(element.textContent).toContain(confirmed.translation);
});

it("removes the reading action on clear and starts new session content in live following", async () => {
  await mount(); await readHistory(); expect(returnButton()).not.toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.clearSubtitles}"]`)!.click());
  expect(returnButton()).toBeNull(); expect(host.querySelector('[tabindex="0"]')).toBeNull();
  await act(async () => useStore.setState(state => ({ session: { ...state.session, status: { kind: "connecting" }, subtitles: empty } })));
  const next = { source: "New synthetic session.", translation: "新的测试会话。", createdAt: 2 };
  await act(async () => useStore.setState(state => ({ session: { ...state.session, status: { kind: "listening" },
    subtitles: { ...empty, source: { text: next.source, isFinal: true }, translation: { text: next.translation, isFinal: true }, history: [next] } } })));
  expect(returnButton()).toBeNull(); expect(timeline().querySelectorAll("[aria-label]")).toHaveLength(2);
  expect(timeline().textContent).not.toContain(confirmed.source);
});

it("opens an unconfirmed long pair through pause and mode changes and returns it to live following explicitly", async () => {
  const live = { ...empty, source: { text: confirmed.source, isFinal: false },
    previewPair: { source: confirmed.source, translation: confirmed.translation } };
  useStore.setState(state => ({ session: { ...state.session, subtitles: live }, settings: {
    ...state.settings, profiles: [{ id: "atomic", name: "Alibaba", provider: "alibabaCloud", credentialState: "present" }], activeProfileId: "atomic",
  } }));
  await mount(); const element = timeline();
  expect(element.querySelector("[aria-label]")).not.toBeNull();
  await readHistory();
  for (const subtitleDisplayMode of ["original", "translation", "bilingual"] as const) {
    await act(async () => useStore.setState(state => ({ settings: { ...state.settings, subtitleDisplayMode } })));
    expect(timeline()).toBe(element); expect(returnButton()).not.toBeNull();
    expect(element.querySelector("[aria-label]")).toBeNull();
  }
  for (const isPaused of [true, false]) {
    await act(async () => useStore.setState(state => ({ session: { ...state.session, isPaused } })));
    expect(element.querySelector("[aria-label]")).toBeNull(); expect(returnButton()).not.toBeNull();
  }
  expect(element.textContent).toContain(confirmed.source);
  expect(element.textContent).toContain(confirmed.translation);
  expect(useStore.getState().session.subtitles.history).toEqual([]);
  await act(async () => returnButton()!.click());
  expect(returnButton()).toBeNull(); expect(element.scrollTop).toBe(249);
  expect(element.querySelectorAll("[aria-label]")).toHaveLength(2);
  expect(useStore.getState().session.subtitles.history).toEqual([]);
});

it.each(["zh", "en", "ja"] as const)("keeps the return action beside timings and actionable errors at 360×136 in %s", async language => {
  vi.stubGlobal("innerWidth", 360); setStoredUiLanguage(language);
  useStore.setState({ stop: vi.fn().mockRejectedValue(new Error("synthetic-action-failure")) });
  await mount(); await readHistory();
  const row = host.querySelector<HTMLElement>(".overlay-status-row")!;
  expect(row.style.top).toBe("67px");
  expect(returnButton()?.textContent).toBe({ zh: "回到实时", en: "Back to live", ja: "リアルタイムへ" }[language]);
  expect(row.querySelector('[data-testid="overlay-latency"]')).not.toBeNull();
  expect(returnButton()?.closest(".overlay-status-row")).toBe(row);
  expect(timeline().contains(returnButton())).toBe(false);
  await act(async () => host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.closeSubtitles}"]`)!.click());
  expect(row.querySelector('[role="alert"]')?.textContent).toBe(I18N.overlay.controlActionFailed);
  expect(returnButton()).not.toBeNull();
  await act(async () => returnButton()!.click());
  expect(returnButton()).toBeNull();
  expect(row.querySelector('[role="alert"]')).not.toBeNull();
});

it.each(["zh", "en", "ja"] as const)("hides immersive timings while retaining bilingual reading and return to live in %s", async language => {
  vi.stubGlobal("innerWidth", 360); setStoredUiLanguage(language);
  useStore.setState(state => ({ session: { ...state.session, apiLatencyMs: 1_500, translationLatencyMs: 3_000 },
    settings: { ...state.settings, subtitleBlendsWithBackground: true } }));
  await mount();
  expect(host.querySelector('[data-testid="overlay-latency"]')).toBeNull();
  expect(host.querySelector(".overlay-status-row")).toBeNull();
  expect(timeline().textContent).toContain(confirmed.source);
  expect(timeline().textContent).toContain(confirmed.translation);
  await readHistory();
  expect(returnButton()?.textContent).toBe(I18N.overlay.returnToLive);
  expect(host.querySelector('[data-testid="overlay-latency"]')).toBeNull();
  await act(async () => returnButton()!.click());
  expect(returnButton()).toBeNull();
  expect(host.querySelector(".overlay-status-row")).toBeNull();
  expect(timeline().scrollTop).toBe(249);
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: false } })));
  expect(host.querySelector('[data-testid="overlay-latency"]')).not.toBeNull();
  expect(Array.from(host.querySelectorAll(".overlay-latency strong"), element => (element as HTMLElement).dataset.tone)).toEqual(["slow", "slow"]);
});

it("retains pending and failed action feedback after enabling immersion without restoring timings", async () => {
  let fail!: (error: Error) => void;
  useStore.setState({ togglePaused: () => new Promise<void>((_resolve, reject) => { fail = reject; }) });
  await mount();
  await act(async () => host.querySelector<HTMLButtonElement>(`button[aria-label="${I18N.overlay.pause}"]`)!.click());
  await act(async () => useStore.setState(state => ({ settings: { ...state.settings, subtitleBlendsWithBackground: true } })));
  expect(host.querySelector('[data-testid="overlay-latency"]')).toBeNull();
  expect(host.querySelector(".overlay-action-feedback")?.getAttribute("role")).toBe("status");
  await act(async () => fail(new Error("synthetic-action-failure")));
  expect(host.querySelector(".overlay-action-feedback")?.getAttribute("role")).toBe("alert");
  await readHistory();
  expect(returnButton()).not.toBeNull();
  expect(host.querySelector('[data-testid="overlay-latency"]')).toBeNull();
  await act(async () => returnButton()!.click());
  expect(host.querySelector(".overlay-action-feedback")?.getAttribute("role")).toBe("alert");
  await act(async () => useStore.setState(state => ({ session: { ...state.session, isPaused: true } })));
  expect(host.querySelector(".overlay-status-row")).toBeNull();
});
