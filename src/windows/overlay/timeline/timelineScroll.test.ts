import { expect, it, vi } from "vitest";
import { TimelineScroll } from "./timelineScroll";

function fixture() {
  let top = 200;
  const older = { dataset: { utteranceId: "old" }, getBoundingClientRect: () => ({ top: -top, bottom: 160 - top }) };
  const latest = { dataset: { utteranceId: "last" }, getBoundingClientRect: () => ({ top: 160 - top, bottom: 300 - top }) };
  const scrollTo = vi.fn((options: ScrollToOptions) => { top = Math.max(0, Math.min(options.top ?? 0, 200)); });
  const element = { get scrollTop() { return top; }, set scrollTop(t: number) { top = t; }, scrollHeight: 300, clientHeight: 100, getBoundingClientRect: () => ({ top: 0 }), children: [older, latest], lastElementChild: latest, scrollTo } as unknown as HTMLElement;
  return { element, scrollTo, scroll: new TimelineScroll() };
}
it("keeps the mode sentence anchor after layout resize, then resumes tail-following for new content", () => {
  const { element, scroll } = fixture();
  scroll.displayChanged(element); expect(element.scrollTop).toBe(160);
  scroll.reflow(element); expect(element.scrollTop).toBe(160);
  scroll.contentChanged(element, "instant"); expect(element.scrollTop).toBe(200);
});
it("keeps a reader on their sentence through reflow and resumes following only after returning to the tail", () => {
  const { element, scroll, scrollTo } = fixture();
  scroll.userIntent(element); element.scrollTop = 30; scroll.scrolled(element);
  scroll.reflow(element); expect(element.scrollTop).toBe(30);
  scroll.contentChanged(element, "smooth"); expect(element.scrollTop).toBe(30);
  scroll.userIntent(element); element.scrollTop = 200; scroll.scrolled(element);
  scroll.contentChanged(element, "smooth"); expect(scrollTo).toHaveBeenLastCalledWith({ top: 200, behavior: "smooth" });
});
it("ignores programmatic scroll events while revealing the bilingual original", () => {
  const { element, scroll } = fixture();
  scroll.displayChanged(element); scroll.scrolled(element);
  scroll.contentChanged(element, "instant"); expect(element.scrollTop).toBe(200);
});

it("does not stop following merely because someone clicks or selects text without scrolling", () => {
  const { element, scroll, scrollTo } = fixture();
  scroll.userIntent(element);
  scroll.contentChanged(element, "smooth");
  expect(scrollTo).toHaveBeenLastCalledWith({ top: 200, behavior: "smooth" });
});

it("can start reading before compact content overflows and explicitly return to the tail", () => {
  const { element, scroll, scrollTo } = fixture();
  scroll.beginReading(element);
  expect(scroll.isFollowing()).toBe(false);
  scroll.contentChanged(element, "smooth");
  expect(element.scrollTop).toBe(200);
  scroll.followTail(element);
  expect(scroll.isFollowing()).toBe(true);
  expect(scrollTo).toHaveBeenLastCalledWith({ top: 200, behavior: "instant" });
});

it("keeps explicit Home at the start through expansion, then restores ordinary reading anchors after new input", () => {
  const { element, scroll } = fixture();
  scroll.readFromStart(element);
  expect(element.scrollTop).toBe(0);
  expect(scroll.isFollowing()).toBe(false);
  Object.defineProperty(element, "scrollHeight", { value: 800, configurable: true });
  scroll.reflow(element);
  scroll.scrolled(element);
  scroll.contentChanged(element, "instant");
  expect(element.scrollTop).toBe(0);

  scroll.userIntent(element, "down"); element.scrollTop = 45; scroll.scrolled(element);
  scroll.reflow(element);
  expect(element.scrollTop).toBe(45);
  expect(scroll.isFollowing()).toBe(false);
});

it("reveals the same read sentence when removing a lane makes its previous offset exceed the new height", () => {
  const { element, scroll } = fixture();
  scroll.userIntent(element); element.scrollTop = 30; scroll.scrolled(element);
  const older = element.children[0] as HTMLElement;
  older.getBoundingClientRect = () => ({ top: -element.scrollTop, bottom: 20 - element.scrollTop } as DOMRect);
  scroll.displayChanged(element);
  expect(element.scrollTop).toBe(0);
});

it.each([false, true])("keeps reading after a delayed geometry scroll at the tail with evicted anchor=%s", evicted => {
  const { element, scroll } = fixture();
  element.scrollTop = 30;
  scroll.beginReading(element);
  if (evicted) (element.children[0] as HTMLElement).dataset.utteranceId = "new-final";
  Object.defineProperty(element, "scrollHeight", { value: 130, configurable: true });
  scroll.reflow(element);
  // Native layout can clamp the full row to its new bottom and dispatch a
  // scroll after reflow. No new downward input authorized leaving reading.
  element.scrollTop = 30;
  scroll.scrolled(element);
  expect(scroll.isFollowing()).toBe(false);
});

it("consumes a scroll input instead of authorizing a later unrelated native scroll", () => {
  const { element, scroll } = fixture();
  element.scrollTop = 30;
  scroll.beginReading(element);
  scroll.userIntent(element); element.scrollTop = 50; scroll.scrolled(element);
  expect(scroll.isFollowing()).toBe(false);
  element.scrollTop = 200; scroll.scrolled(element);
  expect(scroll.isFollowing()).toBe(false);
});

it("does not grant an upward gesture or a stationary pointer permission to resume at the tail", () => {
  const { element, scroll } = fixture();
  scroll.beginReading(element); scroll.scrolled(element);
  expect(scroll.isFollowing()).toBe(false);
  scroll.userIntent(element); scroll.scrolled(element);
  expect(scroll.isFollowing()).toBe(false);
  scroll.userIntent(element, "down"); scroll.followIfAtTail(element);
  expect(scroll.isFollowing()).toBe(true);
});

it("stops an in-flight follow glide immediately when the motion preference is disabled", () => {
  const { element, scroll, scrollTo } = fixture();
  element.scrollTop = 30;
  scrollTo.mockImplementation(() => {});
  scroll.contentChanged(element, "smooth");
  scroll.contentChanged(element, "instant", false);
  expect(scrollTo).toHaveBeenLastCalledWith({ top: 200, behavior: "instant" });
  scroll.reflow(element);
  expect(scrollTo).toHaveBeenLastCalledWith({ top: 200, behavior: "instant" });
});

it("resumes a clicked follow glide without mistaking its queued scroll for history reading", () => {
  const { element, scroll, scrollTo } = fixture();
  element.scrollTop = 30;
  scrollTo.mockImplementation(() => {});
  scroll.contentChanged(element, "smooth");
  scroll.pointerDown(element);
  expect(scrollTo).toHaveBeenLastCalledWith({ top: 30, behavior: "instant" });
  scroll.scrolled(element);
  expect(scroll.isFollowing()).toBe(true);
  scroll.pointerUp(element);
  expect(scrollTo).toHaveBeenLastCalledWith({ top: 200, behavior: "smooth" });

  scroll.pointerDown(element);
  scroll.userIntent(element);
  element.scrollTop = 10;
  scroll.scrolled(element);
  expect(scroll.isFollowing()).toBe(false);
  scrollTo.mockClear();
  scroll.pointerUp(element);
  expect(scrollTo).not.toHaveBeenCalled();
});
