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
