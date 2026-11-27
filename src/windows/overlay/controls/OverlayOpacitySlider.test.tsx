// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OVERLAY_POINTER_TARGET_EVENT } from "../../../lib/overlay/overlayPointer";
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

    // Continues tracking via window pointermove even if the cursor drifts off the narrow strip.
    act(() => {
      window.dispatchEvent(pointerEvent("pointermove", { clientY: 120, buttons: 1, pointerId: 1 }));
    });
    expect(onChange).toHaveBeenLastCalledWith(80);

    act(() => {
      window.dispatchEvent(pointerEvent("pointerup", { clientY: 120, pointerId: 1 }));
    });
    expect(host?.querySelector("[data-dragging]")).toBeNull();
  });

  it("supports wheel adjustment over the slider without bubbling", () => {
    const onChange = vi.fn();
    const { control } = mount(50, onChange);
    const parentWheel = vi.fn();
    host!.addEventListener("wheel", parentWheel);

    act(() => {
      control.dispatchEvent(new WheelEvent("wheel", { deltaY: -10, bubbles: true, cancelable: true }));
    });
    expect(onChange).toHaveBeenLastCalledWith(55);
    expect(parentWheel).not.toHaveBeenCalled();

    act(() => {
      control.dispatchEvent(new WheelEvent("wheel", { deltaY: 10, bubbles: true, cancelable: true }));
    });
    expect(onChange).toHaveBeenLastCalledWith(45);
  });

  it("tracks native pointer hover targets for nonactivating overlay windows", () => {
    const { input } = mount(80, vi.fn());
    const slider = host!.querySelector('[data-testid="overlay-opacity-slider"]')!;
    expect(slider.hasAttribute("data-hovered")).toBe(false);

    act(() => {
      document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: input }));
    });
    expect(slider.hasAttribute("data-hovered")).toBe(true);

    act(() => {
      document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: null }));
    });
    expect(slider.hasAttribute("data-hovered")).toBe(false);
  });

  it("ignores input when disabled", () => {
    const onChange = vi.fn();
    const { input } = mount(50, onChange, true);
    act(() => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true })); });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("clears dragging if pointerup arrives immediately before effect commit and ignores secondary pointers", () => {
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

    // Secondary pointer while primary is already dragging must be ignored.
    act(() => {
      control.dispatchEvent(pointerEvent("pointerdown", { clientY: 140, button: 0, pointerId: 1 }));
      control.dispatchEvent(pointerEvent("pointerdown", { clientY: 180, button: 0, pointerId: 2 }));
    });
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenLastCalledWith(60);

    // Releasing pointer 2 must not end pointer 1; releasing pointer 1 ends drag.
    act(() => {
      window.dispatchEvent(pointerEvent("pointerup", { clientY: 180, pointerId: 2 }));
    });
    expect(host?.querySelector("[data-dragging]")).not.toBeNull();
    act(() => {
      window.dispatchEvent(pointerEvent("pointerup", { clientY: 140, pointerId: 1 }));
    });
    expect(host?.querySelector("[data-dragging]")).toBeNull();

    // Pointerdown + window pointerup inside the same act batch must not leave data-dragging stuck.
    act(() => {
      control.dispatchEvent(pointerEvent("pointerdown", { clientY: 150, button: 0, pointerId: 3 }));
      window.dispatchEvent(pointerEvent("pointerup", { clientY: 150, pointerId: 3 }));
    });
    expect(host?.querySelector("[data-dragging]")).toBeNull();
  });

  it("keeps wheel listener stable across parent rerenders while reading updated props", () => {
    const firstOnChange = vi.fn();
    const secondOnChange = vi.fn();
    const { control } = mount(50, firstOnChange);
    const addSpy = vi.spyOn(control, "addEventListener");
    const removeSpy = vi.spyOn(control, "removeEventListener");

    // Simulate frequent parent rerenders with updated opacity & new onChange identities.
    act(() => {
      root!.render(<OverlayOpacitySlider opacity={55} onChange={firstOnChange} />);
      root!.render(<OverlayOpacitySlider opacity={60} onChange={secondOnChange} />);
    });
    expect(addSpy).not.toHaveBeenCalled();
    expect(removeSpy).not.toHaveBeenCalled();

    act(() => {
      control.dispatchEvent(new WheelEvent("wheel", { deltaY: -10, bubbles: true, cancelable: true }));
    });
    expect(firstOnChange).not.toHaveBeenCalled();
    expect(secondOnChange).toHaveBeenCalledWith(65);
  });
});
