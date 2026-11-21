import type { SessionStateEvent } from "../../../lib/types";

export const OVERLAY_INSET = 6;

// The detached language capsule can take 156px plus its left inset. Keep
// the action strip clear even when the localized language name fills that space.
const CAPSULE_END = 18 + 156;
const TOOLBAR_RIGHT_INSET = OVERLAY_INSET + 1 + 10; // Canvas border and strip offset.
const CHROME_GAP = 8;
const BUTTON_WIDTH = 24;
const STRIP_EDGE_WIDTH = 6;
const BUTTON_GAP = 2;
const COLLAPSE_STRIP_WIDTH = STRIP_EDGE_WIDTH + BUTTON_WIDTH * 3 + BUTTON_GAP * 2;
const CLEAR_STRIP_WIDTH = COLLAPSE_STRIP_WIDTH + BUTTON_WIDTH + BUTTON_GAP;
const FULL_STRIP_WIDTH = CLEAR_STRIP_WIDTH + 2 * (BUTTON_WIDTH + BUTTON_GAP);
// Two compact timing readings, including spacing between their tabular values.
const TIMING_GROUP_WIDTH = 144;
export const TIMING_INSET_BESIDE_ACTIONS = 10 + FULL_STRIP_WIDTH + CHROME_GAP;

export function overlaySessionChromeLayout(width: number, session: Pick<SessionStateEvent, "isActive" | "status">) {
  const showControls = session.isActive || session.status.kind === "error";
  const actionSpace = width - CAPSULE_END - CHROME_GAP - TOOLBAR_RIGHT_INSET;
  return {
    showControls,
    showPrimaryAction: showControls && actionSpace >= STRIP_EDGE_WIDTH + BUTTON_WIDTH,
    showSessionAction: showControls && actionSpace >= STRIP_EDGE_WIDTH + BUTTON_WIDTH * 2 + BUTTON_GAP,
    showCollapseAction: showControls && actionSpace >= COLLAPSE_STRIP_WIDTH,
    showClearAction: showControls && actionSpace >= CLEAR_STRIP_WIDTH,
    showPresentationActions: showControls && actionSpace >= FULL_STRIP_WIDTH,
    timingsFitBesideActions: actionSpace >= FULL_STRIP_WIDTH + CHROME_GAP + TIMING_GROUP_WIDTH,
    topBandHeight: showControls ? 40 : 34,
  };
}
