// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Timeline } from "./Timeline";
import { Select } from "../../components/Select";
import { buildSubtitleBlocks, type SubtitleBlock } from "./overlayModel";

const originalScrollTo = HTMLElement.prototype.scrollTo;
let root: Root, host: HTMLDivElement;
let timeline: HTMLDivElement;
let scrollTop = 0, scrollHeight = 200;
let observers: Array<{ targets: Set<Element>; resized: () => void }>;
function resizeRow(element: Element) { for (const observer of observers) if (observer.targets.has(element)) observer.resized(); }
const scrollTo = vi.fn((options: ScrollToOptions) => { scrollTop = Math.min(options.top ?? 0, scrollHeight - 80); });
const blocks: SubtitleBlock[] = [
  { id: "old", createdAt: 1, presentation: "history", source: "Older source", translation: "较早译文" },
  { id: "latest", createdAt: 2, presentation: "latestCommitted", source: "Current source", translation: "当前译文" },
];
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  observers = [];
  vi.stubGlobal("ResizeObserver", class {
    private targets = new Set<Element>();
    constructor(resized: () => void) { observers.push({ targets: this.targets, resized }); }
    observe(element: Element) { this.targets.add(element); }
    disconnect() { this.targets.clear(); }
  });
  scrollTop = 0; scrollHeight = 200; scrollTo.mockClear();
  HTMLElement.prototype.scrollTo = vi.fn();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); HTMLElement.prototype.scrollTo = originalScrollTo; vi.unstubAllGlobals(); });
async function render(mode: "original" | "translation" | "bilingual", list = blocks, followTailRequest = 0, onReadingHistoryChange?: (reading: boolean) => void) {
  await act(async () => root.render(<Timeline blocks={list.map(b => mode === "translation" ? { ...b, source: null } : mode === "original" ? { ...b, translation: null } : b)} displayMode={mode} fontSize={18} alignment="center" color="white" motionEnabled={true}
    followTailRequest={followTailRequest} onReadingHistoryChange={onReadingHistoryChange} />));
}
async function mount(list = blocks) {
  await render("translation", list); timeline = host.firstElementChild as HTMLDivElement;
  Object.defineProperties(timeline, { scrollHeight: { get: () => scrollHeight }, clientHeight: { get: () => 80 }, scrollTop: { get: () => scrollTop, set: (top: number) => { scrollTop = Math.min(top, scrollHeight - 80); } } });
  timeline.scrollTo = scrollTo as HTMLElement["scrollTo"];
  timeline.getBoundingClientRect = () => ({ top: 0, bottom: 80 } as DOMRect);
  for (const child of Array.from(timeline.children)) {
    const index = Array.from(timeline.children).indexOf(child);
    child.getBoundingClientRect = () => ({ top: (index === 0 ? 0 : 220) - scrollTop, bottom: (index === 0 ? 220 : scrollHeight) - scrollTop } as DOMRect);
  }
  // Equivalent to the existing resize pin after the initial native layout.
  scrollTop = 120;
}

