import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { Timeline } from "./Timeline";
import type { SubtitleBlock } from "./overlayModel";

const createdAt = new Date(2026, 9, 5, 12, 34, 56).getTime();
const confirmed: SubtitleBlock = { id: "confirmed", createdAt, presentation: "latestCommitted", source: "Ready", translation: "准备好了" };

it.each(["left", "center", "right"] as const)("keeps second-precision confirmation time out of the text gutter with %s alignment", alignment => {
  const html = renderToStaticMarkup(<Timeline blocks={[confirmed]} fontSize={18} alignment={alignment} color="white" displayMode="bilingual" showTimestamps />);
  expect(html).toContain("12:34:56");
  expect(html).toContain(`dateTime="${new Date(createdAt).toISOString()}"`);
  expect(html).toContain("padding-left:10px;padding-right:10px");
  expect(html).not.toContain("position:absolute;left:18px");
  expect(html).not.toContain("subtitle-audio-source");
  expect(html).toContain(`justify-content:${alignment === "center" ? "center" : alignment === "right" ? "flex-end" : "flex-start"}`);
});

it("keeps source identity accessible without repeated full-sized labels beside each sentence", () => {
  const html = renderToStaticMarkup(<Timeline blocks={[{ ...confirmed, audioSource: "system" }, { ...confirmed, id: "mic", audioSource: "microphone" }]}
    fontSize={18} alignment="left" color="white" displayMode="bilingual" showTimestamps />);
  expect(html.match(/subtitle-audio-source/g)).toHaveLength(2);
  expect(html.match(/role="img"/g)).toHaveLength(2);
  // Count visible timestamps only: in UTC the ISO dateTime attribute has
  // the same clock text, while local offsets can hide that mistake.
  expect(html.match(/>12:34:56<\/time>/g)).toHaveLength(2);
  expect(html).toContain('aria-label="System audio"');
  expect(html).toContain('aria-label="Microphone"');
  expect(html).not.toContain('>System audio<');
});

it("never assigns the current clock to live text, including immersive mode", () => {
  const props = { fontSize: 18, alignment: "center" as const, color: "white" as const, displayMode: "bilingual" as const, showTimestamps: true };
  expect(renderToStaticMarkup(<Timeline {...props} blocks={[{ ...confirmed, createdAt: null, presentation: "live" }]} />)).not.toContain("subtitle-timestamp");
  expect(renderToStaticMarkup(<Timeline {...props} blocks={[{ ...confirmed, createdAt: null, presentation: "live" }]} blendsWithBackground />)).not.toContain("subtitle-timestamp");
});

it.each([false, true])("honors the time opt-in in immersive mode (enabled=%s)", showTimestamps => {
  const html = renderToStaticMarkup(<Timeline blocks={[{ ...confirmed, audioSource: "microphone" }]}
    fontSize={18} alignment="center" color="white" displayMode="bilingual" audioInput="microphone"
    showTimestamps={showTimestamps} blendsWithBackground />);
  expect(html.includes("subtitle-timestamp")).toBe(showTimestamps);
  expect(html).toContain("filter:drop-shadow(");
  expect(html).toContain("color:rgba(255,255,255,0.95)");
  if (showTimestamps) {
    expect(html).toContain("text-shadow:");
    expect(html).toContain("font-size:13px;line-height:18px;font-weight:500");
  }
});

it.each([false, true])("protects retained microphone identity in system-only mode (time=%s)", showTimestamps => {
  const html = renderToStaticMarkup(<Timeline blocks={[{ ...confirmed, audioSource: "microphone" }]}
    fontSize={18} alignment="center" color="white" displayMode="bilingual" audioInput="system"
    showTimestamps={showTimestamps} blendsWithBackground />);
  expect(html).toContain("filter:drop-shadow(");
  expect(html.includes("subtitle-metadata")).toBe(showTimestamps);
  expect(html.includes("subtitle-timestamp")).toBe(showTimestamps);
});
