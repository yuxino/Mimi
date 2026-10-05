import { useMemo } from "react";
import type { SessionStateEvent, SettingsSnapshot, SubtitleSnapshot } from "../../lib/types";
import { subtitleStreamKey, useStableText } from "./animation";
import { visibleLiveSubtitles, type LiveTail } from "./overlayModel";
import { formatGeminiTranscriptForDisplay } from "./geminiTranscriptDisplay";

type TranslationSignals = Pick<SessionStateEvent, "detectedLanguage" | "isTranslationPending" | "isTranslationTimedOut">;

/** Every input has its own stabilization state; another input cannot replace it. */
export function useSubtitleTail(subtitles: SubtitleSnapshot, settings: SettingsSnapshot,
  signals: TranslationSignals, running: boolean, atomicProvider: boolean, sourceIdentity: string,
  streamingProvider = false): LiveTail {
  const previews = useMemo(() => visibleLiveSubtitles(subtitles, settings, signals.detectedLanguage,
    signals.isTranslationPending, signals.isTranslationTimedOut,
    atomicProvider && subtitles.previewPair !== undefined),
  [subtitles, settings, signals.detectedLanguage, signals.isTranslationPending, signals.isTranslationTimedOut, atomicProvider]);
  const source = previews.find(preview => preview.kind === "source");
  const translation = previews.find(preview => preview.kind === "translation");
  const latestCommittedAt = subtitles.history.at(-1)?.createdAt ?? null;
  const streamIdentity = `${sourceIdentity}:${streamingProvider}`;
  const displaySource = streamingProvider ? formatGeminiTranscriptForDisplay(source?.text ?? "") : source?.text ?? "";
  const displayTranslation = streamingProvider ? formatGeminiTranscriptForDisplay(translation?.text ?? "") : translation?.text ?? "";
  const sourceText = useStableText(displaySource, source === undefined || source.isFinal || source.isStable ? 0 : 180, 750,
    `${streamIdentity}:${subtitleStreamKey(settings.subtitleDisplayMode, "source", source?.utteranceId, latestCommittedAt)}`);
  // Continuous output transcription is already translated. Keep a short
  // coalescing window with a non-resetting maximum, independent of source
  // activity and the backend's durable confirmation heuristic.
  const translationText = useStableText(displayTranslation, translation === undefined || translation.isFinal || translation.isStable ? 0 : streamingProvider ? 100 : 400,
    streamingProvider ? 250 : 1_500,
    `${streamIdentity}:${subtitleStreamKey(settings.subtitleDisplayMode, "translation", translation?.utteranceId, latestCommittedAt)}`);
  const isStreaming = running && ((source !== undefined && !source.isFinal && !source.isStable) ||
    (translation !== undefined && !translation.isFinal && !translation.isStable));
  return useMemo(() => ({ source: sourceText || null, translation: translationText || null,
    utteranceId: source?.utteranceId ?? translation?.utteranceId, isStreaming }),
  [sourceText, translationText, source?.utteranceId, translation?.utteranceId, isStreaming]);
}