it("starts a new sentence glide before measuring rows and preserves it through delayed layout", async () => {
  await mount();
  const movements: Array<{ before: number; after: number; behavior?: ScrollBehavior }> = [];
  timeline.scrollTo = ((options: ScrollToOptions) => {
    const before = scrollTop;
    // A real smooth scroll does not synchronously reach its destination.
    if (options.behavior !== "smooth") scrollTop = Math.min(options.top ?? 0, scrollHeight - 80);
    movements.push({ before, after: scrollTop, behavior: options.behavior });
  }) as HTMLElement["scrollTo"];
  const next: SubtitleBlock = { id: "new-live", createdAt: null, presentation: "live", source: null, translation: "A new sentence" };
  scrollHeight = 320;
  await render("translation", [...blocks, next]);
  expect(movements[0]).toEqual({ before: 120, after: 120, behavior: "smooth" });
  expect(movements.every(move => move.behavior === "smooth")).toBe(true);

  scrollHeight = 350;
  await act(async () => resizeRow(timeline.lastElementChild!));
  expect(scrollTop).toBe(120);
  expect(movements.at(-1)?.behavior).toBe("smooth");

  // The next streaming chunk must retarget the in-flight sentence glide too.
  scrollHeight = 370;
  await render("translation", [...blocks, { ...next, translation: "A new sentence continues streaming" }]);
  expect(scrollTop).toBe(120);
  expect(movements.at(-1)?.behavior).toBe("smooth");

  // Once the glide reaches the tail, later viewport changes pin immediately.
  await act(async () => { scrollTop = 290; timeline.dispatchEvent(new Event("scroll", { bubbles: true })); });
  scrollHeight = 380;
  await act(async () => resizeRow(timeline.lastElementChild!));
  expect(scrollTop).toBe(300);
  expect(movements.at(-1)?.behavior).toBe("instant");
});

it("keeps a sibling embedded picker open through a new caption's automatic scroll", async () => {
  const choose = vi.fn();
  const choices = [{ value: "en", label: "English" }, { value: "zh", label: "Chinese" }];
  const show = async (list: SubtitleBlock[]) => act(async () => root.render(<>
    <Timeline blocks={list.map(block => ({ ...block, source: null }))} displayMode="translation" fontSize={18} alignment="center" color="white" />
    <div className="embedded-panel"><Select label="Language" value="en" options={choices} onChange={choose} /></div>
  </>));
  await show(blocks);
  timeline = host.querySelector<HTMLDivElement>(".overlay-timeline")!;
  Object.defineProperties(timeline, {
    scrollHeight: { get: () => scrollHeight }, clientHeight: { get: () => 80 },
    scrollTop: { get: () => scrollTop, set: (top: number) => { scrollTop = top; } },
  });
  scrollTop = 120;
  const scrolled = vi.fn();
  timeline.addEventListener("scroll", scrolled);
  timeline.scrollTo = ((options: ScrollToOptions) => {
    const next = Math.max(0, Math.min(options.top ?? 0, scrollHeight - 80));
    if (next === scrollTop) return;
    scrollTop = next;
    // jsdom has no layout scrolling. Deliver only the browser's asynchronous
    // non-bubbling event after the actual Timeline requests its new position.
    const target = timeline;
    queueMicrotask(() => target.dispatchEvent(new Event("scroll")));
  }) as HTMLElement["scrollTo"];
  const trigger = host.querySelector<HTMLButtonElement>('[role="combobox"]')!;
  await act(() => trigger.click());
  const popup = document.querySelector<HTMLElement>('[role="listbox"]')!;
  expect(popup).not.toBeNull();
  expect(timeline.contains(trigger)).toBe(false);
  expect(timeline.contains(popup)).toBe(false);
  scrollHeight = 320;
  await show([...blocks, { id: "new-live", createdAt: null, presentation: "live", source: null, translation: "Synthetic next caption" }]);
  expect(scrolled).toHaveBeenCalledOnce();
  expect((scrolled.mock.calls[0][0] as Event).target).toBe(timeline);
  expect(document.querySelector('[role="listbox"]')).toBe(popup);
  expect(trigger.getAttribute("aria-expanded")).toBe("true");
  expect(document.activeElement).toBe(trigger);
  await act(() => trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })));
  await act(() => trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
  expect(choose).toHaveBeenCalledExactlyOnceWith("zh");
  expect(document.querySelector('[role="listbox"]')).toBeNull();
});
it("switches to bilingual at the latest sentence's start rather than hiding its original at the old bottom", async () => {
  await mount(); scrollHeight = 320;
  await render("bilingual");
  expect(scrollTop).toBe(220); // source and actual translated line fit; unused compact budget extends below
});

