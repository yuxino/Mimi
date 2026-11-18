import { isTauri, overlayMoveStart } from "../../../lib/ipc";

/** Move the native window from empty chrome without focusing or toggling it. */
export function OverlayDragArea({ disabled }: { disabled: boolean }) {
  return <button
    type="button"
    className="overlay-drag-target"
    data-testid="overlay-drag-area"
    aria-hidden="true"
    tabIndex={-1}
    disabled={disabled}
    onMouseDown={event => {
      if (disabled || event.button !== 0) return;
      event.preventDefault();
      if (isTauri) void overlayMoveStart().catch(() => {});
    }}
  />;
}
