// @vitest-environment jsdom
import { act, useLayoutEffect, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useOverlayChromePeers } from "./useOverlayChromePeers";

const transport = vi.hoisted(() => {
  type Target = { kind: string; label: string };
  type Receiver = (event: { payload: unknown }) => void;
  const receivers = new Map<string, Receiver>();
  const listen = vi.fn(async (event: string, callback: Receiver, { target }: { target: Target }) => {
    const key = JSON.stringify([event, target]);
    receivers.set(key, callback);
    return () => receivers.delete(key);
  });
  const emitTo = vi.fn(async (target: Target, event: string, payload: unknown) => {
    receivers.get(JSON.stringify([event, target]))?.({ payload });
  });
  return { emitTo, listen, receivers };
});

vi.mock("@tauri-apps/api/event", () => ({ emitTo: transport.emitTo, listen: transport.listen }));

type SurfaceKind = "overlay" | "island";
let mount: HTMLElement;
let root: Root;
let nativePointer: Partial<Record<SurfaceKind, (inside: boolean) => void>>;
const ownHover = "data-overlay-hover";
const peerHover = "data-overlay-peer-hover";
const keyboardFocus = "data-overlay-keyboard-focus";

function Surface({ kind, share = true }: { kind: SurfaceKind; share?: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const updateNativePointer = useOverlayChromePeers(ref, kind, share);
  useLayoutEffect(() => { nativePointer[kind] = updateNativePointer; }, [kind, updateNativePointer]);
  return <div ref={ref} data-test-surface={kind}><button type="button">Focus control</button></div>;
}

function surface(kind: SurfaceKind) {
  return mount.querySelector(`[data-test-surface="${kind}"]`)!;
}

function shown(kind: SurfaceKind) {
  const element = surface(kind);
  return element.hasAttribute(ownHover) || element.hasAttribute(peerHover);
}

async function move(kind: SurfaceKind, event: "pointerenter" | "pointermove" | "pointerleave") {
  await act(async () => surface(kind).dispatchEvent(new Event(event)));
}

async function tick(ms: number) {
  await act(async () => vi.advanceTimersByTimeAsync(ms));
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener() {}, removeEventListener() {} }));
  vi.clearAllMocks();
  nativePointer = {};
  mount = document.createElement("div");
  document.body.append(mount);
  root = createRoot(mount);
});

