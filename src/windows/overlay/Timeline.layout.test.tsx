// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Timeline } from "./Timeline";
import type { SubtitleBlock } from "./overlayModel";
import { minimumOverlayHeight, OVERLAY_MAXIMUM_CHROME_HEIGHT } from "./overlayMinimumHeight";

const originalRect = HTMLElement.prototype.getBoundingClientRect;
const originalScrollTo = HTMLElement.prototype.scrollTo;
const clientHeightDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
let root: Root;
let host: HTMLDivElement;
let measuredHeight = 24;
let viewportHeight = 100;
let resize: Array<() => void>;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  resize = [];
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { resize.push(callback); }
    observe() {}
    disconnect() {}
  });
  HTMLElement.prototype.scrollTo = vi.fn();
  HTMLElement.prototype.getBoundingClientRect = function () {
    return { top: 0, bottom: measuredHeight, height: measuredHeight } as DOMRect;
  };
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get(this: HTMLElement) { return Number.parseFloat(this.style.height) || viewportHeight; },
  });
  measuredHeight = 24;
  viewportHeight = 100;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  HTMLElement.prototype.scrollTo = originalScrollTo;
  if (clientHeightDescriptor) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeightDescriptor);
  else delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
  vi.unstubAllGlobals();
});

const confirmed: SubtitleBlock = {
  id: "confirmed",
  createdAt: 1,
  presentation: "latestCommitted",
  // Exercise long-sentence layout rather than the extreme-repeat projection.
  source: Array.from({ length: 30 }, (_, index) => `完整原文${index}。`).join(""),
  translation: Array.from({ length: 30 }, (_, index) => `完整译文${index}。`).join(""),
};
const live: SubtitleBlock = {
  id: "live", createdAt: null, presentation: "live",
  source: "新的流式原文。", translation: "新的流式译文。", streaming: true,
};

async function render(blocks: SubtitleBlock[], fontSize = 18, motionEnabled = false, showSubtitleDividers = false, displayMode: "original" | "translation" | "bilingual" = "bilingual") {
  await act(async () => root.render(<Timeline blocks={blocks} fontSize={fontSize} alignment="center" color="white" displayMode={displayMode} motionEnabled={motionEnabled} showSubtitleDividers={showSubtitleDividers} />));
  return host.firstElementChild as HTMLDivElement;
}

it("keeps full confirmed history open when sentence dividers are toggled", async () => {
  measuredHeight = 240;
  const blocks = [{ ...confirmed, presentation: "history" as const }, live];
  const timeline = await render(blocks, 18, false, true);
  await act(async () => timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));
  const read = timeline.querySelector<HTMLElement>('[data-utterance-id="confirmed"]')!;
  expect(read.querySelector("[aria-label]:not([role])")).toBeNull();
  expect(timeline.querySelector(".subtitle-separator")).not.toBeNull();
  await render(blocks, 18, false, false);
  expect(timeline.querySelector('[data-utterance-id="confirmed"]')).toBe(read);
  expect(read.querySelector("[aria-label]:not([role])")).toBeNull();
  expect(read.textContent).toContain(confirmed.source);
  expect(read.textContent).toContain(confirmed.translation);
  expect(timeline.querySelector(".subtitle-separator")).toBeNull();
});

it("uses only the actual line height for a short translation, then bounds later growth", async () => {
  const timeline = await render([{ ...confirmed, source: null, translation: "短句" }]);
  let viewport = timeline.querySelector<HTMLElement>('[aria-label="短句"]')!;
  expect(viewport.style.height).toBe("24px"); // Two reserved lines would waste another 24px.
  measuredHeight = 240;
  await act(async () => { resize.forEach(callback => callback()); });
  viewport = timeline.querySelector<HTMLElement>('[aria-label="短句"]')!;
  expect(viewport.style.height).toBe("72px");
  expect(viewport.firstElementChild?.textContent).toBe("短句");
});

