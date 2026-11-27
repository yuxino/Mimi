import { isTauri, overlayMoveStart } from "../../../lib/ipc";
import type { PointerEvent } from "react";

export function hitsVerticalScrollbar(element: HTMLElement, clientX: number, clientY?: number) {
  if (element.scrollHeight <= element.clientHeight) return false;
  // AppKit can overlay Mimi's 6px scrollbar without shrinking clientWidth.
  const width = Math.max(6, element.offsetWidth - element.clientWidth);
  const { right, top, bottom } = element.getBoundingClientRect();
  return clientX >= right - width && clientX < right &&
    (clientY === undefined || (clientY >= top && clientY < bottom));
}

/** Move from subtitle content or panel labels without taking a control's gesture. */
export function startOverlayContentDrag(event: PointerEvent<HTMLElement>, disabled: boolean) {
  if (disabled || event.button !== 0 || event.pointerType !== "mouse" || event.isPrimary === false) return;
  const target = event.target;
  if (!(target instanceof Element) || !event.currentTarget.contains(target)) return;
  if (target.closest('button, a[href], input, select, textarea, label, summary, [role="button"], [role="combobox"], [role="switch"], [role="slider"], [role="menuitem"], [role="listbox"], [role="tooltip"], [contenteditable]:not([contenteditable="false"]), .overlay-opacity-slider')) return;

  const scrollArea = target.closest<HTMLElement>(".overlay-timeline, .overlay-control-panel");
  if (scrollArea === target && hitsVerticalScrollbar(scrollArea, event.clientX)) return;

  // Take the gesture before Timeline interprets pointer movement as reading
  // history. Wheel/trackpad scrolling and the scrollbar keep their own paths.
  event.preventDefault();
  event.stopPropagation();
  if (isTauri) void overlayMoveStart().catch(() => {});
}
