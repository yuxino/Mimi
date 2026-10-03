import { useLayoutEffect, useRef, type RefObject } from "react";
import { committedOverlayMeasurements, observeOverlayCommitted, subtitleCharacterCount } from "../../lib/developmentTrace";
import type { SessionStateEvent, SettingsSnapshot, SourceSubtitleSnapshot, SubtitleSnapshot } from "../../lib/types";
import { visibleLiveSubtitles, type LiveTail, type SubtitleBlock } from "./overlayModel";

interface Props {
  subtitleRootRef: RefObject<HTMLDivElement | null>;
  session: SessionStateEvent;
  settings: SettingsSnapshot;
  blocks: SubtitleBlock[];
  dual: boolean;
  primarySubtitles: SubtitleSnapshot;
  microphoneSubtitles: SourceSubtitleSnapshot;
  primaryTail: LiveTail;
  microphoneTail: LiveTail;
  atomicProvider: boolean;
  presentationCollapsed: boolean;
}

/** Local-development DOM evidence. Vite excludes this module in release builds. */
export function DevelopmentOverlayTrace({ subtitleRootRef, session, settings, blocks, dual,
  primarySubtitles, microphoneSubtitles, primaryTail, microphoneTail, atomicProvider, presentationCollapsed }: Props) {
  const debugReportRef = useRef<(() => void) | null>(null);
  const debugBlocksRef = useRef<SubtitleBlock[] | null>(null);
  const debugProjectionRevision = useRef(0);
  const debugEnabled = session.debugSnapshotId != null;

  // Observe the committed projection without making raw draft snapshots a
  // Timeline prop: its existing memo/stabilization behavior stays unchanged.
  useLayoutEffect(() => {
    if (!debugEnabled) {
      debugReportRef.current = null;
      debugBlocksRef.current = null;
      debugProjectionRevision.current = 0;
      return;
    }
    if (debugBlocksRef.current !== blocks) {
      debugBlocksRef.current = blocks;
      debugProjectionRevision.current += 1;
    }
    const inputs = dual
      ? [primarySubtitles, microphoneSubtitles]
      : [primarySubtitles];
    let selectedSourceCharacters = 0;
    let selectedTranslationCharacters = 0;
    for (const input of inputs) {
      const signals = dual ? input as SourceSubtitleSnapshot : session;
      const selected = visibleLiveSubtitles(input, settings, signals.detectedLanguage,
        signals.isTranslationPending, signals.isTranslationTimedOut,
        atomicProvider && input.previewPair !== undefined);
      for (const preview of selected) {
        if (preview.kind === "source") selectedSourceCharacters += subtitleCharacterCount(preview.text);
        else selectedTranslationCharacters += subtitleCharacterCount(preview.text);
      }
    }
    const details = {
      projection: presentationCollapsed ? "collapsed" as const : blocks.length === 0 ? "empty" as const
        : settings.subtitleDisplayMode === "bilingual" ? "dual" as const : settings.subtitleDisplayMode,
      selectedSourceCharacters, selectedTranslationCharacters,
      stableSourceCharacters: subtitleCharacterCount(primaryTail.source) +
        (dual ? subtitleCharacterCount(microphoneTail.source) : 0),
      stableTranslationCharacters: subtitleCharacterCount(primaryTail.translation) +
        (dual ? subtitleCharacterCount(microphoneTail.translation) : 0),
      projectionRevision: debugProjectionRevision.current,
    };
    const report = () => {
      const root = subtitleRootRef.current;
      if (root) observeOverlayCommitted(session, { ...details, ...committedOverlayMeasurements(root) });
    };
    debugReportRef.current = report;
    report();
  });

  useLayoutEffect(() => {
    const root = subtitleRootRef.current;
    if (!debugEnabled || root === null) return;
    let frame: number | null = null;
    const reportLayout = () => {
      if (frame !== null) return;
      frame = window.requestAnimationFrame(() => {
        frame = null;
        debugReportRef.current?.();
      });
    };
    // Scroll/layout can change visibility without a new backend snapshot.
    root.addEventListener("scroll", reportLayout, true);
    const resize = new ResizeObserver(reportLayout);
    resize.observe(root);
    const mutation = new MutationObserver(reportLayout);
    mutation.observe(root, { subtree: true, childList: true, characterData: true,
      attributes: true, attributeFilter: ["style"] });
    return () => {
      root.removeEventListener("scroll", reportLayout, true);
      resize.disconnect();
      mutation.disconnect();
      if (frame !== null) window.cancelAnimationFrame(frame);
    };
  }, [debugEnabled, subtitleRootRef]);

  return null;
}
