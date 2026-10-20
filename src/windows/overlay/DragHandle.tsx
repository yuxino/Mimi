import { Tooltip } from "../../components/Tooltip";
import { I18N } from "../../lib/i18n";
import { isTauri, overlayMoveStart } from "../../lib/ipc";

interface DragHandleProps {
  onToggleCollapsed: () => void;
  /** Compact variant uses the 42×30 collapsed-overlay drag area. */
  compact?: boolean;
  /** Expanded-handle width (default 120); narrowed on small windows so the
   * handle never overlaps the language capsule or the control buttons. */
  width?: number;
  disabled?: boolean;
  busy?: boolean;
  /** Keep movement available when a temporary error surface cannot collapse. */
  collapseDisabled?: boolean;
}

function beginOverlayDrag(
  event: React.MouseEvent<HTMLButtonElement>,
  { disabled, busy, collapseDisabled, onToggleCollapsed }: DragHandleProps,
) {
  if (disabled || busy || event.button !== 0) return;
  // Moving a nonactivating overlay must not focus its button target.
  event.preventDefault();
  if (event.detail === 2) {
    // The second press toggles instead of dragging, including in previews.
    if (!collapseDisabled) onToggleCollapsed();
    return;
  }
  if (!isTauri) return;
  // Record explicit user intent before the native window server owns the drag.
  // A following collapse or Space transition must retain that new origin.
  void overlayMoveStart().catch(() => {});
}

/** Pointer-only extension of the visible handle into the empty top band. */
export function OverlayDragArea(props: DragHandleProps) {
  return <button
    type="button"
    data-testid="overlay-drag-area"
    aria-hidden="true"
    tabIndex={-1}
    disabled={props.disabled || props.busy}
    onMouseDown={event => beginOverlayDrag(event, props)}
    style={{
      position: "absolute", inset: 0, width: "100%", height: "100%",
      border: 0, padding: 0, background: "transparent", pointerEvents: "auto",
      cursor: props.busy ? "progress" : props.disabled ? "default" : "pointer",
    }}
  />;
}

/**
 * The drag handle, mirroring `WindowDragArea`: a primary-button press drags
 * the overlay window (through the native drag command, regardless of which
 * child receives the press), and a double-click collapses or expands it.
 */
export function DragHandle({
  onToggleCollapsed,
  compact = false,
  width = 120,
  disabled = false,
  busy = false,
  collapseDisabled = false,
}: DragHandleProps) {
  const handleWidth = compact ? 42 : width;
  const height = compact ? 30 : 18;
  const blocked = disabled || busy;
  const toggleBlocked = blocked || collapseDisabled;

  return (
    <div className="flex shrink-0">
      <Tooltip label={collapseDisabled ? I18N.overlay.moveSubtitle : compact ? I18N.overlay.expandDragTooltip : I18N.overlay.dragTooltip}>
        {(descriptionId, hovered) => <button
          type="button"
          data-testid="drag-handle"
          aria-label={collapseDisabled ? I18N.overlay.moveSubtitle : compact ? I18N.overlay.expandSubtitle : I18N.overlay.collapseSubtitle}
          aria-describedby={descriptionId}
          aria-busy={busy || undefined}
          disabled={blocked}
          onMouseDown={event => beginOverlayDrag(event, { disabled, busy, collapseDisabled, onToggleCollapsed })}
          onKeyDown={(event) => {
            if (event.key !== "Enter" && event.key !== " ") return;
            // Cancel native activation even if a modifier is released before
            // Space: its keyup would otherwise synthesize an unmodified click.
            event.preventDefault();
            if (toggleBlocked) return;
            if (!event.repeat && !event.nativeEvent.isComposing && !event.altKey && !event.ctrlKey && !event.metaKey) onToggleCollapsed();
          }}
          onClick={(event) => {
            // Assistive activation emits a click without a pointer press.
            // Modified Space can also synthesize one after keyup.
            if (!toggleBlocked && event.detail === 0 && !event.altKey && !event.ctrlKey && !event.metaKey) onToggleCollapsed();
          }}
          className="relative flex items-center justify-center rounded-md focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
          style={{ width: handleWidth, height, border: 0, padding: 0, background: "transparent",
            cursor: busy ? "progress" : disabled ? "default" : "pointer", opacity: disabled && !busy ? 0.5 : 1 }}
        >
          <span aria-hidden="true" style={{
            width: hovered && !blocked ? 40 : 32,
            height: 3,
            borderRadius: 1.5,
            background: hovered && !blocked
              ? "rgba(255, 255, 255, 0.96)"
              : "rgba(255, 255, 255, 0.68)",
          }} />
        </button>}
      </Tooltip>
    </div>
  );
}