it.each(["Wait... really?", "等等……真的吗？", "待って…本当？", "잠깐... 정말?"])("keeps plain %s text and measured lane heights stable as streaming settles", async (source) => {
  // Model a phrase at the wrapping edge: an extra inline element would take
  // another visual line. The body's only child must remain its text node.
  HTMLElement.prototype.getBoundingClientRect = function () {
    const isBody = this.tagName === "SPAN" && this.parentElement?.hasAttribute("aria-label");
    const lineHeight = Math.round((Number.parseFloat(this.style.fontSize) || 18) * 1.32);
    const height = isBody ? lineHeight * (this.childElementCount > 0 ? 2 : 1) : 24;
    return { top: 0, bottom: height, height } as DOMRect;
  };
  for (const displayMode of ["original", "translation", "bilingual"] as const) {
    const block = { ...live, source, translation: "Plain translation..." };
    const mount = async (streaming?: true) => {
      await act(async () => root.render(<Timeline blocks={[{ ...block, streaming }]} fontSize={18}
        alignment="center" color="white" displayMode={displayMode} motionEnabled={false} />));
    };
    await mount(true);
    const lanes = Array.from(host.querySelectorAll<HTMLElement>("[aria-label]:not([role])"));
    const initialHeights = lanes.map(lane => lane.style.height);
    for (const lane of lanes) {
      const body = lane.firstElementChild!;
      expect(body.children).toHaveLength(0);
      expect(body.firstChild?.nodeType).toBe(Node.TEXT_NODE);
      expect(body.textContent).toBe(lane.getAttribute("aria-label"));
      expect(Number.parseFloat(lane.style.height)).toBeLessThanOrEqual(24);
    }
    await mount();
    await act(async () => { resize.forEach(callback => callback()); });
    expect(Array.from(host.querySelectorAll<HTMLElement>("[aria-label]:not([role])"))).toEqual(lanes);
    expect(lanes.map(lane => lane.style.height)).toEqual(initialHeights);
    await mount(true);
    await act(async () => { resize.forEach(callback => callback()); });
    expect(lanes.map(lane => lane.style.height)).toEqual(initialHeights);
    expect(host.querySelector(".stream-dots")).toBeNull();
  }
});

it("keeps genuine source ellipsis without adding a second marker or fading readable text", async () => {
  measuredHeight = 240;
  const source = "Genuine source... keeps going";
  const timeline = await render([{ ...confirmed, source, translation: null }]);
  const lane = timeline.querySelector<HTMLElement>(`[aria-label="${source}"]`)!;
  expect(lane.firstElementChild?.textContent).toBe(source);
  expect(lane.children).toHaveLength(1);
  expect(lane.style.maskImage).toBe("");
  expect(lane.querySelector(".stream-dots")).toBeNull();
});

it("keeps the same confirmed long sentence bounded when the next live sentence appears", async () => {
  measuredHeight = 240;
  const timeline = await render([confirmed]);
  const before = timeline.querySelector<HTMLElement>('[data-utterance-id="confirmed"]')!;
  const viewport = before.querySelector<HTMLElement>("[aria-label]:not([role])");
  expect(before.textContent).toContain(confirmed.source);
  expect(before.textContent).toContain(confirmed.translation);
  await render([{ ...confirmed, presentation: "history" }, live]);
  const after = timeline.querySelector<HTMLElement>('[data-utterance-id="confirmed"]')!;
  expect(after).toBe(before);
  expect(after.querySelector("[aria-label]:not([role])")).toBe(viewport);
  expect(after.textContent).toContain(confirmed.source);
  expect(after.textContent).toContain(confirmed.translation);
});

it.each(["original", "translation", "bilingual"] as const)("keeps a fitting three-line confirmed phrase readable when the next short phrase starts in %s", async displayMode => {
  viewportHeight = 480;
  const source = "The first sentence describes a room. The second describes a door. The last ends with tomorrow.";
  const translation = "第一句描述房间。第二句描述门。最后一句关于明天。";
  HTMLElement.prototype.getBoundingClientRect = function () {
    const line = Number.parseFloat(this.style.lineHeight) || 24;
    const height = this.textContent === source || this.textContent === translation ? line * 3 : line;
    return { top: 0, bottom: height, height } as DOMRect;
  };
  const phrase = { ...confirmed, source, translation };
  const timeline = await render([phrase], 18, false, false, displayMode);
  const row = timeline.querySelector<HTMLElement>('[data-utterance-id="confirmed"]')!;
  const before = Array.from(row.querySelectorAll<HTMLElement>("[aria-label]:not([role])"));
  const heights = before.map(lane => Number.parseFloat(lane.style.height));
  expect(before.map(lane => lane.textContent)).toEqual(displayMode === "original" ? [source]
    : displayMode === "translation" ? [translation] : [source, translation]);
  for (const lane of before) {
    expect(Number.parseFloat(lane.style.height)).toBe(Number.parseFloat((lane.firstElementChild as HTMLElement).style.lineHeight) * 3);
  }

  await render([{ ...phrase, presentation: "history" }, live], 18, false, false, displayMode);
  const previous = timeline.querySelector<HTMLElement>('[data-utterance-id="confirmed"]')!;
  const after = Array.from(previous.querySelectorAll<HTMLElement>("[aria-label]:not([role])"));
  expect(previous).toBe(row);
  expect(after).toEqual(before);
  expect(after.map(lane => Number.parseFloat(lane.style.height))).toEqual(heights);
  expect(previous.textContent).toContain(displayMode === "translation" ? translation : source);
  expect(timeline.querySelector('[data-utterance-id="live"]')?.textContent).toContain(displayMode === "translation" ? live.translation : live.source);
});

