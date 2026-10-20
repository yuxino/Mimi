// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PulseRing } from "./PulseRing";
import type { OverlayActivityPhaseKind, PulseStyle } from "../../lib/types";

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  vi.useFakeTimers();
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
});
async function render(phase: OverlayActivityPhaseKind, pulseStyle?: PulseStyle, motionEnabled = true, compact = false, prominent = false, animationVisible = true) {
  await act(async () => root.render(<PulseRing phase={phase} pulseStyle={pulseStyle} motionEnabled={motionEnabled} compact={compact} prominent={prominent} animationVisible={animationVisible} />));
  return host.firstElementChild as HTMLElement;
}

function clocks() {
  return ["syllables", "ribbons", "translation"].map(kind =>
    host.querySelector<SVGGElement>(`.sound-light__${kind}`)!.dataset.clock);
}

function tracks() {
  return [...host.querySelectorAll(".sound-light__beat, .sound-light__wave, .sound-light__current")];
}

describe("selectable pulse styles", () => {
  it("defaults to ribbon without the old breathing light and gives both styles readable bounds", async () => {
    expect((await render("listening")).dataset.pulseStyle).toBe("ribbon");
    expect(host.querySelector(".phase-light__dot")).toBeNull();
    for (const style of ["syllable", "ribbon"] as const) {
      const normal = await render("listening", style);
      expect(normal.dataset.soundStyle).toBe(style);
      expect(normal.style.width).toBe("40px");
      expect((await render("listening", style, true, true)).style.width).toBe("24px");
      expect((await render("listening", style, true, false, true)).style.width).toBe("80px");
      expect((await render("listening", style, true, true, true)).style.width).toBe("48px");
    }
  });

  it.each(["syllable", "ribbon"] as const)("keeps %s tracks mounted when empty-overlay space changes with motion off", async style => {
    const light = await render("listening", style, false, false, true);
    const track = host.querySelector(".sound-light__beat");
    expect(light.dataset.clock).toBe("paused");
    await render("listening", style, false, true, true);
    expect(host.querySelector(".sound-light__beat")).toBe(track);
    expect(light.dataset.size).toBe("prominent-compact");
    expect(light.dataset.clock).toBe("paused");
    expect(light.getAttribute("aria-hidden")).toBe("true");
  });

  it.each(["syllable", "ribbon"] as const)("keeps %s track nodes across phases and pauses/resumes its clock", async style => {
    const light = await render("recognizing", style);
    const track = host.querySelector(".sound-light__beat");
    await render("translating", style);
    expect(host.querySelector(".sound-light__beat")).toBe(track);
    await render("paused", style);
    expect(light.dataset.clock).toBe("running");
    await act(async () => { await vi.advanceTimersByTimeAsync(520); });
    expect(light.dataset.clock).toBe("paused");
    await render("listening", style);
    expect(host.querySelector(".sound-light__beat")).toBe(track);
    expect(light.dataset.clock).toBe("running");
    await render("recognizing", style, false);
    expect(light.dataset.clock).toBe("paused");
    expect(light.classList.contains("sound-light--still")).toBe(true);
  });

  it("runs only the visible style, lets an outgoing phase fade, and preserves all track nodes", async () => {
    await render("listening", "ribbon");
    const originalTracks = tracks();
    expect(originalTracks).toHaveLength(11);
    expect(clocks()).toEqual(["paused", "running", "paused"]);
    expect(vi.getTimerCount()).toBe(0);

    await render("translating", "ribbon");
    expect(clocks()).toEqual(["paused", "running", "running"]);
    await act(async () => { await vi.advanceTimersByTimeAsync(519); });
    expect(clocks()).toEqual(["paused", "running", "running"]);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(clocks()).toEqual(["paused", "paused", "running"]);

    await render("recognizing", "syllable");
    expect(clocks()).toEqual(["running", "paused", "running"]);
    await act(async () => { await vi.advanceTimersByTimeAsync(520); });
    expect(clocks()).toEqual(["running", "paused", "paused"]);
    tracks().forEach((track, index) => expect(track).toBe(originalTracks[index]));
  });

  it("cancels an outgoing style's pause when it becomes visible again before settling", async () => {
    await render("recognizing", "syllable");
    await render("recognizing", "ribbon");
    expect(clocks()).toEqual(["running", "running", "paused"]);
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    await render("recognizing", "syllable");
    await act(async () => { await vi.advanceTimersByTimeAsync(220); });
    expect(clocks()).toEqual(["running", "running", "paused"]);
    await act(async () => { await vi.advanceTimersByTimeAsync(300); });
    expect(clocks()).toEqual(["running", "paused", "paused"]);
  });

  it("freezes hidden measurement copies immediately and resumes their retained tracks", async () => {
    const light = await render("recognizing", "ribbon");
    const originalTracks = tracks();
    await render("translating", "ribbon");
    expect(vi.getTimerCount()).toBe(1);
    await render("translating", "ribbon", true, false, false, false);
    expect(clocks()).toEqual(["paused", "paused", "paused"]);
    expect(light.dataset.clock).toBe("paused");
    expect(light.classList.contains("sound-light--still")).toBe(false);
    expect(vi.getTimerCount()).toBe(0);

    await render("translating", "ribbon");
    expect(clocks()).toEqual(["paused", "paused", "running"]);
    tracks().forEach((track, index) => expect(track).toBe(originalTracks[index]));
  });

  it("clears outgoing-track timers on unmount", async () => {
    await render("recognizing", "syllable");
    await render("translating", "syllable");
    await render("paused", "syllable");
    expect(vi.getTimerCount()).toBe(2);
    await act(async () => root.unmount());
    expect(vi.getTimerCount()).toBe(0);
  });
});
