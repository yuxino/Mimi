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
  expect(html.match(/12:34:56/g)).toHaveLength(2);
  expect(html).toContain('aria-label="System audio"');
  expect(html).toContain('aria-label="Microphone"');
  expect(html).not.toContain('>System audio<');
});

it("never assigns the current clock to live text and keeps timestamps hidden in immersive mode", () => {
  const props = { fontSize: 18, alignment: "center" as const, color: "white" as const, displayMode: "bilingual" as const, showTimestamps: true };
  expect(renderToStaticMarkup(<Timeline {...props} blocks={[{ ...confirmed, createdAt: null, presentation: "live" }]} />)).not.toContain("subtitle-timestamp");
  expect(renderToStaticMarkup(<Timeline {...props} blocks={[confirmed]} blendsWithBackground />)).not.toContain("subtitle-timestamp");
});
