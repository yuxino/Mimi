import { describe, expect, it } from "vitest";
import { overlayTopChromeLayout, overlaySessionChromeLayout } from "./overlayChromeLayout";

function handleEdges(layout: ReturnType<typeof overlayTopChromeLayout>) {
  return {
    left: layout.dragHandleCenterX - layout.dragHandleWidth / 2,
    right: layout.dragHandleCenterX + layout.dragHandleWidth / 2,
  };
}

describe("overlay top chrome layout", () => {
  it("keeps the native capsule clearance and recovery actions after a session fails", () => {
    const error = overlaySessionChromeLayout(640, { isActive: false, status: { kind: "error", message: "unavailable" } });
    const listening = overlaySessionChromeLayout(640, { isActive: true, status: { kind: "listening" } });
    expect(error).toEqual(listening);
    expect(error.topBandHeight).toBe(61);
    expect(error.showActions).toBe(true);
    expect(error.showControls).toBe(true);
    expect(error.showPrimaryAction).toBe(true);
  });
  it("retains the pause or retry action at the native minimum width", () => {
    for (const status of [{ kind: "listening" } as const, { kind: "error", message: "unavailable" } as const]) {
      const layout = overlaySessionChromeLayout(348, { isActive: status.kind === "listening", status });
      expect(layout.showActions).toBe(false);
      expect(layout.showPrimaryAction).toBe(true);
      expect(layout.topBandHeight).toBe(61);
    }
  });
  it("leaves an idle overlay without an island in its smaller chrome layout", () => {
    const idle = overlaySessionChromeLayout(640, { isActive: false, status: { kind: "idle" } });
    expect(idle.showActions).toBe(false);
    expect(idle.showControls).toBe(false);
    expect(idle.topBandHeight).toBe(37);
  });
  it("reserves a centered drag handle between metadata at 360px", () => {
    const layout = overlayTopChromeLayout(360, true);
    const handle = handleEdges(layout);

    expect(layout.showActions).toBe(false);
    expect(layout.dragHandleCenterX).toBe(180);
    expect(handle.left).toBeGreaterThanOrEqual(140);
    expect(360 - handle.right).toBeGreaterThanOrEqual(140);
  });

  it("keeps the compact fallback through widths that cannot fit all actions", () => {
    for (const width of [400, 480]) {
      const layout = overlayTopChromeLayout(width, true);
      expect(layout.showActions).toBe(false);
      expect(layout.dragHandleCenterX).toBe(width / 2);
      expect(handleEdges(layout).left).toBeGreaterThanOrEqual(140);
    }
  });

  it("restores actions without shifting the handle off the window center", () => {
    const layout = overlayTopChromeLayout(552, true);

    expect(layout.showActions).toBe(true);
    expect(layout.dragHandleCenterX).toBe(276);
    expect(layout.dragHandleWidth).toBe(68);
  });

  it("preserves the centered 120px handle and all actions at 640px", () => {
    expect(overlayTopChromeLayout(640, true)).toEqual({
      dragHandleCenterX: 320,
      dragHandleWidth: 120,
      showActions: true,
    });
  });

  it("keeps an inactive overlay centered because it has no control island", () => {
    expect(overlayTopChromeLayout(360, false)).toEqual({
      dragHandleCenterX: 180,
      dragHandleWidth: 100,
      showActions: false,
    });
  });
});