it.each(["original", "translation", "bilingual"] as const)("reaches the beginning on the first Home after compact %s rows expand", async mode => {
  await mount();
  const reading = vi.fn();
  await render(mode, blocks, 0, reading);
  const home = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Home" });
  await act(async () => timeline.dispatchEvent(home));
  expect(scrollTop).toBe(0);
  expect(home.defaultPrevented).toBe(true);
  expect(reading).toHaveBeenLastCalledWith(true);
  expect(timeline.querySelector("[aria-label]")).toBeNull();

  // Full rows finish measuring after the key event. The old compact-row
  // anchor must not override explicit Home during this delayed layout.
  scrollHeight = 800;
  await act(async () => {
    resizeRow(timeline.firstElementChild!);
    timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  expect(scrollTop).toBe(0);
  expect(reading).toHaveBeenLastCalledWith(true);

  // Ordinary gestures release Home's start intent and keep their own anchor.
  await act(async () => {
    timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 45 }));
    scrollTop = 45; timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
    resizeRow(timeline.firstElementChild!);
  });
  expect(scrollTop).toBe(45);
  await act(async () => timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Home" })));
  expect(scrollTop).toBe(0); // Already-open reading needs no second React transition.
  await act(async () => timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "End" })));
  expect(scrollTop).toBe(720);
  expect(reading).toHaveBeenLastCalledWith(false);
});

it("keeps a live reader's position through equal-length rewraps, growth, modes and confirmation without following the tail", async () => {
  const live: SubtitleBlock = { id: "live", createdAt: null, presentation: "live", streaming: true,
    source: "Long synthetic original. ".repeat(20), translation: "Long synthetic translation. ".repeat(20) };
  await mount([live]);
  const reading = vi.fn();
  await render("translation", [live], 0, reading);
  const row = timeline.firstElementChild!;
  row.getBoundingClientRect = () => ({ top: -scrollTop, bottom: scrollHeight - scrollTop } as DOMRect);
  scrollTop = 30;
  await act(async () => timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));
  expect(reading).toHaveBeenLastCalledWith(true);
  expect(row.querySelector("[aria-label]")).toBeNull();

  scrollTo.mockClear(); scrollHeight = 350;
  const corrected = { ...live, translation: live.translation!.replaceAll("Long", "Wide") };
  expect(corrected.translation.length).toBe(live.translation!.length);
  await render("translation", [corrected], 0, reading);
  await act(async () => resizeRow(row));
  expect(scrollTop).toBe(30); // A same-length rewrap restores the read anchor instead of following the tail.
  expect(row.textContent).toContain(corrected.translation);

  const growing = { ...corrected, source: `${live.source}New raw recognition.`, translation: `${corrected.translation}New complete translation.` };
  scrollHeight = 400;
  for (const mode of ["bilingual", "original", "translation"] as const) {
    await render(mode, [growing], 0, reading);
    expect(timeline.firstElementChild).toBe(row);
    expect(scrollTop).toBe(30);
    expect(reading).toHaveBeenLastCalledWith(true);
    expect(row.querySelector("[aria-label]")).toBeNull();
  }

  scrollTo.mockClear();
  await render("translation", [{ ...growing, id: "history-2", createdAt: 2, presentation: "latestCommitted", streaming: undefined }], 0, reading);
  expect(scrollTop).toBe(30);
  expect(scrollTo).not.toHaveBeenCalled();
  expect(reading).toHaveBeenLastCalledWith(true);
  expect(timeline.querySelector("[aria-label]")).toBeNull();
  await render("translation", [{ ...growing, id: "history-2", createdAt: 2, presentation: "latestCommitted", streaming: undefined }], 1, reading);
  expect(scrollTop).toBe(320);
  expect(reading).toHaveBeenLastCalledWith(false);
  expect(timeline.querySelector("[aria-label]")).not.toBeNull();
});

