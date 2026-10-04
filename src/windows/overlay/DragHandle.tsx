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
}: DragHandleProps) {
  const handleWidth = compact ? 42 : width;
  const height = compact ? 30 : 18;
  const blocked = disabled || busy;

  const handleMouseDown = (event: React.MouseEvent<HTMLButtonElement>) => {
    if (blocked || event.button !== 0) return;
    // Moving a nonactivating overlay must not focus its new button target.
    event.preventDefault();
    if (event.detail === 2) {
      // The second press toggles instead of dragging. This also covers the
      // plain-Vite preview, where `startDragging` is a no-op.
      onToggleCollapsed();
      return;
    }
    if (!isTauri) return;
    // Record explicit user intent before AppKit takes over the native drag.
    // This prevents an immediately following collapse or Space transition
    // from confusing the new origin with programmatic presentation geometry.
    void overlayMoveStart().catch(() => {});
  };

  return (
    <div className="flex shrink-0">
      <Tooltip label={I18N.overlay.dragTooltip}>
        {(descriptionId, hovered) => <button
          type="button"
          data-testid="drag-handle"
          aria-label={compact ? I18N.overlay.expandSubtitle : I18N.overlay.collapseSubtitle}
          aria-describedby={descriptionId}
          aria-busy={busy || undefined}
          disabled={blocked}
          onMouseDown={handleMouseDown}
          onKeyDown={(event) => {
            if (blocked) return;
            if (event.key !== "Enter" && event.key !== " ") return;
            // Cancel native activation even if a modifier is released before
            // Space: its keyup would otherwise synthesize an unmodified click.
            event.preventDefault();
            if (!event.repeat && !event.nativeEvent.isComposing && !event.altKey && !event.ctrlKey && !event.metaKey) onToggleCollapsed();
          }}
          onClick={(event) => {
            // Assistive activation emits a click without a pointer press.
            // Modified Space can also synthesize one after keyup.
            if (!blocked && event.detail === 0 && !event.altKey && !event.ctrlKey && !event.metaKey) onToggleCollapsed();
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
              ? "rgba(122, 168, 255, 0.78)"
              : "rgba(255, 255, 255, 0.28)",
          }} />
        </button>}
      </Tooltip>
    </div>
  );
}