it("keeps both previous bilingual lanes visible and restores full text on reading intent", async () => {
  measuredHeight = 240;
  const timeline = await render([{ ...confirmed, presentation: "history" }, live]);
  const previous = timeline.querySelector<HTMLElement>('[data-utterance-id="confirmed"]')!;
  const source = previous.querySelector<HTMLElement>(`[aria-label="${confirmed.source}"]`)!;
  const translation = previous.querySelector<HTMLElement>(`[aria-label="${confirmed.translation}"]`)!;
  expect(source.hidden).toBe(false);
  expect(source.getAttribute("aria-hidden")).toBeNull();
  expect(source.style.height).toBe("44px");
  expect(translation.style.height).toBe("48px");
  expect(Number.parseFloat(source.style.height) + Number.parseFloat(translation.style.height) + 2 + 2 + 2).toBeLessThanOrEqual(viewportHeight);
  const newest = timeline.querySelector<HTMLElement>('[data-utterance-id="live"]')!;
  expect(newest.querySelector<HTMLElement>("[aria-label]:not([role])")!.hidden).toBe(false);
  await act(async () => timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));
  expect(previous.querySelector("[aria-label]:not([role])")).toBeNull();
  expect(previous.textContent).toContain(confirmed.source);
  expect(previous.textContent).toContain(confirmed.translation);
  await act(async () => timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "End" })));
  expect(previous.querySelector<HTMLElement>(`[aria-label="${confirmed.source}"]`)!.hidden).toBe(false);
  expect(previous.querySelector<HTMLElement>(`[aria-label="${confirmed.source}"]`)!.style.height).toBe("44px");
  expect(previous.querySelector<HTMLElement>(`[aria-label="${confirmed.translation}"]`)!.style.height).toBe("48px");
});

it("reveals full confirmed history on upward intent and returns to compact following with End", async () => {
  measuredHeight = 240;
  const timeline = await render([{ ...confirmed, presentation: "history" }, live]);
  await act(async () => { timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })); });
  const read = timeline.querySelector<HTMLElement>('[data-utterance-id="confirmed"]')!;
  expect(read.querySelector("[aria-label]:not([role])")).toBeNull();
  expect(read.textContent).toContain(confirmed.source);
  expect(read.textContent).toContain(confirmed.translation);
  expect(timeline.querySelector('[data-utterance-id="live"] [aria-label]:not([role])')).toBeNull();
  await render([{ ...confirmed, presentation: "history" }, { ...live, translation: "仍然流入" }]);
  expect(read.querySelector("[aria-label]:not([role])")).toBeNull();
  await act(async () => { timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "End" })); });
  expect(read.querySelector("[aria-label]:not([role])")).not.toBeNull();
});

it.each(["original", "translation", "bilingual"] as const)("opens the complete lone live sentence on reading intent and returns to bounded lines with End in %s", async displayMode => {
  measuredHeight = 240;
  viewportHeight = 51;
  const longLive = { ...confirmed, id: "live", createdAt: null, presentation: "live" as const, streaming: true as const };
  const timeline = await render([longLive], 20, false, false, displayMode);
  const row = timeline.querySelector('[data-utterance-id="live"]')!;
  expect(row.querySelector("[aria-label]:not([role])")).not.toBeNull();
  await act(async () => timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));
  expect(timeline.querySelector('[data-utterance-id="live"]')).toBe(row);
  expect(row.querySelector("[aria-label]:not([role])")).toBeNull();
  const visible = Array.from(row.querySelectorAll<HTMLElement>(".subtitle-lane"));
  expect(visible.map(lane => lane.textContent)).toEqual(displayMode === "original" ? [longLive.source]
    : displayMode === "translation" ? [longLive.translation] : [longLive.source, longLive.translation]);
  expect(visible.every(lane => lane.style.overflow === "" && lane.parentElement?.style.overflow !== "hidden")).toBe(true);
  await act(async () => timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "End" })));
  expect(timeline.querySelector('[data-utterance-id="live"]')).toBe(row);
  expect(row.querySelector("[aria-label]:not([role])")).not.toBeNull();
});