it.each(["original", "translation", "bilingual"] as const)("does not leave full live %s reading when confirmation clamps geometry and later emits a native scroll", async mode => {
  const live: SubtitleBlock = { id: "live", createdAt: null, presentation: "live", streaming: true,
    source: "Synthetic original before confirmation. ".repeat(20), translation: "Synthetic translation before confirmation. ".repeat(20) };
  await mount([live]);
  const reading = vi.fn();
  await render(mode, [live], 0, reading);
  scrollTop = 30;
  await act(async () => timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));
  expect(reading).toHaveBeenLastCalledWith(true);
  const final = { ...live, id: "confirmed-2", createdAt: 2, presentation: "latestCommitted" as const, streaming: undefined };
  await render(mode, [final], 0, reading);
  const row = timeline.firstElementChild!;
  row.getBoundingClientRect = () => ({ top: -scrollTop, bottom: scrollHeight - scrollTop } as DOMRect);
  // Emulate native final-layout shrink followed by a delayed scroll event.
  // The old live ID no longer exists and the current 30px is now the bottom.
  scrollHeight = 110;
  await act(async () => { resizeRow(row); scrollTop = 30; timeline.dispatchEvent(new Event("scroll", { bubbles: true })); });
  expect(reading).toHaveBeenLastCalledWith(true);
  expect(row.querySelector("[aria-label]")).toBeNull();
  scrollHeight = 400;
  await act(async () => { resizeRow(row); timeline.dispatchEvent(new Event("scroll", { bubbles: true })); });
  expect(scrollTop).toBe(30);
  expect(reading).toHaveBeenLastCalledWith(true);

  // A fresh deliberate downward gesture can still resume compact following.
  await act(async () => {
    timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: 300 }));
    scrollTop = 320; timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  expect(reading).toHaveBeenLastCalledWith(false);
  expect(row.querySelector("[aria-label]")).not.toBeNull();
});

it("renews real scrollbar-drag input on each pointer movement and does not treat a stationary click as return intent", async () => {
  await mount();
  const reading = vi.fn();
  await render("translation", blocks, 0, reading);
  await act(async () => timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));
  await act(async () => {
    timeline.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  expect(reading).toHaveBeenLastCalledWith(true); // Reading at the tail alone is not permission to leave it.
  scrollTop = 30;
  const pointerMove = () => {
    const event = new Event("pointermove", { bubbles: true });
    Object.defineProperty(event, "buttons", { value: 1 });
    timeline.dispatchEvent(event);
  };
  await act(async () => { pointerMove(); scrollTop = 50; timeline.dispatchEvent(new Event("scroll", { bubbles: true })); });
  expect(reading).toHaveBeenLastCalledWith(true);
  await act(async () => { pointerMove(); scrollTop = 120; timeline.dispatchEvent(new Event("scroll", { bubbles: true })); });
  expect(reading).toHaveBeenLastCalledWith(false);
});

it.each(["original", "translation", "bilingual"] as const)("retains a %s reader's viewport when the live row confirms and a new live row arrives together", async mode => {
  const previous = { source: "Older confirmed original.", translation: "较早确认译文。", createdAt: 1 };
  const previewA = { source: "Synthetic draft original A. ".repeat(20), translation: "合成预览译文 A。".repeat(20), isStreaming: true };
  const before = buildSubtitleBlocks([previous], mode, previewA);
  await mount(before);
  let promoted = false;
  scrollHeight = 700;
  const oldLive = timeline.lastElementChild!;
  timeline.firstElementChild!.getBoundingClientRect = () => ({ top: -scrollTop, bottom: 100 - scrollTop } as DOMRect);
  oldLive.getBoundingClientRect = () => ({ top: (promoted ? 700 : 100) - scrollTop, bottom: (promoted ? 800 : 700) - scrollTop } as DOMRect);
  const reading = vi.fn();
  await render(mode, before, 0, reading);
  scrollTop = 130;
  await act(async () => timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));
  expect(reading).toHaveBeenLastCalledWith(true);

  // Final wording can be corrected: the presentation must not guess identity
  // from a matching source/translation string or move to whichever live comes next.
  const finalA = { source: "Corrected confirmed original A. ".repeat(20), translation: "修正后的确认译文 A。".repeat(20), createdAt: 2 };
  const previewB = { source: "New original B.", translation: "新的译文 B。", isStreaming: true };
  const after = buildSubtitleBlocks([previous, finalA], mode, previewB);
  promoted = true; scrollHeight = 800;
  await render(mode, after, 0, reading);
  expect(scrollTop).toBe(130);
  expect(reading).toHaveBeenLastCalledWith(true);
  expect(timeline.lastElementChild).not.toBe(oldLive);
  let trimmed = false;
  const finalRow = timeline.querySelector<HTMLElement>('[data-utterance-id="history-2"]')!;
  finalRow.getBoundingClientRect = () => ({ top: (trimmed ? 0 : 100) - scrollTop, bottom: (trimmed ? 600 : 700) - scrollTop } as DOMRect);
  const newLive = timeline.lastElementChild!;
  newLive.getBoundingClientRect = () => ({ top: (trimmed ? 600 : 700) - scrollTop, bottom: (trimmed ? 700 : 800) - scrollTop } as DOMRect);
  await act(async () => { resizeRow(finalRow); timeline.dispatchEvent(new Event("scroll", { bubbles: true })); });
  expect(scrollTop).toBe(130);
  expect(finalRow.classList.contains("subtitle-block")).toBe(false);
  expect(finalRow.querySelector("[aria-label]")).toBeNull();
  expect(finalRow.textContent).toContain(mode === "translation" ? finalA.translation : finalA.source);
  // Removing older bounded history does not create another live epoch. The
  // captured confirmed-row anchor follows that same row to its new position.
  trimmed = true; scrollHeight = 700;
  const afterTrim = buildSubtitleBlocks([finalA], mode, previewB);
  await render(mode, afterTrim, 0, reading);
  expect(scrollTop).toBe(30);
  expect(timeline.lastElementChild).toBe(newLive);
  expect(reading).toHaveBeenLastCalledWith(true);
  await render(mode, afterTrim, 1, reading);
  expect(reading).toHaveBeenLastCalledWith(false);
  expect(scrollTop).toBe(620);
});

