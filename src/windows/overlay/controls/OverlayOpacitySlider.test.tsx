// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OverlayOpacitySlider } from "./OverlayOpacitySlider";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
});

afterEach(() => {
  act(() => { root?.unmount(); });
  host?.remove();
  host = null;
  root = null;
  vi.unstubAllGlobals();
});

function mount(opacity: number, onChange: (value: number) => void, disabled = false) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root!.render(<OverlayOpacitySlider opacity={opacity} disabled={disabled} onChange={onChange} />);
  });
  return {
    control: host.querySelector<HTMLDivElement>(".overlay-opacity-slider__control")!,
    input: host.querySelector<HTMLInputElement>('input[type="range"]')!,
  };
}

describe("OverlayOpacitySlider", () => {
  it("adjusts opacity via keyboard and clamps to 0..100", () => {
    const onChange = vi.fn();
    const { input } = mount(80, onChange);

    act(() => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })); });
    expect(onChange).toHaveBeenLastCalledWith(85);

    act(() => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })); });
    expect(onChange).toHaveBeenLastCalledWith(75);

    act(() => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true })); });
    expect(onChange).toHaveBeenLastCalledWith(0);

    act(() => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true })); });
    expect(onChange).toHaveBeenLastCalledWith(100);
  });

  it("maps vertical pointer drag from bottom (0%) to top (100%)", () => {
    const onChange = vi.fn();
    const { control } = mount(80, onChange);
    control.getBoundingClientRect = () => ({
      x: 0, y: 100, left: 0, top: 100, right: 24, bottom: 200, width: 24, height: 100, toJSON: () => ({}),
    });

    const pointerEvent = (type: string, init: MouseEventInit & { pointerId?: number; isPrimary?: boolean }) => {
      const event = new MouseEvent(type, { bubbles: true, ...init });
      Object.defineProperty(event, "pointerId", { value: init.pointerId ?? 1 });
      Object.defineProperty(event, "isPrimary", { value: init.isPrimary ?? true });
      return event;
    };

    act(() => {
      control.dispatchEvent(pointerEvent("pointerdown", { clientY: 140, button: 0, pointerId: 1 }));
    });
    expect(onChange).toHaveBeenLastCalledWith(60);

    act(() => {
      control.dispatchEvent(pointerEvent("pointermove", { clientY: 190, buttons: 1, pointerId: 1 }));
    });
    expect(onChange).toHaveBeenLastCalledWith(10);

    act(() => {
      control.dispatchEvent(pointerEvent("pointerup", { clientY: 190, pointerId: 1 }));
    });
    expect(host?.querySelector("[data-dragging]")).toBeNull();
  });

  it("ignores input when disabled", () => {
    const onChange = vi.fn();
    const { input } = mount(50, onChange, true);
    act(() => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })); });
    expect(onChange).not.toHaveBeenCalled();
  });
});