it("opens a single compact confirmed sentence without needing a scrollbar, while clicks keep following", async () => {
  measuredHeight = 240;
  const timeline = await render([confirmed]);
  await act(async () => { timeline.dispatchEvent(new Event("pointerdown", { bubbles: true })); });
  expect(timeline.querySelector("[aria-label]:not([role])")).not.toBeNull();
  await act(async () => { timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowUp" })); });
  expect(timeline.querySelector("[aria-label]:not([role])")).toBeNull();
  // With no remaining overflow, a downward wheel must restore following even
  // though the browser does not produce a scroll event.
  await act(async () => { timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 30 })); });
  expect(timeline.querySelector("[aria-label]:not([role])")).not.toBeNull();
});

it("fits both long bilingual lanes into the actual 51px body and uses added space after resize", async () => {
  measuredHeight = 240;
  viewportHeight = 51;
  const timeline = await render([confirmed], 20);
  const row = timeline.firstElementChild as HTMLElement;
  const lanes = Array.from(row.querySelectorAll<HTMLElement>("[aria-label]:not([role])"));
  const laneHeights = lanes.map(lane => Number.parseFloat(lane.style.height));
  expect(laneHeights).toEqual([22, 27]);
  expect(laneHeights.reduce((sum, height) => sum + height, 0) + 1 + 1).toBeLessThanOrEqual(viewportHeight);
  expect(row.style.paddingTop).toBe("0px");
  viewportHeight = 130;
  await act(async () => { resize.forEach(callback => callback()); });
  const resizedHeights = lanes.map(lane => Number.parseFloat(lane.style.height));
  expect(resizedHeights).toEqual([48, 54]);
  expect(resizedHeights.reduce((sum, height) => sum + height, 0) + 2 + 3 + 2)
    .toBeLessThanOrEqual(viewportHeight);
  expect(row.textContent).toContain(confirmed.source);
  expect(row.textContent).toContain(confirmed.translation);
});

it("uses a downward finger gesture to open full confirmed text while keeping touch taps compact", async () => {
  measuredHeight = 240;
  const timeline = await render([confirmed]);
  const touch = (type: string, y: number) => {
    const event = new Event(type, { bubbles: true });
    Object.defineProperty(event, "touches", { value: [{ clientY: y }] });
    timeline.dispatchEvent(event);
  };
  await act(async () => { touch("touchstart", 20); });
  expect(timeline.querySelector("[aria-label]:not([role])")).not.toBeNull();
  await act(async () => { touch("touchmove", 40); });
  expect(timeline.querySelector("[aria-label]:not([role])")).toBeNull();
});

it.each([80, 81])("fits long original and short translation at the %ipx responsive boundary", async (height) => {
  viewportHeight = height;
  HTMLElement.prototype.getBoundingClientRect = function () {
    const height = this.textContent === "短句" ? 27 : 240;
    return { top: 0, bottom: height, height } as DOMRect;
  };
  const timeline = await render([{ ...confirmed, translation: "短句" }], 20);
  const row = timeline.firstElementChild as HTMLElement;
  const lanes = Array.from(row.querySelectorAll<HTMLElement>("[aria-label]:not([role])"));
  const textHeight = lanes.reduce((sum, lane) => sum + Number.parseFloat(lane.style.height), 0);
  expect(textHeight + 7).toBeLessThanOrEqual(viewportHeight);
  expect(lanes[0].style.height).toBe("44px");
});

