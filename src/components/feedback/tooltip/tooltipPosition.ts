interface TooltipRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
}

/** Keep labels inside the WebView: a portal cannot escape native bounds. */
export function tooltipPosition(
  trigger: TooltipRect,
  popup: Pick<TooltipRect, "width" | "height">,
  viewport: { width: number; height: number },
) {
  const margin = 6;
  const gap = 4;
  let left = trigger.left + (trigger.width - popup.width) / 2;
  let top = trigger.bottom + gap;
  if (top + popup.height > viewport.height - margin) {
    if (trigger.top - gap - popup.height >= margin) {
      top = trigger.top - gap - popup.height;
    } else if (trigger.left - gap - popup.width >= margin) {
      left = trigger.left - gap - popup.width;
      top = trigger.top + (trigger.height - popup.height) / 2;
    } else if (trigger.right + gap + popup.width <= viewport.width - margin) {
      left = trigger.right + gap;
      top = trigger.top + (trigger.height - popup.height) / 2;
    }
  }
  return {
    left: Math.max(margin, Math.min(left, viewport.width - popup.width - margin)),
    top: Math.max(margin, Math.min(top, viewport.height - popup.height - margin)),
  };
}
