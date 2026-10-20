// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { subtitleColorHex } from "../../lib/subtitleColor";
import { subtitleFontFamily } from "../../lib/subtitleFont";
import { Timeline } from "./Timeline";
import { buildSubtitleBlocks } from "./overlayModel";

vi.mock("../../lib/subtitleColor", async original => {
  const actual = await original<typeof import("../../lib/subtitleColor")>();
  return { ...actual, subtitleColorHex: vi.fn(actual.subtitleColorHex) };
});
vi.mock("../../lib/subtitleFont", async original => {
  const actual = await original<typeof import("../../lib/subtitleFont")>();
  return { ...actual, subtitleFontFamily: vi.fn(actual.subtitleFontFamily) };
});

it("renders arriving text without repeating unchanged history presentation work", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const bounds = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockReturnValue({ top: 0, bottom: 24, height: 24 } as DOMRect);
  const height = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true,
    get(this: HTMLElement) { return Number.parseFloat(this.style.height) || 100; } });
  const scrollTo = HTMLElement.prototype.scrollTo;
  HTMLElement.prototype.scrollTo = vi.fn();
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const history = Array.from({ length: 20 }, (_, index) => ({
    source: `Confirmed source ${index}`, translation: `Confirmed translation ${index}`, createdAt: index + 1,
  }));
  const render = (frame: number) => root.render(<Timeline
    // Native snapshots and their projected block objects are fresh each time.
    blocks={buildSubtitleBlocks(history.map(pair => ({ ...pair })), "bilingual", {
      source: "Current original", translation: `Current translation ${frame}`, isStreaming: true, utteranceId: "current",
    })}
    fontSize={18} fontFamily="Synthetic Font" alignment="left" color="white" displayMode="bilingual" audioInput="system" motionEnabled={false}
  />);
  try {
    await act(() => render(0));
    const rows = [...host.querySelectorAll("[data-utterance-id]")];
    vi.mocked(subtitleColorHex).mockClear();
    vi.mocked(subtitleFontFamily).mockClear();
    for (let frame = 1; frame <= 20; frame++) {
      await act(() => render(frame));
      expect(host.querySelector('[data-utterance-id="live-utterance-current"]')!.textContent)
        .toContain(`Current translation ${frame}`);
    }
    // Only the twenty arriving translations need their reading color resolved.
    // The forty unchanged historical lanes and the original remain reusable.
    expect(subtitleColorHex).toHaveBeenCalledTimes(20);
    expect(subtitleFontFamily).not.toHaveBeenCalled();
    [...host.querySelectorAll("[data-utterance-id]")].forEach((row, index) => expect(row).toBe(rows[index]));
  } finally {
    await act(() => root.unmount()); host.remove(); bounds.mockRestore();
    HTMLElement.prototype.scrollTo = scrollTo;
    if (height) Object.defineProperty(HTMLElement.prototype, "clientHeight", height);
    else delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
    vi.mocked(subtitleColorHex).mockClear(); vi.mocked(subtitleFontFamily).mockClear();
    vi.unstubAllGlobals();
  }
});