it.each([83, 84, 85])("keeps the reference font stable as translation arrives and leaves at the %ipx boundary", async (height) => {
  viewportHeight = height;
  measuredHeight = 240;
  const waiting = { ...live, translation: null };
  const timeline = await render([waiting], 20);
  const original = timeline.querySelector<HTMLElement>(`[aria-label="${live.source}"]`)!;
  const text = original.firstElementChild as HTMLElement;
  const font = text.style.fontSize;
  const lineHeight = text.style.lineHeight;
  expect(font).toBe(height < 85 ? "16.4px" : "18px");

  await render([live], 20);
  expect(timeline.querySelector(`[aria-label="${live.source}"]`)).toBe(original);
  expect(original.firstElementChild).toBe(text);
  expect(text.style.fontSize).toBe(font);
  expect(text.style.lineHeight).toBe(lineHeight);
  const pairedLanes = Array.from(timeline.querySelectorAll<HTMLElement>("[aria-label]:not([role])"));
  expect(pairedLanes.map(lane => lane.firstElementChild?.textContent)).toEqual([live.source, live.translation]);
  expect(pairedLanes.reduce((sum, lane) => sum + Number.parseFloat(lane.style.height), 0) + 7).toBeLessThanOrEqual(height);

  await render([waiting], 20);
  expect(original.firstElementChild).toBe(text);
  expect(text.style.fontSize).toBe(font);
  expect(text.style.lineHeight).toBe(lineHeight);
});

it("gives a bilingual translation the unused original lane's space before that original arrives", async () => {
  measuredHeight = 240;
  viewportHeight = 51;
  const timeline = await render([{ ...live, source: null }]);
  expect(timeline.querySelector<HTMLElement>("[aria-label]:not([role])")?.style.height).toBe("48px");
});

it("rolls a newly clipped line up from its previous position without first opening a gap below the text", async () => {
  viewportHeight = 60;
  measuredHeight = 48;
  const timeline = await render([{ ...confirmed, source: null }], 18, true);
  const inner = timeline.querySelector<HTMLElement>("[aria-label]:not([role])")!.firstElementChild as HTMLElement;
  const transforms = vi.spyOn(inner.style, "transform", "set");
  measuredHeight = 72;
  await act(async () => { resize.forEach(callback => callback()); });
  expect(transforms.mock.calls.map(([value]) => value)).toEqual(["translateY(24px)", "translateY(0)"]);
  expect(inner.style.transition).toBe("transform 180ms ease-out");
  transforms.mockRestore();
});

it("does not glide a new line that still fits the compact lane", async () => {
  measuredHeight = 24;
  const timeline = await render([{ ...confirmed, source: null }], 18, true);
  const inner = timeline.querySelector<HTMLElement>("[aria-label]:not([role])")!.firstElementChild as HTMLElement;
  const transforms = vi.spyOn(inner.style, "transform", "set");
  measuredHeight = 48;
  await act(async () => { resize.forEach(callback => callback()); });
  expect(transforms.mock.calls.map(([value]) => value)).toEqual(["translateY(0)"]);
  expect(inner.style.transition).toBe("none");
  transforms.mockRestore();
});

it("stops an active glide and keeps further growth still when motion is disabled", async () => {
  viewportHeight = 60;
  measuredHeight = 48;
  const blocks = [{ ...confirmed, source: null }];
  const timeline = await render(blocks, 18, true);
  const inner = timeline.querySelector<HTMLElement>("[aria-label]:not([role])")!.firstElementChild as HTMLElement;
  measuredHeight = 72;
  await act(async () => { resize.forEach(callback => callback()); });
  expect(inner.style.transition).toBe("transform 180ms ease-out");
  const transforms = vi.spyOn(inner.style, "transform", "set");
  await render(blocks, 18, false);
  expect(inner.style.transition).toBe("none");
  measuredHeight = 96;
  await act(async () => { resize.forEach(callback => callback()); });
  expect(transforms.mock.calls.map(([value]) => value)).toEqual(["translateY(0)", "translateY(0)"]);
  transforms.mockRestore();
});

it("shares the visible space between both long live sources with compact source metadata", async () => {
  viewportHeight = 174;
  measuredHeight = 240;
  const tracks = (["system", "microphone"] as const).map(audioSource => ({
    ...live, id: audioSource, audioSource, source: confirmed.source, translation: confirmed.translation,
  }));
  const timeline = await render(tracks, 18);
  const rows = [...timeline.querySelectorAll<HTMLElement>("[data-utterance-id]")];
  const heights = rows.map(row => [...row.querySelectorAll<HTMLElement>("[aria-label]:not([role])")]
    .reduce((sum, lane) => sum + Number.parseFloat(lane.style.height), 0));
  expect(heights.reduce((sum, height) => sum + height + 7 + 22, 0)).toBeLessThanOrEqual(viewportHeight);
  for (const row of rows) expect(row.querySelector<HTMLElement>(".subtitle-audio-source")?.style.fontSize).toBe("14px");
});

