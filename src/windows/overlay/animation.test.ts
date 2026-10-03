import { describe, expect, it } from "vitest";
import { resolveMotion } from "./animation";
import { OVERLAY_ACTIVITY_PHASES } from "../../lib/types";

describe("resolveMotion", () => {
  it("lets an explicit choice override the system preference", () => {
    expect(resolveMotion(true, true)).toBe(true);
    expect(resolveMotion(false, false)).toBe(false);
  });

  it("follows the system preference until the user chooses", () => {
    expect(resolveMotion(null, true)).toBe(false);
    expect(resolveMotion(null, false)).toBe(true);
  });
});

describe("activity phases that animate", () => {
  it("only the working phases are active, so a paused session is still", () => {
    const active = Object.entries(OVERLAY_ACTIVITY_PHASES)
      .filter(([, info]) => info.animationSpeed > 0)
      .map(([phase]) => phase)
      .sort();
    expect(active).toEqual(["connecting", "listening", "recognizing", "translating"]);
  });
});
