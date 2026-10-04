import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { Timeline } from "./Timeline";
import type { SubtitleBlock } from "./overlayModel";
import type { SubtitleColor } from "../../lib/types";

function block(overrides: Partial<SubtitleBlock> = {}): SubtitleBlock {
  return {
    id: "block",
    createdAt: null,
    presentation: "latestCommitted",
    source: null,
    translation: "Subtitle",
    ...overrides,
  };
}

describe("subtitle colors", () => {
  it.each([false, true])("renders every preset in immersive=%s", (immersive) => {
    const palette: [SubtitleColor, string][] = [
      ["white", "255,255,255"], ["teal", "79,209,197"],
      ["yellow", "255,213,79"], ["green", "154,230,110"], ["pink", "244,154,181"], ["#123456", "18,52,86"],
    ];
    for (const [color, rgb] of palette) {
      const html = renderToStaticMarkup(
        <Timeline
          blocks={[block()]}
          fontSize={18}
          alignment="center"
          color={color}
          displayMode="translation"
          blendsWithBackground={immersive}
        />,
      );
      expect(html.replaceAll(" ", "")).toContain(`color:rgba(${rgb},1)`);
    }
  });

  it("keeps bilingual sources neutral and history at full opacity", () => {
    const html = renderToStaticMarkup(
      <Timeline
        blocks={[
          block({ id: "older", presentation: "history", translation: "Older translation" }),
          block({ id: "old", presentation: "history", source: "Old source", translation: "旧译文" }),
          block({ id: "live", presentation: "live", source: "Source", translation: "Translation" }),
        ]}
        fontSize={18}
        alignment="left"
        color="#123456"
        displayMode="bilingual"
      />,
    ).replaceAll(" ", "");
    // The recognized original is the neutral reference lane, the translation
    // reads in the user's subtitle color.
    expect(html).toContain("color:rgba(255,255,255,0.86)");
    expect(html).toContain("color:rgba(18,52,86,1)");
    // Neither the previous sentence nor older history loses contrast.
    expect(html).not.toContain("opacity:");
  });

  it("keeps the recognized lane primary when it is the only language shown", () => {
    const html = renderToStaticMarkup(
      <Timeline
        blocks={[block({ source: "Hello", translation: null })]}
        fontSize={18}
        alignment="center"
        color="#123456"
        displayMode="original"
      />,
    ).replaceAll(" ", "");
    expect(html).toContain("color:rgba(18,52,86,1)");
    expect(html).toContain("font-size:18px");
  });

  it("keeps the same reference size and contrast before its bilingual translation arrives", () => {
    const html = renderToStaticMarkup(
      <Timeline blocks={[block({ source: "Recognized phrase", translation: null, presentation: "live", streaming: true })]}
        fontSize={18} alignment="center" color="white" displayMode="bilingual" />,
    ).replaceAll(" ", "");
    expect(html).toContain("font-size:16.2px");
    expect(html).toContain("color:rgba(255,255,255,0.86)");
    expect(html).not.toContain("stream-dots");
    expect(html).toContain("Recognizedphrase");
    expect(html).not.toContain("stream-chunk");
  });
});

it.each([false, true])("keeps all subtitle text opaque without changing immersive=%s backgrounds", immersive => {
  const html = renderToStaticMarkup(<Timeline blocks={[
    block({ id: "older", presentation: "history", audioSource: "system", source: "Old", translation: "Older" }),
    block({ id: "latest", presentation: "live", audioSource: "microphone", source: "Live", translation: "Latest" }),
  ]} fontSize={18} alignment="left" color="white" displayMode="bilingual" keepTextOpaque blendsWithBackground={immersive} />).replaceAll(" ", "");
  expect(html).not.toContain("opacity:0.");
  expect(html).not.toContain("255,255,255,0.86");
  expect(html).not.toContain("subtitle-block");
  expect(html).not.toContain("subtitle-lane");
  expect(html.includes("text-shadow:")).toBe(immersive);
});

it("colors each confirmed and live source with its own preference, including both bilingual lanes", () => {
  const html = renderToStaticMarkup(<Timeline blocks={[
    block({ id: "system", presentation: "history", audioSource: "system", source: "System", translation: "System translated" }),
    block({ id: "microphone", presentation: "live", audioSource: "microphone", source: "Microphone", translation: "Microphone translated" }),
  ]} fontSize={18} alignment="left" color="#123456" microphoneColor="#abcdef" displayMode="bilingual" keepTextOpaque />).replaceAll(" ", "");
  const system = html.slice(html.indexOf('data-utterance-id="system"'), html.indexOf('data-utterance-id="microphone"'));
  const microphone = html.slice(html.indexOf('data-utterance-id="microphone"'));
  expect(system.match(/color:rgba\(18,52,86,1\)/g)).toHaveLength(2);
  expect(system).not.toContain("171,205,239");
  expect(microphone.match(/color:rgba\(171,205,239,1\)/g)).toHaveLength(2);
  expect(microphone).not.toContain("18,52,86");
});

it.each(["system", "microphone"] as const)("keeps single-source %s color and neutral references independently of metadata", audioSource => {
  const html = renderToStaticMarkup(<Timeline blocks={[
    block({ audioSource, createdAt: 10, source: "Original", translation: "Translation" }),
  ]} fontSize={18} alignment="left" color="#123456" microphoneColor="#abcdef"
    displayMode="bilingual" audioInput={audioSource} showTimestamps />).replaceAll(" ", "");
  expect(html).toContain("color:rgba(255,255,255,0.86)");
  expect(html).toContain(audioSource === "microphone" ? "color:rgba(171,205,239,1)" : "color:rgba(18,52,86,1)");
  expect(html.includes("subtitle-audio-source")).toBe(audioSource === "microphone");
  expect(html).toContain("subtitle-timestamp");
});