function renderedBlockHeight(row: HTMLElement): number {
  const lanes = [...row.querySelectorAll<HTMLElement>("[aria-label]:not([role])")];
  const column = lanes[0].parentElement!;
  const laneHeight = lanes.reduce((height, lane) => height + Number.parseFloat(lane.style.height), 0)
    + Math.max(0, lanes.length - 1) * Number.parseFloat(column.style.gap);
  const metadata = row.querySelector<HTMLElement>(".subtitle-metadata");
  const metadataHeight = metadata ? Number.parseFloat(metadata.style.height) + Number.parseFloat(metadata.style.marginBottom) : 0;
  const dividerHeight = row.querySelector(".subtitle-separator") ? 15 : 0;
  return Number.parseFloat(row.style.paddingTop) + Number.parseFloat(row.style.paddingBottom) + laneHeight + metadataHeight + dividerHeight;
}

const dualLive = (["system", "microphone"] as const).map(audioSource => ({
  ...live, id: audioSource, audioSource, source: confirmed.source, translation: confirmed.translation,
}));

it("reproduces the 136px native dual-bilingual clipping and fits both rows after the minimum grows", async () => {
  measuredHeight = 240;
  viewportHeight = 136 - OVERLAY_MAXIMUM_CHROME_HEIGHT;
  const timeline = await render(dualLive, 18);
  const rows = [...timeline.querySelectorAll<HTMLElement>("[data-utterance-id]")];
  const totalHeight = rows.reduce((height, row) => height + renderedBlockHeight(row), 0);
  expect(viewportHeight).toBe(51);
  expect(totalHeight).toBe(92);
  // Bottom following starts below the system original's entire first lane.
  const systemOriginalHeight = Number.parseFloat(rows[0].querySelector<HTMLElement>("[aria-label]:not([role])")!.style.height);
  expect(totalHeight - viewportHeight).toBeGreaterThan(systemOriginalHeight);
  viewportHeight = minimumOverlayHeight({ audioInput: "both", subtitleDisplayMode: "bilingual", targetLanguage: "zh", fontSize: 18 })
    - OVERLAY_MAXIMUM_CHROME_HEIGHT;
  await act(async () => { resize.forEach(callback => callback()); });
  expect(rows.reduce((height, row) => height + renderedBlockHeight(row), 0)).toBeLessThanOrEqual(viewportHeight);
  expect(timeline.querySelectorAll("[data-utterance-id]")).toHaveLength(2);
});

it.each([14, 15, 16, 17, 18, 19, 20])("fits both long bilingual live rows below the active native chrome at font %i", async fontSize => {
  measuredHeight = 240;
  viewportHeight = minimumOverlayHeight({ audioInput: "both", subtitleDisplayMode: "bilingual", targetLanguage: "zh", fontSize })
    - OVERLAY_MAXIMUM_CHROME_HEIGHT;
  const timeline = await render([{ ...confirmed, presentation: "history" }, ...dualLive], fontSize);
  const rows = dualLive.map(block => timeline.querySelector<HTMLElement>(`[data-utterance-id="${block.id}"]`)!);
  expect(rows.reduce((height, row) => height + renderedBlockHeight(row), 0)).toBeLessThanOrEqual(viewportHeight);
  for (const row of rows) {
    expect(row.querySelectorAll("[aria-label]:not([role])")).toHaveLength(2);
    expect(row.querySelector<HTMLElement>(".subtitle-audio-source")?.style.fontSize).toBe("14px");
    const lanes = [...row.querySelectorAll<HTMLElement>("[aria-label]:not([role])")];
    expect(lanes.every(lane => Number.parseFloat(lane.style.height) > 0 && !lane.hidden)).toBe(true);
  }
});

