import { expect, it } from "vitest";
import { tooltipPosition } from "./tooltipPosition";

function rect(left: number, top: number) {
  return { left, top, right: left + 24, bottom: top + 24, width: 24, height: 24 };
}

it("places an expanded-overlay hint below its icon and inside the right window edge", () => {
  expect(tooltipPosition(rect(600, 16), { width: 130, height: 24 }, { width: 640, height: 136 }))
    .toEqual({ left: 504, top: 44 });
});

it("places collapsed-overlay hints beside the icon when neither above nor below fits", () => {
  expect(tooltipPosition(rect(244, 15), { width: 130, height: 24 }, { width: 280, height: 54 }))
    .toEqual({ left: 110, top: 15 });
  expect(tooltipPosition(rect(8, 15), { width: 130, height: 24 }, { width: 280, height: 54 }))
    .toEqual({ left: 36, top: 15 });
});

it("uses the space above an icon near the bottom edge", () => {
  expect(tooltipPosition(rect(400, 100), { width: 130, height: 24 }, { width: 640, height: 136 }))
    .toEqual({ left: 347, top: 72 });
});

it("keeps a wrapped label inside a short native viewport even if it has to overlap nearby content", () => {
  const position = tooltipPosition(rect(244, 15), { width: 268, height: 38 }, { width: 280, height: 54 });
  expect(position).toEqual({ left: 6, top: 10 });
  expect(position.left + 268).toBeLessThanOrEqual(274);
  expect(position.top + 38).toBeLessThanOrEqual(48);
});
