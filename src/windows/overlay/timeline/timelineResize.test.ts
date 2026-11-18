import { afterEach, expect, it, vi } from "vitest";
import { observeTimelineResize } from "./timelineResize";

afterEach(() => vi.unstubAllGlobals());

it("repins unchanged subtitles after a narrower viewport rewraps them and disconnects on unmount", () => {
  let resize = () => {};
  const observe = vi.fn();
  const disconnect = vi.fn();
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { resize = callback; }
    observe = observe;
    disconnect = disconnect;
  });
  const scrollTo = vi.fn();
  const row = {} as Element;
  const element = { scrollHeight: 180, scrollTo, children: [row] } as unknown as HTMLElement;
  const cleanup = observeTimelineResize(element);
  expect(observe).toHaveBeenCalledWith(element);
  expect(observe).toHaveBeenCalledWith(row);
  Object.defineProperty(element, "scrollHeight", { value: 340 });
  resize();
  expect(scrollTo).toHaveBeenLastCalledWith({ top: 340, behavior: "instant" });
  cleanup();
  expect(disconnect).toHaveBeenCalledOnce();
});
