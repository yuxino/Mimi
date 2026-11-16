// @vitest-environment jsdom
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useOverlayChromePeers } from "./useOverlayChromePeers";

it("shares pointer entry and exit between separate subtitle and language surfaces", async () => {
  const channels = new Set<LocalChannel>();
  class LocalChannel extends EventTarget {
    readonly name: string;
    constructor(name: string) {
      super();
      this.name = name;
      channels.add(this);
    }
    postMessage(data: unknown) {
      for (const peer of channels) {
        if (peer !== this && peer.name === this.name) {
          queueMicrotask(() => peer.dispatchEvent(new MessageEvent("message", { data })));
        }
      }
    }
    close() { channels.delete(this); }
  }
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("BroadcastChannel", LocalChannel);
  vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener() {}, removeEventListener() {} }));
  const mount = document.createElement("div");
  document.body.append(mount);
  const root = createRoot(mount);
  function Surface({ kind }: { kind: "overlay" | "island" }) {
    const ref = useRef<HTMLDivElement>(null);
    useOverlayChromePeers(ref, kind, true);
    return <div ref={ref} data-test-surface={kind} />;
  }
  try {
    await act(async () => root.render(<><Surface kind="overlay" /><Surface kind="island" /></>));
    const overlay = mount.querySelector('[data-test-surface="overlay"]')!;
    const island = mount.querySelector('[data-test-surface="island"]')!;
    const exposure = "data-overlay-peer-hover";
    expect(island.hasAttribute(exposure)).toBe(false);
    await act(async () => overlay.dispatchEvent(new Event("pointermove")));
    expect(island.hasAttribute(exposure)).toBe(true);
    await act(async () => {
      overlay.dispatchEvent(new Event("pointerleave"));
      island.dispatchEvent(new Event("pointerenter"));
    });
    expect(overlay.hasAttribute(exposure)).toBe(true);
    expect(island.hasAttribute(exposure)).toBe(false);
    await act(async () => island.dispatchEvent(new Event("pointerleave")));
    expect(overlay.hasAttribute(exposure)).toBe(false);
  } finally {
    await act(async () => root.unmount());
    mount.remove();
    vi.unstubAllGlobals();
  }
  expect(channels.size).toBe(0);
});
