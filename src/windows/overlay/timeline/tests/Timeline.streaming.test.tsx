import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Timeline } from "../Timeline";
import type { SubtitleBlock } from "../overlayModel";

const LIVE_STREAMING: SubtitleBlock = {
  id: "live",
  createdAt: null,
  presentation: "live",
  source: "It is not attached",
  translation: "它并未附着",
  streaming: true,
};

const LIVE_SETTLED: SubtitleBlock = {
  id: "live",
  createdAt: null,
  presentation: "live",
  source: "It is not attached",
  translation: "它并未附着",
};

const COMMITTED: SubtitleBlock = {
  id: "history-1700",
  createdAt: 1_700_000_000_000,
  presentation: "latestCommitted",
  source: "It is not attached",
  translation: "它并未附着",
};

function render(blocks: SubtitleBlock[], displayMode: "bilingual" | "original" = "bilingual") {
  return renderToStaticMarkup(
    <Timeline
      showTimestamps
      blocks={blocks}
      fontSize={18}
      alignment="center"
      color="white"
      displayMode={displayMode}
      motionEnabled
    />,
  );
}

/** The markup of one lane, addressed by its accessible name. */
function laneMarkup(markup: string, label: string): string {
  const start = markup.indexOf(`aria-label="${label}"`);
  expect(start).toBeGreaterThan(-1);
  const rest = markup.slice(start + 1);
  const next = rest.indexOf("aria-label=");
  return next === -1 ? rest : rest.slice(0, next);
}

describe("live streaming row", () => {
  it("renders arriving and settled text identically without an inline status marker", () => {
    const markup = render([LIVE_STREAMING]);
    expect(markup).not.toContain("stream-dots");
    expect(markup).toBe(render([LIVE_SETTLED]));
  });

  it("shows each revised phrase immediately at full opacity without word fades", () => {
    const markup = render([LIVE_STREAMING]);
    expect(laneMarkup(markup, "它并未附着")).toContain("它并未附着");
    expect(markup).not.toContain("stream-chunk");
    expect(markup).toContain("It is not attached");
  });

  it("keeps settled body text free of loaders", () => {
    expect(render([LIVE_SETTLED])).not.toContain("stream-dots");
  });
});

describe("committed row", () => {
  it("does not replay an entrance animation, so a commit never blinks", () => {
    const markup = render([COMMITTED]);
    expect(markup).not.toContain("subtitle-block");
    expect(markup).not.toContain("subtitle-lane");
  });

  it("renders plain text with no animation wrappers left behind", () => {
    const markup = render([COMMITTED]);
    expect(markup).not.toContain("stream-chunk");
    expect(markup).not.toContain("stream-dots");
    expect(markup).toContain("它并未附着");
  });

  it("keeps its optional timestamp, which the live row does not have", () => {
    expect(render([COMMITTED])).toContain("subtitle-timestamp");
    expect(render([LIVE_STREAMING])).not.toContain("subtitle-timestamp");
  });
});

describe("display modes", () => {
  it("keeps original-only arriving text plain", () => {
    const original = render([LIVE_STREAMING], "original");
    expect(original).not.toContain("stream-dots");
    expect(original).toContain("It is not attached");
    expect(original).toBe(render([LIVE_SETTLED], "original"));
    expect(original).not.toContain("它并未附着");
  });

  it("keeps history rows free of wrappers in every mode", () => {
    for (const mode of ["bilingual", "original"] as const) {
      const markup = render([COMMITTED], mode);
      expect(markup).not.toContain("subtitle-block");
      expect(markup).not.toContain("stream-chunk");
    }
  });
});

describe("streaming corrections", () => {
  it("replaces a correction as one phrase without fading words again", () => {
    const markup = render([LIVE_STREAMING]);
    const corrected = render([{ ...LIVE_STREAMING, translation: "它没有附着在表面上" }]);
    expect(markup).toContain("它并未附着");
    expect(corrected).toContain("它没有附着在表面上");
    expect(corrected).not.toContain("它并未附着");
    expect(corrected).not.toContain("stream-chunk");
  });
});

it.each(["Wait... really?", "等等……真的吗？", "待って…本当？", "잠깐... 정말?"])("preserves literal punctuation in arriving source text: %s", (source) => {
  const block = { ...LIVE_STREAMING, source, translation: null };
  const markup = render([block], "original");
  expect(markup).toContain(source);
  expect(markup).toContain(`aria-label="${source}"`);
  expect(markup).not.toContain("stream-dots");
  expect(markup).toBe(render([{ ...block, streaming: undefined }], "original"));
});

// Hiding metadata does not remove any subtitle sentence or language lane.
it("hides timestamps by default without hiding either subtitle lane", () => {
  const html = renderToStaticMarkup(<Timeline blocks={[COMMITTED]} fontSize={18} alignment="center" color="white" displayMode="bilingual" />);
  expect(html).not.toContain("subtitle-timestamp");
  expect(html).toContain("It is not attached");
  expect(html).toContain("它并未附着");
});
