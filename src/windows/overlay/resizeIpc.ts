import { invoke } from "@tauri-apps/api/core";

type Point = {
  x: number;
  y: number;
};

interface Gesture {
  start: Point & { region: string };
  latestMove: Point | null;
  ended: boolean;
}

export interface NativeResizeGesture {
  move(x: number, y: number): void;
  finish(): void;
}

// Module state belongs to this WebView, not a component mount. A new set of
// handles must wait for the previous mount's native end acknowledgement.
let active: Gesture | null = null;
let pending: Gesture | null = null;
let running = false;
let needsEnd = false;
let recoveryRequested = false;

async function drain() {
  if (running) return;
  running = true;
  try {
    for (;;) {
      if (needsEnd) {
        if (!recoveryRequested) return;
        recoveryRequested = false;
        try {
          await invoke("resize_end");
          needsEnd = false;
        } catch {
          // A new gesture may request another recovery while this one is in
          // flight. Otherwise wait for a fresh gesture, without a retry loop.
          if (recoveryRequested && pending) continue;
          return;
        }
      }

      if (!active) {
        active = pending;
        pending = null;
        if (!active) return;
        try {
          await invoke("resize_start", active.start);
        } catch {
          active.ended = true;
          active.latestMove = null;
          active = null;
          needsEnd = true;
          recoveryRequested = true;
          continue;
        }
      }

      const gesture = active;
      const point = gesture.latestMove;
      if (point) {
        gesture.latestMove = null;
        try {
          await invoke("resize_move", point);
        } catch {
          gesture.ended = true;
          gesture.latestMove = null;
          active = null;
          needsEnd = true;
          recoveryRequested = true;
        }
        continue;
      }
      if (!gesture.ended) return;
      active = null;
      try {
        await invoke("resize_end");
      } catch {
        needsEnd = true;
        recoveryRequested = true;
      }
    }
  } finally {
    running = false;
  }
}

export function startNativeResize(region: string, x: number, y: number): NativeResizeGesture {
  // Keep only one waiting owner. A superseded or already released click must
  // never become a backlog of resize_start/end pairs after a slow IPC call.
  if (pending) {
    pending.ended = true;
    pending.latestMove = null;
  }
  if (active) active.ended = true;
  const gesture: Gesture = { start: { region, x, y }, latestMove: null, ended: false };
  pending = gesture;
  if (needsEnd) recoveryRequested = true;
  void drain();
  return {
    move(nextX, nextY) {
      if (gesture.ended) return;
      gesture.latestMove = { x: nextX, y: nextY };
      void drain();
    },
    finish() {
      if (gesture.ended) return;
      gesture.ended = true;
      if (pending === gesture) {
        pending = null;
        gesture.latestMove = null;
      }
      void drain();
    },
  };
}