it.each(["original", "translation", "bilingual"] as const)("keeps reading the unchanged live %s sentence when an earlier final inserts above it", async mode => {
  const previous = { source: "Older synthetic original.", translation: "较早合成译文。", createdAt: 1 };
  const previewB = { source: "Current synthetic original B. ".repeat(20), translation: "当前合成译文 B。".repeat(20), isStreaming: true,
    utteranceId: "synthetic-epoch:1:B" };
  const before = buildSubtitleBlocks([previous], mode, previewB);
  const earlierA = { source: "Earlier synthetic original A.", translation: "较早合成译文 A。", createdAt: 2 };
  const after = buildSubtitleBlocks([previous, earlierA], mode, previewB);
  await mount(before);
  const liveRow = timeline.lastElementChild!;
  let insertedA = false;
  scrollHeight = 700;
  timeline.firstElementChild!.getBoundingClientRect = () => ({ top: -scrollTop, bottom: 100 - scrollTop } as DOMRect);
  liveRow.getBoundingClientRect = () => ({ top: (insertedA ? 400 : 100) - scrollTop,
    bottom: (insertedA ? 1000 : 700) - scrollTop } as DOMRect);
  const reading = vi.fn();
  await render(mode, before, 0, reading);
  scrollTop = 130;
  await act(async () => timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 })));

  // Supply the new final/live row geometry before their layout effects. B's
  // source and translation do not change; only the earlier A is inserted.
  const originalRect = HTMLElement.prototype.getBoundingClientRect;
  HTMLElement.prototype.getBoundingClientRect = function () {
    if (this.dataset.utteranceId === "history-2") return { top: 100 - scrollTop, bottom: 400 - scrollTop } as DOMRect;
    if (this.dataset.utteranceId?.startsWith("live")) return { top: 400 - scrollTop, bottom: 1000 - scrollTop } as DOMRect;
    return originalRect.call(this);
  };
  try {
    insertedA = true; scrollHeight = 1000;
    await render(mode, after, 0, reading);
    expect(scrollTop).toBe(430); // Still 30px into B, rather than 30px into A.
    expect(timeline.lastElementChild).toBe(liveRow);
    expect(reading).toHaveBeenLastCalledWith(true);
  } finally {
    HTMLElement.prototype.getBoundingClientRect = originalRect;
  }
});

