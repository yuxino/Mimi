import type { SessionStateEvent } from "../../lib/types";

// The detached language capsule can take 280px plus its left inset. Keep
// the action strip clear even when the localized language name fills that space.
const CAPSULE_END = 18 + 280;
const TOOLBAR_RIGHT_INSET = 11;
const CHROME_GAP = 8;
const BUTTON_WIDTH = 24;
const STRIP_EDGE_WIDTH = 6;
const BUTTON_GAP = 2;
const SEPARATOR_WIDTH = 5;
const FULL_STRIP_WIDTH = STRIP_EDGE_WIDTH + BUTTON_WIDTH * 7 + BUTTON_GAP * 8 + SEPARATOR_WIDTH * 2;

export function overlaySessionChromeLayout(width: number, session: Pick<SessionStateEvent, "isActive" | "status">) {
  const showControls = session.isActive || session.status.kind === "error";
  const actionSpace = width - CAPSULE_END - CHROME_GAP - TOOLBAR_RIGHT_INSET;
  return {
    showControls,
    showPrimaryAction: showControls && actionSpace >= STRIP_EDGE_WIDTH + BUTTON_WIDTH,
    showSessionAction: showControls && actionSpace >= STRIP_EDGE_WIDTH + BUTTON_WIDTH * 2 + BUTTON_GAP,
    showActions: showControls && actionSpace >= FULL_STRIP_WIDTH,
    topBandHeight: showControls ? 56 : 34,
  };
}
