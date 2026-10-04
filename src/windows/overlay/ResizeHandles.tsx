import { useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "../../lib/ipc";
import { OVERLAY_BASE_MINIMUM_HEIGHT } from "./overlayMinimumHeight";
import type {
  CSSProperties,
  PointerEvent as ReactPointerEvent,
} from "react";

type Region =
  | "top"
  | "left"
  | "bottom"
  | "right"
  | "topLeft"
  | "topRight"
  | "bottomLeft"
  | "bottomRight";

const OVERLAY_MIN_WIDTH = 360;
const OVERLAY_MAX_WIDTH = 1200;
const OVERLAY_MAX_HEIGHT = 600;

const CURSORS: Record<Region, string> = {
  top: "ns-resize",
  bottom: "ns-resize",
  left: "ew-resize",
  right: "ew-resize",
  topLeft: "nwse-resize",
  bottomRight: "nwse-resize",
  topRight: "nesw-resize",
  bottomLeft: "nesw-resize",
};

const HANDLES: Array<{ region: Region; style: CSSProperties }> = [
  { region: "topLeft", style: { left: 0, top: 0, width: 14, height: 14 } },
  { region: "topRight", style: { right: 0, top: 0, width: 14, height: 14 } },
  { region: "bottomLeft", style: { left: 0, bottom: 0, width: 14, height: 14 } },
  {
    region: "bottomRight",
    style: { right: 0, bottom: 0, width: 14, height: 14 },
  },
  { region: "top", style: { left: 14, top: 0, right: 14, height: 6 } },
  { region: "bottom", style: { left: 14, bottom: 0, right: 14, height: 6 } },
  { region: "left", style: { left: 0, top: 14, bottom: 14, width: 6 } },
  { region: "right", style: { right: 0, top: 14, bottom: 14, width: 6 } },
];

interface ResizeHandlesProps {
  disabled: boolean;
  minimumHeight?: number;
  /** `x`/`y` are the new window origin when the dragged edge/corner moves it. */
  onResize: (
    width: number,
    height: number,
    x?: number,
    y?: number,
  ) => void;
}

interface DragState {
  region: Region;
  startX: number;
  startY: number;
  startWidth: number;
  startHeight: number;
  /** Window origin in CSS (logical) pixels at drag start. */
  startWinX: number;
  startWinY: number;
}

/** Self-drawn resize handles for the overlay's eight edge/corner regions. */
export function ResizeHandles({ disabled, onResize, minimumHeight = OVERLAY_BASE_MINIMUM_HEIGHT }: ResizeHandlesProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<DragState | null>(null);
  // Tauri mode: the backend owns the resize math; the webview forwards pointer
  // positions. `active` gates move/end so a stale backend drag can never be
  // re-triggered by an unrelated hover, and `element`/`pointerId` remember who
  // owns the pointer so it can be released on the fallback path.
  const tauriDragRef = useRef<{
    active: boolean;
    element: HTMLElement | null;
    pointerId: number;
  }>({ active: false, element: null, pointerId: 0 });

  // Stable callback (only refs and module imports), so the one-time window
  // listeners below can add/remove it with a matching identity.
  const finishTauriDrag = useCallback(() => {
    const drag = tauriDragRef.current;
    if (!drag.active) return;
    drag.active = false;
    try {
      if (drag.element) drag.element.releasePointerCapture(drag.pointerId);
    } catch {
      // Pointer capture was already lost (e.g. window blur); nothing to do.
    }
    drag.element = null;
    void invoke("resize_end").catch(() => {});
  }, []);

  // Safety net: if pointer capture is lost mid-drag, or the window loses
  // focus, these finish the drag (idempotent via the active flag). Without
  // them a drag that escapes the 14px handle would leave the backend drag
  // state stuck and later hovers would keep resizing the window.
  useEffect(() => {
    window.addEventListener("pointerup", finishTauriDrag);
    window.addEventListener("pointercancel", finishTauriDrag);
    window.addEventListener("blur", finishTauriDrag);
    return () => {
      window.removeEventListener("pointerup", finishTauriDrag);
      window.removeEventListener("pointercancel", finishTauriDrag);
      window.removeEventListener("blur", finishTauriDrag);
    };
  }, [finishTauriDrag]);

  const start = (region: Region, event: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    if (isTauri) {
      // Send SCREEN coordinates, not clientX/Y: the backend moves the window
      // during the drag, so window-relative coordinates would shift under a
      // stationary cursor and feed back into the math (the overlay used to
      // oscillate between two frames on corner drags).
      void invoke("resize_start", {
        region,
        x: event.screenX,
        y: event.screenY,
      }).catch(() => {});
      const element = event.currentTarget;
      tauriDragRef.current = { active: true, element, pointerId: event.pointerId };
      try {
        element.setPointerCapture(event.pointerId);
      } catch {
        // Capture is an optimisation; the window-level listeners still
        // guarantee the drag ends even if capture is unavailable.
      }
      return;
    }
    const rect = rootRef.current?.getBoundingClientRect();
    if (!rect) return;
    dragRef.current = {
      region,
      startX: event.clientX,
      startY: event.clientY,
      startWidth: rect.width,
      startHeight: rect.height,
      startWinX: window.screenX,
      startWinY: window.screenY,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const move = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (isTauri) {
      if (!tauriDragRef.current.active) return;
      void invoke("resize_move", { x: event.screenX, y: event.screenY }).catch(
        () => {},
      );
      return;
    }
    const drag = dragRef.current;
    if (!drag) return;
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;
    const region = drag.region.toLowerCase();

    // Anchor the dragged edge/corner: edges that move also shift the window
    // origin (top/left grow upward/leftward), matching native window resize.
    let width = drag.startWidth;
    let height = drag.startHeight;
    let x = drag.startWinX;
    let y = drag.startWinY;
    if (region.includes("left")) {
      width = drag.startWidth - dx;
      x = drag.startWinX + dx;
    }
    if (region.includes("right")) {
      width = drag.startWidth + dx;
    }
    if (region.includes("top")) {
      height = drag.startHeight - dy;
      y = drag.startWinY + dy;
    }
    if (region.includes("bottom")) {
      height = drag.startHeight + dy;
    }

    const clampedW = clamp(Math.round(width), OVERLAY_MIN_WIDTH, OVERLAY_MAX_WIDTH);
    const clampedH = clamp(Math.round(height), minimumHeight, OVERLAY_MAX_HEIGHT);
    // Recede the dragged edge when the size got clamped, so dragging past the
    // minimum does not keep drifting the window off-screen.
    if (region.includes("left")) x += width - clampedW;
    if (region.includes("top")) y += height - clampedH;
    // Keep at least a sliver of the window on screen so it cannot get lost.
    const minVisible = 48;
    x = clamp(
      Math.round(x),
      -clampedW + minVisible,
      window.screen.width - minVisible,
    );
    y = clamp(Math.round(y), 0, Math.max(0, window.screen.availHeight - minVisible));
    onResize(clampedW, clampedH, x, y);
  };

  const end = () => {
    dragRef.current = null;
    if (isTauri) finishTauriDrag();
  };

  return (
    <div
      ref={rootRef}
      // The root must never swallow pointer events: it overlays the whole
      // window and would otherwise block the drag handle and the control
      // buttons beneath it. Only the eight edge handles stay interactive.
      className="absolute inset-0"
      style={{ pointerEvents: "none" }}
    >
      {HANDLES.map(({ region, style }) => (
        <div
          key={region}
          style={{
            position: "absolute",
            cursor: CURSORS[region],
            pointerEvents: disabled ? "none" : "auto",
            ...style,
          }}
          onPointerDown={(event) => start(region, event)}
          onPointerMove={move}
          onPointerUp={end}
          onPointerCancel={end}
        />
      ))}
    </div>
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
