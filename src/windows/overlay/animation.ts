import { useEffect, useRef, useState } from "react";
import type { SettingsSnapshot } from "../../lib/types";

/** Keep the stabilizer's cached text inside the actually projected utterance.
 * A completed B pair must use its own owner even when raw ASR is already C.
 * Providers without stamps get a new identity after a confirmed pair. */
export function subtitleStreamKey(
  displayMode: SettingsSnapshot["subtitleDisplayMode"],
  lane: "source" | "translation",
  utteranceId: string | null | undefined,
  latestCommittedAt: number | null,
): string {
  return JSON.stringify([displayMode, lane, utteranceId == null
    ? ["history", latestCommittedAt]
    : ["utterance", utteranceId]]);
}

/** Tracks the user's reduced-motion preference reactively. */
export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(
    () =>
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );

  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onChange = (event: MediaQueryListEvent) => setReduced(event.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);

  return reduced;
}

/**
 * Resolves one animation switch. `null` means the user never touched it, so the
 * system's reduce-motion preference decides; an explicit value wins, which is
 * how the breathing light stays reachable on machines that disable animations
 * system-wide.
 */
export function resolveMotion(
  explicit: boolean | null,
  reduced: boolean,
): boolean {
  return explicit ?? !reduced;
}

/** Resolved switch value, reacting to both the stored choice and the system. */
export function useResolvedMotion(explicit: boolean | null): boolean {
  return resolveMotion(explicit, useReducedMotion());
}

/**
 * Stabilizes a streaming text value: the returned value only advances after
 * `settleMs` pass without a change, and even under a continuous stream it
 * advances at least every `maxWaitMs`. This turns per-character draft
 * churn into calm, chunked updates (the preview settles between speech
 * pauses instead of flickering on every recognition block).
 */
export function useStableText(
  text: string,
  settleMs = 400,
  maxWaitMs = 1500,
  streamKey = "",
): string {
  const [stable, setStable] = useState({ text, streamKey });
  const latestRef = useRef({ text, streamKey });
  const maxTimerRef = useRef<{ id: number; streamKey: string } | null>(null);

  // Seed an identity once, rather than returning every raw draft until its
  // first timer settles. Immediate removals/finals also replace the cache:
  // the next draft must not resurrect the text from before that boundary.
  const immediate = text === "" || settleMs === 0;
  if (streamKey !== stable.streamKey || (immediate && text !== stable.text)) {
    setStable({ text, streamKey });
  }

  // Keep the latest text available to the (non-resetting) force-sync timer.
  useEffect(() => {
    latestRef.current = { text, streamKey };
  }, [text, streamKey]);

  useEffect(() => {
    if (text === stable.text && streamKey === stable.streamKey) {
      // Already in sync: cancel any pending force-sync timer.
      if (maxTimerRef.current !== null) {
        window.clearTimeout(maxTimerRef.current.id);
        maxTimerRef.current = null;
      }
      return;
    }

    if (maxTimerRef.current !== null && maxTimerRef.current.streamKey !== streamKey) {
      window.clearTimeout(maxTimerRef.current.id);
      maxTimerRef.current = null;
    }

    // Settle timer: restarts on every change, so the value only advances
    // after a pause.
    const settleTimer = window.setTimeout(() => {
      if (latestRef.current.streamKey !== streamKey) return;
      if (maxTimerRef.current !== null) {
        window.clearTimeout(maxTimerRef.current.id);
        maxTimerRef.current = null;
      }
      setStable(latestRef.current);
    }, settleMs);

    // Force-sync timer: started once when the value first goes stale and
    // never reset by subsequent changes, so a continuous stream still
    // advances the preview at least every `maxWaitMs`.
    if (maxTimerRef.current === null) {
      const timer = { id: 0, streamKey };
      timer.id = window.setTimeout(() => {
        if (maxTimerRef.current !== timer) return;
        maxTimerRef.current = null;
        if (latestRef.current.streamKey === streamKey) setStable(latestRef.current);
      }, maxWaitMs);
      maxTimerRef.current = timer;
    }

    return () => window.clearTimeout(settleTimer);
  }, [text, streamKey, stable, settleMs, maxWaitMs]);

  // Unmount cleanup for the ref-held force-sync timer.
  useEffect(
    () => () => {
      if (maxTimerRef.current !== null) {
        window.clearTimeout(maxTimerRef.current.id);
      }
    },
    [],
  );

  // Removal, confirmation, and display-mode changes take effect during the
  // render itself. A previous source preview must not survive in the timer's
  // cached value when the next preview is a translation.
  return immediate || streamKey !== stable.streamKey
    ? text
    : stable.text;
}
