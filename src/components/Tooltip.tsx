import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { tooltipPosition } from "./tooltipPosition";
import { OVERLAY_POINTER_TARGET_EVENT } from "../lib/overlayPointer";
import "./tooltip.css";

interface TooltipProps {
  label: string;
  popupClassName?: string;
  children: (descriptionId: string | undefined, hovered: boolean) => ReactNode;
}

/** Shared visible label for compact controls, including keyboard focus. */
export function Tooltip({ label, popupClassName, children }: TooltipProps) {
  const id = useId();
  const trigger = useRef<HTMLSpanElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const hovered = useRef(false);
  const dismissed = useRef(false);
  const pointerFocus = useRef(false);
  const [isHovered, setIsHovered] = useState(false);
  const [open, setOpen] = useState(false);

  const showOnHover = useCallback(() => {
    if (!hovered.current) {
      hovered.current = true;
      setIsHovered(true);
    }
    if (!dismissed.current) setOpen(true);
  }, []);
  const leaveHover = useCallback(() => {
    if (hovered.current) setIsHovered(false);
    hovered.current = false;
    dismissed.current = false;
    setOpen(false);
  }, []);
  const dismiss = () => {
    dismissed.current = true;
    setOpen(false);
  };

  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      const anchor = trigger.current;
      const tooltip = popup.current;
      if (!anchor || !tooltip) return;
      const next = tooltipPosition(
        anchor.getBoundingClientRect(),
        tooltip.getBoundingClientRect(),
        { width: window.innerWidth, height: window.innerHeight },
      );
      tooltip.style.left = `${next.left}px`;
      tooltip.style.top = `${next.top}px`;
      tooltip.style.visibility = "visible";
    };
    position();
    const observer = new ResizeObserver(position);
    if (trigger.current) observer.observe(trigger.current);
    if (popup.current) observer.observe(popup.current);
    window.addEventListener("resize", position);
    document.addEventListener("scroll", position, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", position);
      document.removeEventListener("scroll", position, true);
    };
  }, [open, label]);

  useEffect(() => {
    const nativeHover = (event: Event) => {
      const target = (event as CustomEvent<Element | null>).detail;
      if (target && trigger.current?.contains(target)) showOnHover();
      else leaveHover();
    };
    document.addEventListener(OVERLAY_POINTER_TARGET_EVENT, nativeHover);
    return () => document.removeEventListener(OVERLAY_POINTER_TARGET_EVENT, nativeHover);
  }, [showOnHover, leaveHover]);

  useEffect(() => {
    const useKeyboard = () => { pointerFocus.current = false; };
    const blurWindow = () => {
      leaveHover();
    };
    document.addEventListener("keydown", useKeyboard, true);
    window.addEventListener("blur", blurWindow);
    return () => {
      document.removeEventListener("keydown", useKeyboard, true);
      window.removeEventListener("blur", blurWindow);
    };
  }, [leaveHover]);

  return (
    <span
      ref={trigger}
      className="mimi-tooltip-trigger"
      onPointerEnter={(event) => {
        if (event.pointerType !== "touch") showOnHover();
      }}
      onPointerMove={(event) => {
        if (event.pointerType !== "touch") showOnHover();
      }}
      // Some nonactivating native WebViews deliver movement without a fresh
      // enter event. Movement is still an explicit hover, never a click.
      onMouseMove={showOnHover}
      onPointerLeave={leaveHover}
      onMouseLeave={leaveHover}
      onPointerDownCapture={() => {
        pointerFocus.current = true;
        dismiss();
      }}
      onClickCapture={dismiss}
      onFocus={(event) => {
        if (!pointerFocus.current && event.target.matches(":focus-visible")) {
          dismissed.current = false;
          setOpen(true);
        }
      }}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          pointerFocus.current = false;
          if (!hovered.current) setOpen(false);
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") dismiss();
      }}
    >
      {children(open ? id : undefined, isHovered)}
      {open && createPortal(
        <div ref={popup} id={id} className={popupClassName ? `mimi-tooltip ${popupClassName}` : "mimi-tooltip"} role="tooltip">{label}</div>,
        document.body,
      )}
    </span>
  );
}
