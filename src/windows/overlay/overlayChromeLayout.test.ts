import { describe, expect, it } from "vitest";
import { overlaySessionChromeLayout } from "./overlayChromeLayout";

const listening = { isActive: true, status: { kind: "listening" as const } };
const error = { isActive: false, status: { kind: "error" as const, message: "unavailable" } };

describe("subtitle action strip", () => {
  it.each([listening, error])("keeps all three actions beside a maximum-width language capsule at minimum width", session => {
    expect(overlaySessionChromeLayout(348, session)).toMatchObject({
      showPrimaryAction: true, showSessionAction: true, showActions: true,
    });
  });

  it("keeps the same controls at intermediate and default sizes", () => {
    expect(overlaySessionChromeLayout(400, listening)).toMatchObject({
      showPrimaryAction: true, showSessionAction: true, showActions: true,
    });
    expect(overlaySessionChromeLayout(568, listening)).toMatchObject({
      showPrimaryAction: true, showSessionAction: true, showActions: true,
    });
    expect(overlaySessionChromeLayout(568, error)).toEqual(overlaySessionChromeLayout(568, listening));
  });

  it("keeps actions out of the idle surface", () => {
    expect(overlaySessionChromeLayout(568, { isActive: false, status: { kind: "idle" } })).toMatchObject({
      showControls: false, showPrimaryAction: false, showSessionAction: false, showActions: false,
    });
  });
});
