// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Timeline } from "../Timeline";
import type { SubtitleBlock } from "../overlayModel";

let root: Root;
let host: HTMLDivElement;
const originalScrollTo = HTMLElement.prototype.scrollTo;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  HTMLElement.prototype.scrollTo = vi.fn();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  HTMLElement.prototype.scrollTo = originalScrollTo;
  vi.unstubAllGlobals();
});

const repeated: SubtitleBlock = {
  id: "synthetic-1", createdAt: null, presentation: "live",
  source: "테스트，".repeat(40), translation: "合成词，".repeat(40),
};

async function render(blocks: SubtitleBlock[], displayMode: "original" | "translation" | "bilingual") {
  await act(async () => root.render(<Timeline blocks={blocks} displayMode={displayMode}
    fontSize={18} alignment="center" color="white" motionEnabled={false} />));
  return host.firstElementChild as HTMLDivElement;
}

it.each(["original", "translation", "bilingual"] as const)("folds extreme repetitions in compact %s lanes while retaining their full accessible text", async mode => {
  for (const presentation of ["live", "latestCommitted", "history"] as const) {
    await render([{ ...repeated, presentation }], mode);
    const lanes = Array.from(host.querySelectorAll<HTMLElement>("[aria-label]"));
    expect(lanes).toHaveLength(mode === "bilingual" ? 2 : 1);
    for (const lane of lanes) {
      const original = lane.getAttribute("aria-label")!;
      expect([repeated.source, repeated.translation]).toContain(original);
      expect(lane.textContent).toContain("×40");
      expect(lane.textContent!.length).toBeLessThan(original.length / 4);
    }
  }
});

it("opens the full unmodified live sentence with Home and folds it again with End", async () => {
  const timeline = await render([repeated], "bilingual");
  const row = timeline.firstElementChild;
  await act(async () => timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Home" })));
  expect(timeline.textContent).toBe(repeated.source! + repeated.translation!);
  expect(timeline.firstElementChild).toBe(row);
  expect(timeline.querySelector("[aria-label]")).toBeNull();
  await act(async () => timeline.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "End" })));
  expect(timeline.textContent).toContain("×40");
  expect(timeline.firstElementChild).toBe(row);
  expect(timeline.querySelector("[aria-label]")?.getAttribute("aria-label")).toBe(repeated.source);
});

it("keeps two distinct repeated utterances and restores ordinary corrected text without mutating input", async () => {
  const original = JSON.stringify(repeated);
  await render([{ ...repeated, id: "synthetic-older", presentation: "history" }, repeated], "translation");
  expect(host.querySelectorAll("[data-utterance-id]")).toHaveLength(2);
  expect(host.textContent?.match(/×40/g)).toHaveLength(2);
  const corrected = { ...repeated, translation: "A different complete sentence." };
  await render([corrected], "translation");
  expect(host.textContent).toBe(corrected.translation);
  expect(JSON.stringify(repeated)).toBe(original);
});
