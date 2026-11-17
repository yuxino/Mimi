import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type MouseEvent, type PointerEvent, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { tooltipPosition } from "./tooltipPosition";
import { OVERLAY_POINTER_TARGET_EVENT } from "../lib/overlayPointer";
import "./tooltip.css";

interface TooltipProps {
  label: string;
  shortcut?: string;
  popupClassName?: string;
  /** An inline space for the label in a window too short to float it. */
  popupArea?: RefObject<HTMLElement | null>;
  children: (descriptionId: string | undefined, hovered: boolean) => ReactNode;
}

type HoverSurface = "trigger" | "popup";

/** Join only the facing edges, without covering nearby controls. */
function hoverBridge(anchor: DOMRect, left: number, top: number, width: number, height: number) {
  const sharedX = Math.max(anchor.left, left);
  const sharedY = Math.max(anchor.top, top);
  const gapX = Math.min(anchor.right, left + width);
  const gapY = Math.min(anchor.bottom, top + height);
  if (sharedX < gapX && sharedY > gapY) {
    return { left: sharedX, top: gapY, width: gapX - sharedX, height: sharedY - gapY };
  }
  if (sharedY < gapY && sharedX > gapX) {
    return { left: gapX, top: sharedY, width: sharedX - gapX, height: gapY - sharedY };
  }
}