it("keeps return-to-live intent through later compact row growth, shrink and replacement while preserving a reader on resize", async () => {
  await mount();
  await act(async () => {
    timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 }));
    scrollTop = 30; timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  scrollHeight = 320;
  await render("bilingual", blocks, 1);
  expect(scrollTop).toBe(240);

  // Compact lanes initially use their budget. Actual inner measurement can
  // then transfer height between languages without changing the viewport or text.
  scrollHeight = 390;
  await act(async () => resizeRow(timeline.lastElementChild!));
  expect(scrollTop).toBe(310);
  scrollHeight = 260;
  await act(async () => resizeRow(timeline.firstElementChild!));
  expect(scrollTop).toBe(180);

  const replacement = { ...blocks[1], id: "final-next" };
  await render("bilingual", [blocks[0], replacement], 1);
  scrollHeight = 350;
  await act(async () => resizeRow(timeline.lastElementChild!));
  expect(scrollTop).toBe(270);

  await act(async () => {
    timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -30 }));
    scrollTop = 30; timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  scrollHeight = 440;
  await act(async () => resizeRow(timeline.firstElementChild!));
  expect(scrollTop).toBe(30); // Content resize restores the same reading anchor, not the tail.
});
it("continues following a growing live tail after a display change", async () => {
  await mount(); scrollHeight = 320; await render("bilingual");
  scrollHeight = 350; await render("bilingual", [blocks[0], { ...blocks[1], translation: "当前译文继续流入" }]);
  expect(scrollTop).toBe(270);
});
it("does not pull a user reading history to the tail on a mode change or incoming text", async () => {
  await mount();
  await act(async () => { timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -100 })); scrollTop = 30; timeline.dispatchEvent(new Event("scroll", { bubbles: true })); });
  scrollHeight = 320; await render("bilingual"); expect(scrollTop).toBe(30);
  scrollHeight = 350; await render("bilingual", [blocks[0], { ...blocks[1], translation: "新译文" }]); expect(scrollTop).toBe(30);
});

it("returns to the live tail only on a new explicit request and reports subsequent reading intent", async () => {
  await mount();
  const reading = vi.fn();
  await render("translation", blocks, 0, reading);
  expect(reading).toHaveBeenLastCalledWith(false);
  await act(async () => {
    timeline.dispatchEvent(new WheelEvent("wheel", { bubbles: true, deltaY: -100 }));
    scrollTop = 30; timeline.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  expect(reading).toHaveBeenLastCalledWith(true);
  scrollHeight = 320;
  await render("bilingual", blocks, 0, reading);
  expect(scrollTop).toBe(30);
  expect(reading).toHaveBeenLastCalledWith(true);

  await render("bilingual", blocks, 1, reading);
  expect(scrollTop).toBe(240);
  expect(reading).toHaveBeenLastCalledWith(false);
  expect(timeline.querySelector("[aria-label]")).not.toBeNull();
  scrollHeight = 350;
  await render("bilingual", [blocks[0], { ...blocks[1], translation: "新的译文继续流入" }], 1, reading);
  expect(scrollTop).toBe(270);

  await act(async () => timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Home" })));
  expect(reading).toHaveBeenLastCalledWith(true);
  await render("bilingual", blocks, 1, reading);
  expect(reading).toHaveBeenLastCalledWith(true); // Re-rendering the request is not another command.
});
