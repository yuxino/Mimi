import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { Timeline } from "../Timeline";
import type { SubtitleBlock } from "../overlayModel";
import type { SettingsSnapshot } from "../../../../lib/types";

const HISTORY: SubtitleBlock = {
  id: "history-1",
  createdAt: 1,
  presentation: "history",
  source: "Hello world",
  translation: "你好世界",
};

const LIVE: SubtitleBlock = {
  id: "live",
  createdAt: null,
  presentation: "live",
  source: "Streaming source",
  translation: "流式译文",
};

function render(
  blocks: SubtitleBlock[],
  displayMode: SettingsSnapshot["subtitleDisplayMode"] = "bilingual",
  blendsWithBackground = false,
  showSubtitleDividers = false,
): string {
  return renderToStaticMarkup(
    <Timeline
      blocks={blocks}
      fontSize={18}
      alignment="center"
      color="white"
      displayMode={displayMode}
      blendsWithBackground={blendsWithBackground}
      showSubtitleDividers={showSubtitleDividers}
    />,
  ).replaceAll(" ", "");
}

describe("sentence block presentation", () => {
  it("keeps long confirmed history compact while following, retaining its full text", () => {
    const html = render([HISTORY]);
    expect(html).toContain("Helloworld");
    expect(html).toContain("overflow:hidden");
    expect(html).toContain('aria-label="Helloworld"');
  });

  it("reserves a small fallback before layout and keeps the full text accessible", () => {
    const html = render([HISTORY, LIVE]);
    // Bilingual live budget: one recognized line, two translation lines.
    expect(html).toContain("height:22px");
    expect(html).toContain("height:48px");
    expect(html).toContain("overflow:hidden");
    expect(html).toContain('aria-label="Streamingsource"');
    expect(html).toContain('aria-label="流式译文"');
  });

  it("sizes the compact budget from the display mode", () => {
    // Translation-only keeps two translation lines and no recognized lane.
    const translationOnly = render([{ ...LIVE, source: null }], "translation");
    expect(translationOnly).toContain("height:48px");
    expect(translationOnly).not.toContain('aria-label="Streamingsource"');

    // Original-only, and bilingual before its translation arrives, keep two
    // recognized lines.
    for (const mode of ["original", "bilingual"] as const) {
      const html = render([{ ...LIVE, translation: null }], mode);
      expect(html).toContain(mode === "bilingual" ? "height:44px" : "height:48px");
    }
  });

  it("defaults sentence dividers off and enables one readable rule between sentences only", () => {
    expect(render([HISTORY, LIVE])).not.toContain("subtitle-separator");
    const enabled = render([HISTORY, LIVE], "bilingual", false, true);
    expect(enabled.match(/subtitle-separator/g)).toHaveLength(1);
    expect(enabled).toContain("width:100%");
    expect(render([LIVE], "bilingual", false, true)).not.toContain("subtitle-separator");
    expect(render([HISTORY, LIVE], "bilingual", true, true)).not.toContain(
      "subtitle-separator",
    );
  });
});
