import type { SessionStateEvent } from "../../lib/types";

interface OverlayTopChromeLayout {
  dragHandleCenterX: number;
  dragHandleWidth: number;
  showActions: boolean;
}

const DRAG_HANDLE_MIN_WIDTH = 40;
const DRAG_HANDLE_MAX_WIDTH = 120;
const CONTROL_ISLAND_RIGHT = 18 + 280;
const ACTION_ROW_RIGHT_MARGIN = 10;
const ACTION_BUTTON_WIDTH = 24;
const ACTION_BUTTON_GAP = 4;
const MAXIMUM_ACTION_COUNT = 6;
const CHROME_GAP = 6;

/** Error states retain their native language capsule and recovery actions. */
export function overlaySessionChromeLayout(
  width: number,
  session: Pick<SessionStateEvent, "isActive" | "status">,
) {
  const showControls = session.isActive || session.status.kind === "error";
  return {
    ...overlayTopChromeLayout(width, showControls),
    showControls,
    // The native capsule plus one primary action fit at the native minimum
    // width. Secondary actions remain available through the capsule panel.
    showPrimaryAction: showControls && width >= CONTROL_ISLAND_RIGHT + CHROME_GAP + ACTION_BUTTON_WIDTH + ACTION_ROW_RIGHT_MARGIN,
    topBandHeight: showControls ? 61 : 37,
  };
}

const MAXIMUM_ACTION_ROW_WIDTH =
  MAXIMUM_ACTION_COUNT * ACTION_BUTTON_WIDTH +
  (MAXIMUM_ACTION_COUNT - 1) * ACTION_BUTTON_GAP;

/**
 * Places the overlay's three independent top surfaces. When the full chrome
 * fits, the drag affordance stays visually centered on the subtitle window;
 * the native control island must not shift it to the right. Narrow overlays
 * temporarily yield the redundant actions. The handle sits below the native
 * capsule; the metadata row reserves its center so all three remain reachable.
 */
export function overlayTopChromeLayout(
  overlayWidth: number,
  isActive: boolean,
): OverlayTopChromeLayout {
  const width = Math.max(0, overlayWidth);
  if (!isActive) {
    return {
      dragHandleCenterX: width / 2,
      dragHandleWidth: clamp(
        width - 260,
        DRAG_HANDLE_MIN_WIDTH,
        DRAG_HANDLE_MAX_WIDTH,
      ),
      showActions: false,
    };
  }

  const actionRowLeft =
    width - ACTION_ROW_RIGHT_MARGIN - MAXIMUM_ACTION_ROW_WIDTH;
  const spaceWithActions =
    actionRowLeft - CONTROL_ISLAND_RIGHT - CHROME_GAP * 2;
  const showActions = spaceWithActions >= DRAG_HANDLE_MIN_WIDTH;
  const rightBoundary = showActions
    ? actionRowLeft
    : width - ACTION_ROW_RIGHT_MARGIN;
  const availableHandleWidth =
    rightBoundary - CONTROL_ISLAND_RIGHT - CHROME_GAP * 2;
  const dragHandleWidth = clamp(
    availableHandleWidth,
    DRAG_HANDLE_MIN_WIDTH,
    DRAG_HANDLE_MAX_WIDTH,
  );

  return {
    // The handle lives below the 30px native capsule, on the metadata row.
    // Keep it centered so that row can reserve equal space on either side.
    dragHandleCenterX: width / 2,
    dragHandleWidth,
    showActions,
  };
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}
