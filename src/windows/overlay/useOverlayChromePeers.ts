import { useCallback, useEffect, useRef, type RefObject } from "react";

type ChromeSurface = "overlay" | "island";

/** A detached language island and the subtitle surface share only whether
 * their top controls should be visible. Each keeps its own hit testing. */
export function useOverlayChromePeers(ref: RefObject<HTMLElement | null>, surface: ChromeSurface, enabled: boolean) {
  const nativeHover = useRef<(inside: boolean) => void>(() => {});
  useEffect(() => {
    const element = ref.current;
    if (!enabled || !element || typeof BroadcastChannel === "undefined") return;
    const fineHover = window.matchMedia("(hover: hover) and (pointer: fine)");
    const channel = new BroadcastChannel("mimi-overlay-chrome");
    let pointerInside = element.matches(":hover");
    let lastExposure: boolean | undefined;
    const publish = (force = false) => {
      const exposed = fineHover.matches && (pointerInside ||
        (document.hasFocus() && element.querySelector(":focus-visible") !== null));
      if (!force && exposed === lastExposure) return;
      lastExposure = exposed;
      channel.postMessage({ surface, exposed });
    };
    const setPointerInside = (inside: boolean) => {
      pointerInside = inside;
      publish();
    };
    const enter = (event: PointerEvent) => {
      if (event.pointerType !== "touch") setPointerInside(true);
    };
    const leave = () => setPointerInside(false);
    const refreshFocus = () => queueMicrotask(() => {
      if (nativeHover.current === setPointerInside) publish();
    });
    const receive = (event: MessageEvent<unknown>) => {
      const data = event.data;
      if (!data || typeof data !== "object" || !("surface" in data) ||
        data.surface !== (surface === "overlay" ? "island" : "overlay")) return;
      if ("request" in data && data.request === true) publish(true);
      else if ("exposed" in data && typeof data.exposed === "boolean") {
        element.toggleAttribute("data-overlay-peer-hover", data.exposed);
      }
    };
    nativeHover.current = setPointerInside;
    element.addEventListener("pointerenter", enter);
    // A WebView may retain browser hover while its window loses focus.
    // Movement reestablishes the shared state without waiting for reentry.
    element.addEventListener("pointermove", enter);
    element.addEventListener("pointerleave", leave);
    element.addEventListener("focusin", refreshFocus);
    element.addEventListener("focusout", refreshFocus);
    window.addEventListener("blur", leave);
    fineHover.addEventListener("change", refreshFocus);
    channel.addEventListener("message", receive);
    channel.postMessage({ surface, request: true });
    publish(true);
    return () => {
      nativeHover.current = () => {};
      channel.postMessage({ surface, exposed: false });
      channel.close();
      element.removeAttribute("data-overlay-peer-hover");
      element.removeEventListener("pointerenter", enter);
      element.removeEventListener("pointermove", enter);
      element.removeEventListener("pointerleave", leave);
      element.removeEventListener("focusin", refreshFocus);
      element.removeEventListener("focusout", refreshFocus);
      window.removeEventListener("blur", leave);
      fineHover.removeEventListener("change", refreshFocus);
    };
  }, [enabled, ref, surface]);
  return useCallback((inside: boolean) => nativeHover.current(inside), []);
}
