import { describe, expect, it } from "vitest";
import { emptyStateDensity, timelineClassName } from "./overlayModel";

describe("overlay responsive presentation", () => {
  it("keeps the status text visible at the native minimum height", () => {
    expect(emptyStateDensity(100)).toBe("minimal");
    expect(emptyStateDensity(112)).toBe("minimal");
    expect(emptyStateDensity(113)).toBe("compact");
    expect(emptyStateDensity(175)).toBe("compact");
    expect(emptyStateDensity(176)).toBe("comfortable");
    expect(emptyStateDensity(240)).toBe("comfortable");
  });

  it("hides only the immersive timeline scrollbar", () => {
    expect(timelineClassName(true)).toContain("overlay-timeline--immersive");
    expect(timelineClassName(false)).not.toContain(
      "overlay-timeline--immersive",
    );
  });
});