afterEach(async () => {
  await act(async () => root.unmount());
  mount.remove();
  expect(transport.receivers.size).toBe(0);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("uses independent native and DOM hover across blur and sends only exposure transitions", async () => {
  await act(async () => root.render(<><Surface kind="overlay" /><Surface kind="island" /></>));
  transport.emitTo.mockClear();
  await move("overlay", "pointerenter");
  await act(async () => nativePointer.overlay?.(true));
  for (let count = 0; count < 12; count++) await move("overlay", "pointermove");
  await move("overlay", "pointerleave");
  await act(async () => window.dispatchEvent(new Event("blur")));
  await tick(200);

  expect(surface("overlay").hasAttribute(ownHover)).toBe(true);
  expect(surface("island").hasAttribute(peerHover)).toBe(true);
  expect(transport.emitTo).toHaveBeenCalledExactlyOnceWith(
    { kind: "WebviewWindow", label: "overlay-control" },
    "overlay-chrome-hover", { kind: "visibility", visible: true },
  );

  await act(async () => nativePointer.overlay?.(false));
  await tick(159);
  expect(shown("overlay") && shown("island")).toBe(true);
  await tick(1);
  expect(shown("overlay") || shown("island")).toBe(false);
});

it.each([true, false])("exposes keyboard focus only while the document has focus (share=%s)", async share => {
  await act(async () => root.render(<><Surface kind="overlay" share={share} /><Surface kind="island" share={share} /></>));
  const island = surface("island");
  const control = island.querySelector("button")!;
  await act(async () => control.focus());
  expect(control.matches(":focus-visible")).toBe(true);
  expect(island.hasAttribute(keyboardFocus)).toBe(true);
  expect(surface("overlay").hasAttribute(peerHover)).toBe(share);
  expect(island.hasAttribute(ownHover)).toBe(false);

  const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
  // Model a WebView whose selector retains the focused descendant on blur.
  const query = island.querySelector.bind(island);
  const retainedFocus = vi.spyOn(island, "querySelector").mockImplementation(selector =>
    selector === ":focus-visible" && document.activeElement === control ? control : query(selector));
  try {
    await act(async () => window.dispatchEvent(new Event("blur")));
    expect(document.activeElement).toBe(control);
    expect(island.querySelector(":focus-visible")).toBe(control);
    expect(island.hasAttribute(keyboardFocus)).toBe(false);
    expect(surface("overlay").hasAttribute(peerHover)).toBe(false);

    hasFocus.mockReturnValue(true);
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(island.hasAttribute(keyboardFocus)).toBe(true);
    expect(surface("overlay").hasAttribute(peerHover)).toBe(share);

    await act(async () => control.blur());
    expect(island.hasAttribute(keyboardFocus)).toBe(false);
    expect(surface("overlay").hasAttribute(peerHover)).toBe(false);
  } finally {
    retainedFocus.mockRestore();
    hasFocus.mockRestore();
  }
});

it("keeps both surfaces shown between the separate leave and enter events in either direction", async () => {
  await act(async () => root.render(<><Surface kind="overlay" /><Surface kind="island" /></>));
  await move("overlay", "pointerenter");
  for (const [from, to] of [["overlay", "island"], ["island", "overlay"]] as const) {
    await move(from, "pointerleave");
    await tick(120);
    expect(shown(from) && shown(to)).toBe(true);
    await move(to, "pointerenter");
    await tick(50);
    expect(surface(from).hasAttribute(ownHover)).toBe(false);
    expect(surface(from).hasAttribute(peerHover)).toBe(true);
    expect(surface(to).hasAttribute(ownHover)).toBe(true);
    expect(surface(to).hasAttribute(peerHover)).toBe(false);
  }
  await move("overlay", "pointerleave");
  await tick(160);
  expect(shown("overlay") || shown("island")).toBe(false);
});

it("recovers existing hover when the peer registers later and clears it on unmount", async () => {
  await act(async () => root.render([<Surface key="overlay" kind="overlay" />]));
  await move("overlay", "pointerenter");
  await act(async () => root.render([
    <Surface key="overlay" kind="overlay" />, <Surface key="island" kind="island" />,
  ]));
  expect(surface("island").hasAttribute(peerHover)).toBe(true);
  await move("overlay", "pointerleave");
  await act(async () => root.render([<Surface key="island" kind="island" />]));
  expect(surface("island").hasAttribute(peerHover)).toBe(false);
  await tick(160);
  expect(shown("island")).toBe(false);
});

it("keeps native hover local and clears it immediately without a detached peer", async () => {
  await act(async () => root.render(<Surface kind="overlay" share={false} />));
  await act(async () => nativePointer.overlay?.(true));
  expect(surface("overlay").hasAttribute(ownHover)).toBe(true);
  await act(async () => window.dispatchEvent(new Event("blur")));
  expect(surface("overlay").hasAttribute(ownHover)).toBe(true);
  await act(async () => nativePointer.overlay?.(false));
  expect(surface("overlay").hasAttribute(ownHover)).toBe(false);
  expect(transport.listen).not.toHaveBeenCalled();
  expect(transport.emitTo).not.toHaveBeenCalled();
});

it("lets confirmed native departure cancel stale detached DOM hover", async () => {
  await act(async () => root.render(<><Surface kind="overlay" /><Surface kind="island" /></>));
  await move("island", "pointerenter");
  await act(async () => nativePointer.island?.(true));
  expect(surface("island").hasAttribute("data-overlay-pointer-tracked")).toBe(true);
  expect(shown("island") && shown("overlay")).toBe(true);

  // A detached WKWebView can resize and lose the pointer without delivering
  // its DOM pointerleave. The actual AppKit departure must still clear it.
  await act(async () => nativePointer.island?.(false));
  await tick(160);
  expect(shown("island") || shown("overlay")).toBe(false);
});
