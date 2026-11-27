// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SubtitleColorControl } from "./SubtitleColorControl";
import { SUBTITLE_BACKGROUND_COLOR_OPTIONS, subtitleBackgroundHex } from "../../../lib/subtitles/subtitleColor";

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

function setNativeValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("SubtitleColorControl", () => {
  it("accepts manual HEX input with or without leading hash and maps presets cleanly", () => {
    const onChange = vi.fn();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root!.render(<SubtitleColorControl value="white" onChange={onChange} />);
    });

    const hexInput = host.querySelector<HTMLInputElement>("input.subtitle-color-hex")!;
    expect(hexInput.value).toBe("#FFFFFF");

    act(() => {
      setNativeValue(hexInput, " #1e293b ");
    });
    act(() => {
      hexInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(onChange).toHaveBeenLastCalledWith("#1E293B");
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });

  it("keeps invalid HEX drafts visible with an inline alert and reverts on Escape", () => {
    const onChange = vi.fn();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root!.render(
        <SubtitleColorControl
          label="Background color"
          value="#000000"
          options={SUBTITLE_BACKGROUND_COLOR_OPTIONS.map((option) => ({
            value: option.value,
            hex: option.value,
            label: option.label,
          }))}
          resolveHex={subtitleBackgroundHex}
          onChange={onChange}
        />,
      );
    });

    const hexInput = host.querySelector<HTMLInputElement>("input.subtitle-color-hex")!;
    act(() => { setNativeValue(hexInput, "#ZZZ"); });
    act(() => { hexInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(onChange).not.toHaveBeenCalled();
    expect(hexInput.value).toBe("#ZZZ");
    expect(host.querySelector('[role="alert"]')).not.toBeNull();

    act(() => { hexInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(hexInput.value).toBe("#000000");
    expect(host.querySelector('[role="alert"]')).toBeNull();

    // Disabling the control (e.g. enabling Immersive Mode) clears any transient invalid draft.
    act(() => { setNativeValue(hexInput, "#BAD"); });
    act(() => { hexInput.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); });
    expect(host.querySelector('[role="alert"]')).not.toBeNull();
    act(() => {
      root!.render(
        <SubtitleColorControl
          label="Background color"
          value="#000000"
          disabled
          options={SUBTITLE_BACKGROUND_COLOR_OPTIONS.map((option) => ({
            value: option.value,
            hex: option.value,
            label: option.label,
          }))}
          resolveHex={subtitleBackgroundHex}
          onChange={onChange}
        />,
      );
    });
    expect(hexInput.value).toBe("#000000");
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });
});
