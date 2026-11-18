import { isTauri, overlayMoveStart } from "../../lib/ipc";
import type { PointerEvent } from "react";

/** Move from subtitle content or panel labels without taking a control's gesture. */
export function startOverlayContentDrag(event: PointerEvent<HTMLElement>, disabled: boolean) {
  if (disabled || event.button !== 0 || event.pointerType !== "mouse" || event.isPrimary === false) return;
  const target = event.target;
  if (!(target instanceof Element) || !event.currentTarget.contains(target)) return;
  if (target.closest('button, a[href], input, select, textarea, label, summary, [role="button"], [role="combobox"], [role="switch"], [role="slider"], [role="menuitem"], [role="listbox"], [role="tooltip"], [contenteditable]:not([contenteditable="false"])')) return;

  const scrollArea = target.closest<HTMLElement>(".overlay-timeline, .overlay-control-panel");
  if (scrollArea === target && scrollArea.scrollHeight > scrollArea.clientHeight) {
    // Mimi's overlay scrollbar is 6 CSS px; AppKit may overlay it without
    // subtracting its width from clientWidth. Keep that real scrollbar target.
    const bounds = scrollArea.getBoundingClientRect();
    const scrollbarWidth = Math.max(6, scrollArea.offsetWidth - scrollArea.clientWidth);
    if (event.clientX >= bounds.right - scrollbarWidth) return;
  }

  // Take the gesture before Timeline interprets pointer movement as reading
  // history. Wheel/trackpad scrolling and the scrollbar keep their own paths.
  event.preventDefault();
  event.stopPropagation();
  if (isTauri) void overlayMoveStart().catch(() => {});
}
