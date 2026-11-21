import { emitTo, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, type RefObject } from "react";

type ChromeSurface = "overlay" | "island";
type HoverMessage = { kind: "ready" | "visibility"; visible: boolean };

const HOVER_EVENT = "overlay-chrome-hover";
const LEAVE_GRACE_MS = 160;
const WINDOW_LABEL: Record<ChromeSurface, string> = { overlay: "overlay", island: "overlay-control" };

/** Share top-control visibility without sharing hit targets or pointer coordinates. */
export function useOverlayChromePeers(
  ref: RefObject<HTMLElement | null>,
  surface: ChromeSurface,
  shareWithPeer: boolean,
) {
  const nativeHover = useRef<(inside: boolean) => void>(() => {});
  const outgoing = useRef<Promise<void> | undefined>(undefined);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    const fineHover = window.matchMedia("(hover: hover) and (pointer: fine)");
    const ownWindow = { kind: "WebviewWindow", label: WINDOW_LABEL[surface] } as const;
    const peerWindow = { kind: "WebviewWindow", label: WINDOW_LABEL[surface === "overlay" ? "island" : "overlay"] } as const;
    let domInside = element.matches(":hover");
    let nativeInside = false;
    let receivedNativeMotion = false;
    let pointerVisible = false;
    let leaveTimer: number | undefined;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    let lastShared: boolean | undefined;

    const send = (message: HoverMessage) => {
      outgoing.current = (outgoing.current ?? Promise.resolve())
        .then(() => emitTo(peerWindow, HOVER_EVENT, message)).catch(() => {});
    };

    const publish = (kind: HoverMessage["kind"] = "visibility", force = false) => {
      const keyboardVisible = document.hasFocus() && element.querySelector(":focus-visible:not(.overlay-timeline)") !== null;
      element.toggleAttribute("data-overlay-keyboard-focus", keyboardVisible);
      if (!unlisten) return;
      const visible = fineHover.matches && (pointerVisible || keyboardVisible);
      if (!force && visible === lastShared) return;
      lastShared = visible;
      send({ kind, visible });
    };

    const setPointerVisible = (visible: boolean) => {
      pointerVisible = visible;
      element.toggleAttribute("data-overlay-hover", visible);
      publish();
    };

    const cancelLeave = () => {
      window.clearTimeout(leaveTimer);
      leaveTimer = undefined;
    };

    const updatePointer = () => {
      const inside = nativeInside || (fineHover.matches && domInside);
      if (inside || !fineHover.matches || !shareWithPeer) {
        cancelLeave();
        setPointerVisible(inside);
      } else if (pointerVisible && leaveTimer === undefined) {
        // Both windows retain the old surface's exposure until its successor
        // has a chance to report entry. An actual departure still expires.
        leaveTimer = window.setTimeout(() => {
          leaveTimer = undefined;
          setPointerVisible(false);
        }, LEAVE_GRACE_MS);
      }
    };

    const setNativeInside = (inside: boolean) => {
      if (receivedNativeMotion && nativeInside === inside && (inside || !domInside)) return;
      if (!receivedNativeMotion) {
        receivedNativeMotion = true;
        element.setAttribute("data-overlay-pointer-tracked", "");
      }
      nativeInside = inside;
      // AppKit has verified departure even if a non-key WebView kept its
      // old :hover state or never delivered a DOM leave after resizing.
      if (!inside) domInside = false;
      updatePointer();
    };
    const enter = (event: PointerEvent) => {
      if (event.pointerType === "touch" || domInside) return;
      domInside = true;
      updatePointer();
    };
    const leave = (event: PointerEvent) => {
      if (event.pointerType === "touch" || !domInside) return;
      domInside = false;
      updatePointer();
    };
    // Read the focus state after the browser handles focus and input modality.
    // Physical pointer ownership is independent of window focus.
    const refreshFocus = () => queueMicrotask(() => {
      if (!disposed) publish();
    });
    const refreshInput = () => {
      updatePointer();
      publish();
    };

    nativeHover.current = setNativeInside;
    element.addEventListener("pointerenter", enter);
    element.addEventListener("pointermove", enter);
    element.addEventListener("pointerleave", leave);
    element.addEventListener("focusin", refreshFocus);
    element.addEventListener("focusout", refreshFocus);
    window.addEventListener("focus", refreshFocus);
    window.addEventListener("blur", refreshFocus);
    window.addEventListener("keydown", refreshFocus, true);
    window.addEventListener("pointerdown", refreshFocus, true);
    fineHover.addEventListener("change", refreshInput);
    updatePointer();

    if (shareWithPeer) {
      void listen<unknown>(HOVER_EVENT, ({ payload }) => {
        if (disposed || !payload || typeof payload !== "object" ||
          !("kind" in payload) || (payload.kind !== "visibility" && payload.kind !== "ready" && payload.kind !== "native-pointer") ||
          !("visible" in payload) || typeof payload.visible !== "boolean") return;
        if (payload.kind === "native-pointer") {
          // Windows supplies physical ownership when its inactive WebView
          // misses DOM entry. Attribute it locally, independently of the peer.
          setNativeInside(payload.visible);
          return;
        }
        element.toggleAttribute("data-overlay-peer-hover", payload.visible);
        if (payload.kind === "ready") publish("visibility", true);
      }, { target: ownWindow }).then((remove) => {
        if (disposed) {
          remove();
          return;
        }
        unlisten = remove;
        // A window that registers after its peer was already hovered asks
        // for the current state instead of waiting for a new pointer move.
        publish("ready", true);
      }).catch(() => {});
    }

    return () => {
      disposed = true;
      nativeHover.current = () => {};
      cancelLeave();
      if (unlisten) {
        if (lastShared) send({ kind: "visibility", visible: false });
        unlisten();
      }
      element.removeAttribute("data-overlay-pointer-tracked");
      element.removeAttribute("data-overlay-hover");
      element.removeAttribute("data-overlay-peer-hover");
      element.removeAttribute("data-overlay-keyboard-focus");
      element.removeEventListener("pointerenter", enter);
      element.removeEventListener("pointermove", enter);
      element.removeEventListener("pointerleave", leave);
      element.removeEventListener("focusin", refreshFocus);
      element.removeEventListener("focusout", refreshFocus);
      window.removeEventListener("focus", refreshFocus);
      window.removeEventListener("blur", refreshFocus);
      window.removeEventListener("keydown", refreshFocus, true);
      window.removeEventListener("pointerdown", refreshFocus, true);
      fineHover.removeEventListener("change", refreshInput);
    };
  }, [ref, shareWithPeer, surface]);

  return useCallback((inside: boolean) => nativeHover.current(inside), []);
}