/** Shared visible label for compact controls, including keyboard focus. */
export function Tooltip({ label, shortcut, popupClassName, popupArea, children }: TooltipProps) {
  const id = useId();
  const trigger = useRef<HTMLSpanElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const hovered = useRef<HoverSurface | null>(null);
  const dismissed = useRef(false);
  const pointerFocus = useRef(false);
  const keyboardFocus = useRef(false);
  const [isHovered, setIsHovered] = useState(false);
  const [open, setOpen] = useState(false);

  const showOnHover = useCallback((surface: HoverSurface) => {
    if (!hovered.current) setIsHovered(true);
    hovered.current = surface;
    if (!dismissed.current) setOpen(true);
  }, []);
  const leaveHover = useCallback(() => {
    if (!hovered.current) return;
    hovered.current = null;
    setIsHovered(false);
    if (!keyboardFocus.current) {
      dismissed.current = false;
      setOpen(false);
    }
  }, []);
  const dismiss = useCallback(() => {
    // Dismissing removes the popup, so its pointer leave handler cannot reset
    // the latch. Only hover on the surviving trigger needs to remain dismissed.
    const hoveredPopup = hovered.current === "popup";
    dismissed.current = !hoveredPopup;
    if (hoveredPopup) {
      hovered.current = null;
      setIsHovered(false);
    }
    setOpen(false);
  }, []);
  const hoverSurfaceForTarget = useCallback((target: EventTarget | null): HoverSurface | null => {
    if (!(target instanceof Node)) return null;
    if (trigger.current?.contains(target)) return "trigger";
    if (popup.current?.contains(target)) return "popup";
    return null;
  }, []);
  const trackHover = (event: MouseEvent) => {
    const surface = hoverSurfaceForTarget(event.target);
    if (surface) showOnHover(surface);
  };
  const trackPointerHover = (event: PointerEvent) => {
    if (event.pointerType !== "touch") trackHover(event);
  };
  const leaveSurface = (event: MouseEvent) => {
    const surface = hoverSurfaceForTarget(event.relatedTarget);
    if (!surface) leaveHover();
    else if (hovered.current) hovered.current = surface;
  };

  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      const anchor = trigger.current;
      const tooltip = popup.current;
      if (!anchor || !tooltip) return;
      const area = popupArea?.current?.getBoundingClientRect();
      tooltip.style.maxWidth = area ? `${area.width}px` : "";
      const rect = tooltip.getBoundingClientRect();
      const anchorRect = anchor.getBoundingClientRect();
      const next = area
        ? { left: area.right - rect.width, top: area.top + (area.height - rect.height) / 2 }
        : tooltipPosition(anchorRect, rect, { width: window.innerWidth, height: window.innerHeight });
      tooltip.style.left = `${next.left}px`;
      tooltip.style.top = `${next.top}px`;
      // The collapsed overlay shows a short label over its own drag area.
      tooltip.toggleAttribute("data-inline", Boolean(area));
      const bridge = area ? undefined : hoverBridge(anchorRect, next.left, next.top, rect.width, rect.height);
      tooltip.toggleAttribute("data-hover-bridge", Boolean(bridge));
      if (bridge) {
        tooltip.style.setProperty("--tooltip-bridge-origin", `${bridge.top}px auto auto ${bridge.left}px`);
        tooltip.style.setProperty("--tooltip-bridge-width", `${bridge.width}px`);
        tooltip.style.setProperty("--tooltip-bridge-height", `${bridge.height}px`);
      }
      tooltip.style.visibility = "visible";
    };
    position();
    const observer = new ResizeObserver(position);
    if (trigger.current) observer.observe(trigger.current);
    if (popup.current) observer.observe(popup.current);
    if (popupArea?.current) observer.observe(popupArea.current);
    window.addEventListener("resize", position);
    const scrolled = (event: Event) => {
      // A sibling subtitle timeline can scroll every animation frame without
      // moving this anchor. Only its scrolling ancestors change its position.
      if (event.target instanceof Element && !event.target.contains(trigger.current)) return;
      position();
    };
    window.addEventListener("scroll", scrolled, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", scrolled, true);
    };
  }, [open, label, shortcut, popupArea]);

  useEffect(() => {
    const nativeHover = (event: Event) => {
      const target = (event as CustomEvent<Element | null>).detail;
      const surface = hoverSurfaceForTarget(target);
      if (surface) showOnHover(surface);
      else leaveHover();
    };
    document.addEventListener(OVERLAY_POINTER_TARGET_EVENT, nativeHover);
    return () => document.removeEventListener(OVERLAY_POINTER_TARGET_EVENT, nativeHover);
  }, [hoverSurfaceForTarget, showOnHover, leaveHover]);

  useEffect(() => {
    const useKeyboard = () => { pointerFocus.current = false; };
    const blurWindow = () => {
      keyboardFocus.current = false;
      leaveHover();
      dismissed.current = false;
      setOpen(false);
    };
    document.addEventListener("keydown", useKeyboard, true);
    window.addEventListener("blur", blurWindow);
    return () => {
      document.removeEventListener("keydown", useKeyboard, true);
      window.removeEventListener("blur", blurWindow);
    };
  }, [leaveHover]);

  useEffect(() => {
    if (!open) return;
    const dismissFromElsewhere = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing || event.keyCode === 229 || event.defaultPrevented) return;
      event.preventDefault();
      event.stopPropagation();
      dismiss();
    };
    // Descendant controls handle Escape first. A hover-only tip can then close
    // before the document/window listener for its containing panel or dialog.
    document.body.addEventListener("keydown", dismissFromElsewhere);
    return () => document.body.removeEventListener("keydown", dismissFromElsewhere);
  }, [open, dismiss]);

  return (
    <span
      ref={trigger}
      className="mimi-tooltip-trigger"
      onPointerEnter={trackPointerHover}
      onPointerMove={trackPointerHover}
      // Some nonactivating native WebViews deliver movement without a fresh
      // enter event. Movement is still an explicit hover, never a click.
      onMouseMove={trackHover}
      onPointerLeave={leaveSurface}
      onMouseLeave={leaveSurface}
      onPointerDownCapture={(event) => {
        if (!trigger.current?.contains(event.target as Node)) return;
        pointerFocus.current = true;
        dismiss();
      }}
      onClickCapture={(event) => {
        if (trigger.current?.contains(event.target as Node)) dismiss();
      }}
      onFocus={(event) => {
        if (!pointerFocus.current && event.target.matches(":focus-visible")) {
          keyboardFocus.current = true;
          dismissed.current = false;
          setOpen(true);
        }
      }}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          pointerFocus.current = false;
          keyboardFocus.current = false;
          if (!hovered.current) {
            dismissed.current = false;
            setOpen(false);
          }
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open && !event.defaultPrevented && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) {
          event.preventDefault();
          event.stopPropagation();
          dismiss();
        }
      }}
    >
      {children(open ? id : undefined, isHovered)}
      {open && createPortal(
        <div ref={popup} id={id} className={popupClassName ? `mimi-tooltip ${popupClassName}` : "mimi-tooltip"}
          role="tooltip" onPointerEnter={trackPointerHover} onMouseEnter={trackHover}
          onPointerLeave={leaveSurface} onMouseLeave={leaveSurface}
          onPointerDown={(event) => event.preventDefault()}>{label}{shortcut && <> <kbd className="mimi-tooltip__shortcut">{shortcut}</kbd></>}</div>,
        document.body,
      )}
    </span>
  );
}