it.each(["original", "translation"] as const)("fits both single-language rows at the derived minimum in %s mode", async subtitleDisplayMode => {
  measuredHeight = 240;
  for (const fontSize of [14, 18, 20]) {
    viewportHeight = minimumOverlayHeight({ audioInput: "both", subtitleDisplayMode, targetLanguage: "zh", fontSize })
      - OVERLAY_MAXIMUM_CHROME_HEIGHT;
    const timeline = await render(dualLive, fontSize, false, false, subtitleDisplayMode);
    await act(async () => { resize.forEach(callback => callback()); });
    const rows = [...timeline.querySelectorAll<HTMLElement>("[data-utterance-id]")];
    expect(rows.reduce((height, row) => height + renderedBlockHeight(row), 0)).toBeLessThanOrEqual(viewportHeight);
    expect(rows.every(row => row.querySelectorAll("[aria-label]:not([role])").length === 1)).toBe(true);
  }
});

it("budgets original-target bilingual rows with a side label instead of adding a nonexistent translation line", async () => {
  measuredHeight = 240;
  viewportHeight = minimumOverlayHeight({ audioInput: "both", subtitleDisplayMode: "bilingual", targetLanguage: "original", fontSize: 20 })
    - OVERLAY_MAXIMUM_CHROME_HEIGHT;
  const timeline = await render(dualLive.map(block => ({ ...block, translation: null })), 20);
  const rows = [...timeline.querySelectorAll<HTMLElement>("[data-utterance-id]")];
  expect(rows.reduce((height, row) => height + renderedBlockHeight(row), 0)).toBeLessThanOrEqual(viewportHeight);
  expect(rows.every(row => row.querySelectorAll("[aria-label]:not([role])").length === 1)).toBe(true);
});

it("keeps retained mixed-source identity beside a single live row at the single-input minimum", async () => {
  measuredHeight = 240;
  viewportHeight = 51;
  const timeline = await render([{ ...live, audioSource: "microphone" }], 20);
  const row = timeline.querySelector<HTMLElement>("[data-utterance-id]")!;
  expect(row.querySelector(".subtitle-audio-source")).not.toBeNull();
  expect(row.querySelector(".subtitle-metadata")).toBeNull();
  expect(renderedBlockHeight(row)).toBeLessThanOrEqual(viewportHeight);
});

it("fits confirmation time and both bilingual lanes at the timestamp minimum", async () => {
  measuredHeight = 240;
  viewportHeight = minimumOverlayHeight({ audioInput: "microphone", fontSize: 20, subtitleDisplayMode: "bilingual", targetLanguage: "zh", showSubtitleTimestamps: true }) - OVERLAY_MAXIMUM_CHROME_HEIGHT;
  await act(async () => root.render(<Timeline blocks={[{ ...confirmed, audioSource: "microphone" }]} fontSize={20} alignment="center" color="white" displayMode="bilingual" audioInput="microphone" showTimestamps />));
  const row = host.querySelector<HTMLElement>("[data-utterance-id]")!;
  expect(row.querySelector(".subtitle-timestamp")).not.toBeNull();
  expect(renderedBlockHeight(row)).toBeLessThanOrEqual(viewportHeight);
});

it.each([136, 240])("restores system rows and keeps old microphone identity inline at height %i despite saved time preference", async height => {
  measuredHeight = 240;
  viewportHeight = height - OVERLAY_MAXIMUM_CHROME_HEIGHT;
  await act(async () => root.render(<Timeline blocks={[
    { ...confirmed, audioSource: "system" },
    { ...confirmed, id: "microphone-history", audioSource: "microphone" },
  ]} fontSize={20} alignment="center" color="white" displayMode="bilingual" audioInput="system" showTimestamps />));
  const rows = [...host.querySelectorAll<HTMLElement>("[data-utterance-id]")];
  expect(host.querySelector('.subtitle-metadata')).toBeNull();
  expect(rows[0].querySelector('.subtitle-audio-source')).toBeNull();
  expect(rows[1].querySelector('.subtitle-audio-source')).not.toBeNull();
  expect(rows.every(row => renderedBlockHeight(row) <= viewportHeight)).toBe(true);
});


it.each([244, 245, 250, 260])("fits dual bilingual live rows with dividers at native height %i", async height => {
  measuredHeight = 240;
  viewportHeight = height - OVERLAY_MAXIMUM_CHROME_HEIGHT;
  const timeline = await render(dualLive, 20, false, true);
  const rows = [...timeline.querySelectorAll<HTMLElement>("[data-utterance-id]")];
  expect(rows.reduce((sum, row) => sum + renderedBlockHeight(row), 0)).toBeLessThanOrEqual(viewportHeight);
  expect(rows[0].querySelector(".subtitle-separator")).not.toBeNull();
});
