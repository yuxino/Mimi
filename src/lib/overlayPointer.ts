import { isTauri, setOverlayPointerCursor, type OverlayPointerMotion } from "./ipc";

export const OVERLAY_POINTER_TARGET_EVENT = "mimi-overlay-pointer-target";
let lastTarget: Element | null | undefined;

/** Coalesce cursor intents, with one IPC in flight. Each native movement over
 * a control reasserts the hand: an ACK does not prevent WebKit resetting it.
 * A stationary successful or failed request never loops. */
export function createPointerCursorUpdater(send: typeof setOverlayPointerCursor) {
  let latest: { point: NonNullable<OverlayPointerMotion>; pointing: boolean } | null = null;
  let sequence = 0;
  let lifetime = 0;
  let accepted = false;
  let pending = false;
  const flush = () => {
    if (pending || !latest || (!latest.pointing && !accepted)) return;
    const intent = latest;
    const token = sequence;
    const generation = lifetime;
    pending = true;
    void send(intent.point, intent.pointing).then(result => {
      if (result && latest && lifetime === generation) accepted = intent.pointing;
    }).catch(() => {}).finally(() => {
      pending = false;
      if (sequence !== token) flush();
    });
  };
  return (point: OverlayPointerMotion, pointing = false) => {
    sequence += 1;
    latest = point === null ? null : { point, pointing };
    if (!latest) {
      lifetime += 1;
      accepted = false; // Native exit/hide/lock restores its cursor.
    }
    flush();
  };
}

const updateCursor = createPointerCursorUpdater(setOverlayPointerCursor);

export function isClickablePointerTarget(target: Element | null): boolean {
  const control = target?.closest('button, a[href], summary, select, [role="button"], [role="switch"], [role="option"], [role="tab"], input:is([type="button"], [type="submit"], [type="reset"], [type="checkbox"], [type="radio"], [type="color"])');
  return Boolean(control && !control.matches(":disabled") && !control.hasAttribute("disabled") && control.getAttribute("aria-disabled") !== "true"
    && control.getAttribute("aria-busy") !== "true");
}

/** One hit test per native movement; only target changes notify controls. */
export function publishOverlayPointerMotion(point: OverlayPointerMotion) {
  const valid = point !== null && Number.isFinite(point.x) && Number.isFinite(point.y)
    && point.x >= 0 && point.y >= 0 && point.x < window.innerWidth && point.y < window.innerHeight
    ? point : null;
  const target = valid ? document.elementFromPoint(valid.x, valid.y) : null;
  if (isTauri) updateCursor(valid, isClickablePointerTarget(target));
  if (lastTarget === target) return;
  lastTarget = target;
  document.dispatchEvent(new CustomEvent(OVERLAY_POINTER_TARGET_EVENT, { detail: target }));
}
