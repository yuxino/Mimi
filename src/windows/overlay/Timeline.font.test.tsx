// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Timeline } from "./Timeline";
import type { SubtitleBlock } from "./overlayModel";

const blocks: SubtitleBlock[] = [
  { id: "system", audioSource: "system", createdAt: 1, presentation: "history", source: "Original", translation: "译文" },
  { id: "mic", audioSource: "microphone", createdAt: 2, presentation: "latestCommitted", source: "Microphone original", translation: "麦克风译文" },
];
const texts = blocks.flatMap(block => [block.source!, block.translation!]);
const originalScrollTo = HTMLElement.prototype.scrollTo;
let root: Root;
let host: HTMLDivElement;

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

async function render(fontFamily: string | undefined, immersive: boolean) {
  await act(async () => root.render(<Timeline blocks={blocks} fontFamily={fontFamily}
    fontSize={18} alignment="center" color="white" displayMode="bilingual"
    showTimestamps audioInput="both" blendsWithBackground={immersive} />));
}

function expectSubtitleFont(expected: string) {
  const lanes = Array.from(host.querySelectorAll("span"))
    .filter(span => texts.includes(span.textContent ?? ""));
  expect(lanes).toHaveLength(4);
  for (const lane of lanes) expect(lane.style.fontFamily).toBe(expected);
  for (const timestamp of host.querySelectorAll<HTMLTimeElement>(".subtitle-timestamp")) {
    expect(timestamp.style.fontFamily).toBe("var(--mimi-ui-font)");
  }
  expect(host.querySelector<HTMLElement>(".overlay-timeline")!.style.fontFamily).toBe("");
}

it.each([false, true])("updates both sources and both languages in compact and full reading lanes (immersive=%s)", async immersive => {
  await render("Noto Serif CJK SC", immersive);
  expectSubtitleFont('"Noto Serif CJK SC", var(--mimi-ui-font)');

  // Deliberate history reading replaces compact lanes with full text spans.
  await act(async () => host.firstElementChild!.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Home", bubbles: true }),
  ));
  expect(host.querySelector('.overlay-timeline [aria-label="Original"]')).toBeNull();
  expectSubtitleFont('"Noto Serif CJK SC", var(--mimi-ui-font)');

  await render("Noto Sans CJK SC", immersive);
  expectSubtitleFont('"Noto Sans CJK SC", var(--mimi-ui-font)');

  await render("", immersive);
  expectSubtitleFont("var(--mimi-ui-font)");
});

it.each([false, true])("keeps old snapshots readable when no font preference exists (immersive=%s)", async immersive => {
  await render(undefined, immersive);
  expectSubtitleFont("var(--mimi-ui-font)");
});
