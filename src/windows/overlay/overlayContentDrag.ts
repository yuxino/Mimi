import { isTauri, overlayMoveStart } from "../../lib/ipc";
import type { PointerEvent } from "react";

/** Mouse-drag subtitles and blank canvas without taking a control's gesture. */
export function startOverlayContentDrag(event: PointerEvent<HTMLElement>, disabled: boolean) {
  if (disabled || event.button !== 0 || event.pointerType !== "mouse" || event.isPrimary === false) return;
  const target = event.target;
  if (!(target instanceof Element) || !event.currentTarget.contains(target)) return;
  if (target.closest('button, a[href], input, select, textarea, label, summary, [role="button"], [role="combobox"], [role="switch"], [role="slider"], [role="menuitem"], [contenteditable]:not([contenteditable="false"])')) return;

  const timeline = target.closest<HTMLElement>(".overlay-timeline");
  if (timeline === target && timeline.scrollHeight > timeline.clientHeight) {
    // Mimi's overlay scrollbar is 6 CSS px; AppKit may overlay it without
    // subtracting its width from clientWidth. Keep that real scrollbar target.
    const bounds = timeline.getBoundingClientRect();
    const scrollbarWidth = Math.max(6, timeline.offsetWidth - timeline.clientWidth);
    if (event.clientX >= bounds.right - scrollbarWidth) return;
  }

  // Take the gesture before Timeline interprets pointer movement as reading
  // history. Wheel/trackpad scrolling and the scrollbar keep their own paths.
  event.preventDefault();
  event.stopPropagation();
  if (isTauri) void overlayMoveStart().catch(() => {});
}

