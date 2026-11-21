// Disposable Windows UI_TEST metadata only. No subtitle strings or configuration.
type Method = "pointerDown" | "pointerMove" | "pointerUp" | "userIntent" | "endUserIntent" | "beginReading" | "readFromStart" | "followTail" | "followIfAtTail" | "scrolled" | "displayChanged" | "contentChanged" | "reflow" | "syncScrollPosition";
type EventKind = "pointerdown" | "pointermove" | "pointerup" | "pointercancel" | "mousedown" | "mousemove" | "mouseup" | "scroll" | "blur";
type Sample = {
  stage: "hello" | "ready" | "method" | "capture" | "react" | "busyBegin" | "busyEnd" | "limit";
  method: Method | null; edge: "enter" | "exit" | null; event: EventKind | null;
  following: boolean | null; userKind: "down" | "pointer" | "scrollbar" | null;
  intentTop: number | null; intentRange: number | null;
  pointerType: "mouse" | "pen" | "touch" | "other" | null;
  targetTimeline: boolean | null; gutter: boolean | null; trusted: boolean | null;
  x: number | null; y: number | null; button: number | null; buttons: number | null; primary: boolean | null;
  scrollTop: number | null; clientHeight: number | null; scrollHeight: number | null;
  offsetWidth: number | null; clientWidth: number | null; rect: number[] | null;
  footer: boolean; sequence: number; perfMs: number; wallMs: number;
};
const api = typeof window !== "undefined" ? (window as Window & {
  __TAURI_INTERNALS__?: { invoke: (command: string, args: unknown) => Promise<unknown> };
}).__TAURI_INTERNALS__ : undefined;
let enabled = false, sequence = 0, pending = Promise.resolve(), lastMove = 0;
let busy180Enabled = false, busy180Started = false, trustedScrollbarPress = false, scrollbarOwned = false;
function current(): HTMLElement | null {
  return typeof document === "undefined" ? null : document.querySelector<HTMLElement>(".overlay-timeline");
}
function sample(stage: Sample["stage"], element: HTMLElement | null, details: Partial<Sample> = {}): Sample {
  const b = element?.getBoundingClientRect();
  return {
    stage, method: null, edge: null, event: null, following: null, userKind: null, intentTop: null, intentRange: null,
    pointerType: null, targetTimeline: null, gutter: null, trusted: null, x: null, y: null, button: null, buttons: null, primary: null,
    scrollTop: element?.scrollTop ?? null, clientHeight: element?.clientHeight ?? null, scrollHeight: element?.scrollHeight ?? null,
    offsetWidth: element?.offsetWidth ?? null, clientWidth: element?.clientWidth ?? null,
    rect: b ? [b.left, b.top, b.width, b.height] : null,
    footer: typeof document !== "undefined" && document.querySelector(".overlay-return-to-live") !== null,
    sequence: ++sequence, perfMs: performance.now(), wallMs: Date.now(), ...details,
  };
}
function submit(value: Sample) {
  pending = pending.then(async () => { await api?.invoke("app_ui_test_frontend_ready", { historyProbe: value }); })
    .catch(() => { enabled = false; });
}
export function traceHistory(stage: "method" | "capture" | "react", element: HTMLElement | null, details: Partial<Sample>) {
  if (!enabled) return;
  if (sequence >= 2047) { enabled = false; submit(sample("limit", element)); return; }
  submit(sample(stage, element, details));
}
export function traceMethod(method: Method, edge: "enter" | "exit", following: boolean,
  intent: { kind: "down" | "pointer" | "scrollbar"; top: number; range?: number } | null,
  element = current()) {
  if (enabled && edge === "exit") {
    if (method === "pointerDown") scrollbarOwned = trustedScrollbarPress && intent?.kind === "scrollbar";
    if (method === "pointerUp" || method === "endUserIntent") scrollbarOwned = false;
  }
  traceHistory("method", element, { method, edge, following, userKind: intent?.kind ?? null,
    intentTop: intent?.top ?? null, intentRange: intent?.range ?? null });
}
export function traceReact(event: Event, element: HTMLElement) {
  if (!enabled) return;
  const p = event as PointerEvent;
  const pointer = event.type.startsWith("pointer");
  traceHistory("react", element, { event: event.type as EventKind, targetTimeline: event.target === element,
    pointerType: pointer ? (["mouse", "pen", "touch"].includes(p.pointerType) ? p.pointerType as "mouse" | "pen" | "touch" : "other") : null,
    x: Number.isFinite(p.clientX) ? p.clientX : null, y: Number.isFinite(p.clientY) ? p.clientY : null,
    button: Number.isFinite(p.button) ? p.button : null, buttons: Number.isFinite(p.buttons) ? p.buttons : null,
    primary: pointer ? p.isPrimary : null, trusted: event.isTrusted,
    gutter: Number.isFinite(p.clientX) ? p.clientX >= element.getBoundingClientRect().right - Math.max(6, element.offsetWidth - element.clientWidth) : null });
}
if (api) {
  // The backend rejects this unless the explicit isolated synthetic-history opt-in exists.
  void api.invoke("app_ui_test_frontend_ready", { historyProbe: sample("hello", current()) }).then(reply => {
    // Only the backend's strictly gated, fixed empty marker enables this one-shot experiment.
    busy180Enabled = reply === true;
    enabled = true;
    submit(sample("ready", current()));
    for (const kind of ["pointerdown", "pointermove", "pointerup", "pointercancel", "mousedown", "mousemove", "mouseup", "scroll", "blur"] as EventKind[]) {
      window.addEventListener(kind, event => {
        const element = current();
        if (!element) return;
        const e = event as MouseEvent;
        if (kind === "pointerdown") {
          const p = event as PointerEvent;
          trustedScrollbarPress = p.isTrusted && p.pointerType === "mouse" && p.button === 0 && p.buttons === 1 &&
            event.target === element && p.clientX >= element.getBoundingClientRect().right - Math.max(6, element.offsetWidth - element.clientWidth);
        }
        if (kind === "pointerup" || kind === "pointercancel" || kind === "blur") trustedScrollbarPress = false;
        if (kind.endsWith("move")) {
          if (!e.buttons || performance.now() - lastMove < 25) return;
          lastMove = performance.now();
        }
        traceHistory("capture", element, { event: kind, targetTimeline: event.target === element,
          trusted: event.isTrusted, x: Number.isFinite(e.clientX) ? e.clientX : null,
          y: Number.isFinite(e.clientY) ? e.clientY : null, buttons: Number.isFinite(e.buttons) ? e.buttons : null,
          button: Number.isFinite(e.button) ? e.button : null,
          pointerType: kind.startsWith("pointer") ? (["mouse", "pen", "touch"].includes((e as PointerEvent).pointerType) ? (e as PointerEvent).pointerType as "mouse" | "pen" | "touch" : "other") : null,
          primary: kind.startsWith("pointer") ? (e as PointerEvent).isPrimary : null,
          gutter: Number.isFinite(e.clientX) ? e.clientX >= element.getBoundingClientRect().right - Math.max(6, element.offsetWidth - element.clientWidth) : null,
        });
        const range = element.scrollHeight - element.clientHeight;
        const gap = range - element.scrollTop;
        if (kind === "scroll" && event.target === element && busy180Enabled && !busy180Started && scrollbarOwned &&
          gap >= 2 && gap <= 70) {
          busy180Started = true;
          submit(sample("busyBegin", element, { intentTop: element.scrollTop, intentRange: range, userKind: "scrollbar" }));
          const stop = performance.now() + 180;
          // Deliberate bounded main-thread work in this disposable opt-in QA build.
          // Do not write scrollTop, call React, dispatch input, or repeat the stall.
          while (performance.now() < stop) { /* timing stimulus only */ }
          submit(sample("busyEnd", element, { intentTop: element.scrollTop,
            intentRange: element.scrollHeight - element.clientHeight, userKind: "scrollbar" }));
        }
      }, true);
    }
  }).catch(() => {});
}
