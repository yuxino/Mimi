import { speechLanguageGuidance, targetLanguageOptionLabel } from "../../lib/speechLanguageGuidance";
import { credentialErrorMessage } from "../../lib/connectionDiagnostics";
/**
 * Pure derived state for the subtitle overlay. Keeping these transformations
 * outside React makes the phase and row logic deterministic and testable.
 */

import type {
  AudioSource,
  OverlayActivityPhaseKind,
  SessionStateEvent,
  ServiceProvider,
  SettingsSnapshot,
  SubtitleSnapshot,
} from "../../lib/types";
import { I18N } from "../../lib/i18n";
import {
  SOURCE_LANGUAGE_DISPLAY_NAMES,
  sourceLanguageStatusDisplayName,
} from "../../lib/types";

const ATOMIC_SUBTITLE_PROVIDERS = [
  "alibabaCloud", "deepLX", "customDashScopeASR", "customOpenAIASR", "appleSpeech", "windowsLiveCaptions",
] as const satisfies readonly ServiceProvider[];
type AtomicSubtitleProvider = typeof ATOMIC_SUBTITLE_PROVIDERS[number];

/** Mirrors the current TranslationClient factory's ASR + text-translation
 * routes, which publish complete PreviewPairs. Independent realtime
 * transports keep their own draft semantics; unknown replay routes are safe. */
export function usesAtomicSubtitlePreview(provider: unknown): provider is AtomicSubtitleProvider {
  return ATOMIC_SUBTITLE_PROVIDERS.some(candidate => candidate === provider);
}

/** Gemini transcribes a continuous stream; coalesce its live display briefly. */
export function usesStreamingSubtitlePreview(provider: unknown): provider is "googleGeminiLive" {
  return provider === "googleGeminiLive";
}

export type SubtitleBlockPresentation = "history" | "latestCommitted" | "live";

/** One spoken utterance: the unit the timeline groups, spaces and fades. */
export interface SubtitleBlock {
  audioSource?: AudioSource;
  id: string;
  /** Epoch ms of the committed utterance; `null` while it is still live. */
  createdAt: number | null;
  /** Where this block sits in the live/history progression. */
  presentation: SubtitleBlockPresentation;
  /** Recognized original; `null` when the display mode omits this lane. */
  source: string | null;
  /** Translation; `null` while it has not arrived or the mode omits it. */
  translation: string | null;
  /** Set on the live block while its lane may still change. */
  streaming?: true;
}

/** The live tail the overlay should render below the committed blocks. */
export interface LiveTail {
  source: string | null;
  translation: string | null;
  /** True while a lane is still streaming and may change again. */
  isStreaming: boolean;
  /** Actual preview owner when supplied; never guessed from matching text. */
  utteranceId?: string | null;
}

function isSameLanguageMode(
  settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage">,
  detectedLanguage: string | null,
): boolean {
  if (settings.targetLanguage === "original") return true;
  // This is a bilingual direction mode, even when source and target codes match.
  if (settings.targetLanguage === "zh_en") return false;
  if (detectedLanguage !== null) {
    return detectedLanguage === settings.targetLanguage;
  }
  return (
    settings.sourceLanguage !== "auto" &&
    settings.sourceLanguage === settings.targetLanguage
  );
}

export function isWaitingForFinalTranslation(
  settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage">,
  detectedLanguage: string | null,
  isTranslationPending: boolean,
): boolean {
  if (isSameLanguageMode(settings, detectedLanguage)) return false;
  return isTranslationPending;
}

export function pendingSourceTranslation(subtitles: SubtitleSnapshot,
  settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage">): boolean | undefined {
  return subtitles.tracks?.length ? subtitles.tracks.some(track =>
    isWaitingForFinalTranslation(settings, track.detectedLanguage, track.isTranslationPending)) : undefined;
}

export function computeActivityPhase(
  session: SessionStateEvent,
  settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage">,
): OverlayActivityPhaseKind {
  const source = session.subtitles.source;
  return computeActivityPhaseFromSignals(
    {
      statusKind: session.status.kind,
      isPaused: session.isPaused,
      detectedLanguage: session.detectedLanguage,
      isTranslationPending: session.isTranslationPending,
      sourceTranslationPending: pendingSourceTranslation(session.subtitles, settings),
      isTranslationPreviewPending: session.isTranslationPreviewPending,
      hasRecognizingSourceDraft: (source.text !== "" && !source.isFinal) ||
        (session.subtitles.tracks?.some(track => track.source.text !== "" && !track.source.isFinal) ?? false),
    },
    settings,
  );
}

interface ActivityPhaseSignals {
  statusKind: SessionStateEvent["status"]["kind"];
  isPaused: boolean;
  detectedLanguage: string | null;
  isTranslationPending: boolean;
  isTranslationPreviewPending?: boolean;
  sourceTranslationPending?: boolean;
  hasRecognizingSourceDraft: boolean;
}

export function computeActivityPhaseFromSignals(
  signals: ActivityPhaseSignals,
  settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage">,
): OverlayActivityPhaseKind {
  if (signals.statusKind === "error") return "error";
  if (signals.statusKind === "idle") return "idle";
  if (signals.isPaused) return "paused";

  switch (signals.statusKind) {
    case "connecting":
    case "stopping":
      return "connecting";
    case "listening": {
      if (signals.isTranslationPreviewPending ||
        (signals.sourceTranslationPending ?? isWaitingForFinalTranslation(
          settings,
          signals.detectedLanguage,
          signals.isTranslationPending,
        ))
      ) {
        return "translating";
      }
      if (signals.hasRecognizingSourceDraft) {
        return "recognizing";
      }
      return "listening";
    }
  }
}

export function emptyStateText(
  session: SessionStateEvent,
  settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage">,
): string {
  if (session.isPaused) return I18N.overlay.paused;

  switch (session.status.kind) {
    case "connecting":
      return I18N.overlay.connecting;
    case "listening":
      return (pendingSourceTranslation(session.subtitles, settings) ?? isWaitingForFinalTranslation(
        settings,
        session.detectedLanguage,
        session.isTranslationPending,
      ))
        ? I18N.overlay.translatingEmpty
        : I18N.overlay.listeningEmpty;
    case "stopping":
      return I18N.overlay.stopping;
    case "error":
      return credentialErrorMessage(session.status.message) ?? session.status.message;
    case "idle":
      return I18N.overlay.idle;
  }
}

export function emptyStateIsError(session: SessionStateEvent): boolean {
  return session.status.kind === "error";
}

type EmptyStateDensity = "minimal" | "compact" | "comfortable";

/**
 * Empty-state chrome adapts to the freely resized overlay height. At the
 * 100px native minimum only one status line fits below the control band, so
 * the decorative pulse yields to the text instead of being clipped.
 */
export function emptyStateDensity(overlayHeight: number): EmptyStateDensity {
  if (overlayHeight <= 112) return "minimal";
  if (overlayHeight < 176) return "compact";
  return "comfortable";
}

export function timelineClassName(blendsWithBackground: boolean): string {
  return [
    "min-h-0 flex-1 overflow-y-auto",
    blendsWithBackground ? "overlay-timeline--immersive" : "",
  ]
    .filter(Boolean)
    .join(" ");
}

/**
 * Visual-line budget while following live subtitles, including confirmed
 * history until the user scrolls up to read it. Use the actual window height;
 * a large window must not keep the same two-line limit as a narrow strip.
 * A short lane gives its spare space to the other language. Full confirmed
 * text remains available when reading history.
 */
export const SUBTITLE_LINE_HEIGHT = 1.32;
export const SUBTITLE_SOURCE_SCALE = 0.9;

export function subtitleSourceScale(availableLaneHeight: number | null): number {
  return availableLaneHeight !== null && availableLaneHeight < 78 ? 0.82 : SUBTITLE_SOURCE_SCALE;
}

export function subtitleLaneBudget(
  displayMode: SettingsSnapshot["subtitleDisplayMode"],
  hasTranslation: boolean,
  availableLaneHeight: number | null = null,
  fontSize = 18,
  measured: { source: number; translation: number } | null = null,
  sourceScale = subtitleSourceScale(availableLaneHeight),
): { source: number; translation: number } {
  const sourceLine = Math.ceil((displayMode === "bilingual"
    ? Math.max(12, fontSize * sourceScale) : fontSize) * SUBTITLE_LINE_HEIGHT);
  const translationLine = Math.ceil(fontSize * SUBTITLE_LINE_HEIGHT);
  const linesThatFit = (lineHeight: number, remaining = availableLaneHeight) =>
    remaining === null ? 2 : Math.max(1, Math.floor(remaining / lineHeight));
  switch (displayMode) {
    case "translation":
      return { source: 0, translation: linesThatFit(translationLine) };
    case "original":
      return { source: linesThatFit(sourceLine), translation: 0 };
    default: {
      if (!hasTranslation) return { source: linesThatFit(sourceLine), translation: 0 };
      if (availableLaneHeight === null) return { source: 1, translation: 2 };
      // Long bilingual lanes start with equal height rather than exposing
      // much less original text. Measured short text still yields its space.
      let source = linesThatFit(sourceLine, availableLaneHeight * 0.5);
      if (measured && measured.source > 0) {
        source = Math.min(source, Math.max(1, Math.ceil(measured.source / sourceLine)));
      }
      let translation = linesThatFit(translationLine, availableLaneHeight - source * sourceLine);
      if (measured && measured.translation > 0) {
        translation = Math.min(translation, Math.max(1, Math.ceil(measured.translation / translationLine)));
        source = linesThatFit(sourceLine, availableLaneHeight - translation * translationLine);
        if (measured.source > 0) source = Math.min(source, Math.max(1, Math.ceil(measured.source / sourceLine)));
      }
      return { source, translation };
    }
  }
}

/**
 * Groups committed pairs and the live tail into the sentence blocks the
 * timeline renders. The block — not an individual text row — carries the
 * timestamp, the age fade and the spacing, so a long sentence that wraps over
 * several lines keeps one visual level instead of reading as older subtitles.
 *
 * Lane selection follows the display mode: translation-only keeps just the
 * translation, original-only just the recognition, and bilingual hides a
 * translation that only repeats its original (same-language sessions). The
 * newest committed block is marked `latestCommitted` until a live tail exists,
 * which is what lets the live presentation stay compact without a separate
 * lifecycle state.
 */
export function buildSubtitleBlocks(
  history: SubtitleSnapshot["history"],
  displayMode: SettingsSnapshot["subtitleDisplayMode"],
  liveTail: LiveTail | null = null,
): SubtitleBlock[] {
  const blocks: SubtitleBlock[] = [];

  for (const pair of history) {
    const sameText = pair.source.trim() === pair.translation.trim();
    const source = displayMode === "translation" || pair.source.trim() === "" ? null : pair.source;
    const translation =
      displayMode === "original" || (source !== null && sameText) || pair.translation.trim() === ""
        ? null
        : pair.translation;
    if (source === null && translation === null) continue;
    blocks.push({
      id: `${pair.audioSource ? `${pair.audioSource}:` : ""}history-${pair.createdAt}`,
      ...(pair.audioSource ? { audioSource: pair.audioSource } : {}),
      createdAt: pair.createdAt,
      presentation: "history",
      source,
      translation,
    });
  }

  const isEmptyTail =
    liveTail === null || (liveTail.source === null && liveTail.translation === null);
  if (isEmptyTail) {
    const newest = blocks[blocks.length - 1];
    if (newest !== undefined) newest.presentation = "latestCommitted";
    return blocks;
  }

  // An identified B remains the same row if a delayed final A inserts above
  // it. Legacy snapshots lack this owner: keep the canonical history epoch so
  // a new B cannot inherit A's former live reading anchor.
  const latestConfirmedAt = history.at(-1)?.createdAt;
  blocks.push({
    id: liveTail.utteranceId ? `live-utterance-${liveTail.utteranceId}`
      : latestConfirmedAt === undefined ? "live" : `live-after-history-${latestConfirmedAt}`,
    createdAt: null,
    presentation: "live",
    source: liveTail.source,
    translation: liveTail.translation,
    ...(liveTail.isStreaming ? { streaming: true as const } : {}),
  });
  return blocks;
}

/** Keep both live tails independent while sharing one chronological history. */
export function buildMultiSourceSubtitleBlocks(history: SubtitleSnapshot["history"],
  displayMode: SettingsSnapshot["subtitleDisplayMode"],
  tails: { audioSource: AudioSource; history: SubtitleSnapshot["history"]; tail: LiveTail }[]): SubtitleBlock[] {
  const committed = buildSubtitleBlocks(history, displayMode).map(block => ({ ...block, audioSource: block.audioSource ?? "system" as const }));
  const live = tails.flatMap(({ audioSource, history: sourceHistory, tail }) => {
    const block = buildSubtitleBlocks(sourceHistory, displayMode, tail).find(block => block.presentation === "live");
    return block ? [{ ...block, id: `${audioSource}:${block.id}`, audioSource }] : [];
  });
  if (live.length && committed.length) committed[committed.length - 1].presentation = "history";
  return [...committed, ...live];
}

/**
 * The live preview line: the current unconfirmed translation (or a just-final
 * line that has not yet entered history). The overlay renders it as the
 * timeline's last row so streaming updates stay in the same reading flow.
 * Returns `null` when
 * there is nothing to preview.
 */
function visibleDraft(
  translation: SubtitleSnapshot["translation"],
  history: SubtitleSnapshot["history"],
): { text: string; isFinal: boolean; utteranceId?: string | null } | null {
  if (translation.text === "") return null;
  const currentIsAlreadyInHistory =
    translation.isFinal &&
    history[history.length - 1]?.translation === translation.text;
  if (currentIsAlreadyInHistory) return null;
  return { text: translation.text, isFinal: translation.isFinal,
    ...(translation.utteranceId == null ? {} : { utteranceId: translation.utteranceId }) };
}

interface LiveSubtitlePreview {
  text: string;
  isFinal: boolean;
  kind: "translation" | "source";
  isStable?: true;
  utteranceId?: string | null;
}

/**
 * Selects the active subtitle tail for the requested display language. A
 * delayed, empty, or timed-out translation must not flash the source in its
 * place. Recognition is display text only in Original or same-language mode.
 */
export function visibleLiveSubtitle(
  subtitles: SubtitleSnapshot,
  settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage"> &
    Partial<Pick<SettingsSnapshot, "subtitleDisplayMode" | "showIntermediateSubtitles">>,
  detectedLanguage: string | null,
  isTranslationPending: boolean,
  isTranslationTimedOut: boolean,
): LiveSubtitlePreview | null {
  const translation = visibleDraft(
    subtitles.translation,
    subtitles.history,
  );
  const showSource = settings.subtitleDisplayMode === "original" ||
    settings.subtitleDisplayMode === "bilingual";
  const sameLanguage = isSameLanguageMode(settings, detectedLanguage);
  // Source/translation snapshots have no shared utterance identity. Only
  // committed history can form a bilingual pair; preview the recognition
  // independently until that pair arrives, never attach a stale translation.
  const bilingualWithoutSource = settings.subtitleDisplayMode === "bilingual" &&
    subtitles.source.text.trim() === "";
  if ((!showSource || bilingualWithoutSource) && translation !== null &&
    (!sameLanguage || subtitles.source.text.trim() === "")) {
    return { ...translation, kind: "translation" };
  }

  if (!showSource && !sameLanguage) return null;

  const source = subtitles.source;
  if (source.text === "") return null;

  const latestPair = subtitles.history[subtitles.history.length - 1];
  const currentTranslationMatchesLatestPair =
    subtitles.translation.isFinal &&
    subtitles.translation.text !== "" &&
    latestPair?.translation === subtitles.translation.text;
  const sourceIsAlreadyCommitted =
    source.isFinal &&
    !isTranslationPending &&
    !isTranslationTimedOut &&
    latestPair?.source === source.text &&
    (showSource || sameLanguage || currentTranslationMatchesLatestPair);
  if (sourceIsAlreadyCommitted) return null;

  return {
    text: source.text,
    isFinal: source.isFinal,
    // Recognition is also the reading text in an original-target or
    // same-language session. Put it in the visible single-language lane.
    kind: settings.subtitleDisplayMode === "translation" ? "translation" : "source",
    ...(source.utteranceId == null ? {} : { utteranceId: source.utteranceId }),
  };
}

/**
 * Every live preview row the overlay should render below the committed
 * history, in display order (original above translation).
 *
 * Bilingual mode used to show only the recognized original until a sentence
 * pair completed, so a service that confirms pairs slowly (or only at long
 * utterance boundaries) left the translation invisible while its draft
 * streamed. The translation preview is therefore shown as soon as it exists,
 * independently of the original; committed pairs still own the durable rows
 * above and never repeat as a preview.
 */
export function visibleLiveSubtitles(
  subtitles: SubtitleSnapshot,
  settings: Pick<SettingsSnapshot, "sourceLanguage" | "targetLanguage"> &
    Partial<Pick<SettingsSnapshot, "subtitleDisplayMode" | "showIntermediateSubtitles">>,
  detectedLanguage: string | null,
  isTranslationPending: boolean,
  isTranslationTimedOut: boolean,
  preferAtomicPreview = false,
): LiveSubtitlePreview[] {
  if (settings.showIntermediateSubtitles === false) {
    // Filter at the presentation boundary only: confirmations, history and
    // provider work keep their original identities and ordering. Never infer
    // confirmation from a stable preview or text matching a history entry.
    subtitles = {
      ...subtitles,
      source: subtitles.source.isFinal ? subtitles.source : { text: "", isFinal: false },
      translation: subtitles.translation.isFinal ? subtitles.translation : { text: "", isFinal: false },
      previewPair: null,
      displayPair: subtitles.displayPairFinal === true ? subtitles.displayPair : null,
    };
  }
  const displayPair = subtitles.displayPair;
  const translatedView = settings.subtitleDisplayMode !== "original" && !isSameLanguageMode(settings, detectedLanguage);
  const hasRealtimeDraft = !preferAtomicPreview && translatedView && (
    (!subtitles.translation.isFinal && subtitles.translation.text.trim() !== "") ||
    (settings.subtitleDisplayMode === "bilingual" && !subtitles.source.isFinal && subtitles.source.text.trim() !== "")
  );
  if (hasRealtimeDraft && displayPair && subtitles.displayPairFinal === true) {
    // Realtime lanes advance independently. A confirmed lane still owned by
    // the previous display pair must not accompany the next lane's draft.
    // Keep the snapshot and its durable history unchanged; this is projection.
    const currentLane = (line: SubtitleSnapshot["source"], previousText: string) =>
      line.isFinal && line.text === previousText &&
      (line.utteranceId ?? null) === (displayPair.utteranceId ?? null)
        ? { text: "", isFinal: false }
        : line;
    subtitles = {
      ...subtitles,
      source: currentLane(subtitles.source, displayPair.source),
      translation: currentLane(subtitles.translation, displayPair.translation),
    };
  }
  // HTTP translation routes intentionally hold complete preview pairs. A
  // realtime transcript draft must instead advance beyond the prior final.
  if (displayPair && !hasRealtimeDraft && translatedView) {
    // A confirmed current pair is already readable in the bounded history lane.
    const last = subtitles.history.at(-1);
    if (last?.source === displayPair.source && last.translation === displayPair.translation && subtitles.previewPair == null) return [];
    const owner = displayPair.utteranceId == null ? {} : { utteranceId: displayPair.utteranceId };
    const source: LiveSubtitlePreview = { kind: "source", text: displayPair.source, isFinal: subtitles.displayPairFinal === true, isStable: true, ...owner };
    const translation: LiveSubtitlePreview = { kind: "translation", text: displayPair.translation, isFinal: subtitles.displayPairFinal === true, isStable: true, ...owner };
    if (settings.subtitleDisplayMode !== "bilingual") return [translation];
    return displayPair.source.trim() === displayPair.translation.trim() ? [source] : [source, translation];
  }
  // HQ's next request can start before its next raw draft is published. Its
  // unstamped final is already owned by history, not a newly recognized tail.
  // Pending/timeout flags cannot reopen it. Preserve real same-text drafts,
  // identified streams, non-atomic providers, and complete preview pairs.
  const committedAtomicSource = preferAtomicPreview &&
    subtitles.source.utteranceId == null && subtitles.source.isFinal &&
    subtitles.history.at(-1)?.source === subtitles.source.text;
  if (committedAtomicSource && (settings.subtitleDisplayMode === "original" ||
    (settings.subtitleDisplayMode === "bilingual" && subtitles.previewPair == null))) {
    return [];
  }
  if (preferAtomicPreview && !isSameLanguageMode(settings, detectedLanguage) && settings.subtitleDisplayMode !== "original") {
    const pair = subtitles.previewPair;
    if (pair) {
      const owner = pair.utteranceId == null ? {} : { utteranceId: pair.utteranceId };
      const source: LiveSubtitlePreview = { kind: "source", text: pair.source, isFinal: false, isStable: true, ...owner };
      const translation: LiveSubtitlePreview = { kind: "translation", text: pair.translation, isFinal: false, isStable: true, ...owner };
      if (settings.subtitleDisplayMode !== "bilingual") return [translation];
      // Match confirmed pairs: identical lanes read once within this utterance.
      return pair.source.trim() === pair.translation.trim() ? [source] : [source, translation];
    }
    // The first recognition may appear before a complete preview exists.
    // A new request's tiny SSE prefixes must not repeatedly erase and rebuild
    // the text the reader just saw. Final history remains independent.
    if (settings.subtitleDisplayMode !== "bilingual") return [];
    const source = visibleLiveSubtitle({ ...subtitles, translation: { text: "", isFinal: false } }, settings,
      detectedLanguage, isTranslationPending, isTranslationTimedOut);
    return source?.kind === "source" ? [source] : [];
  }
  const preview = visibleLiveSubtitle(
    subtitles,
    settings,
    detectedLanguage,
    isTranslationPending,
    isTranslationTimedOut,
  );
  const previews = preview === null ? [] : [preview];
  if (settings.subtitleDisplayMode !== "bilingual") return previews;
  // An empty original already leaves the translation preview on its own.
  if (preview?.kind === "translation") return previews;
  // Same-language drafts can differ briefly while the two streams advance.
  // Showing both would duplicate one language in the bilingual display.
  if (isSameLanguageMode(settings, detectedLanguage)) {
    return previews;
  }
  const translation = visibleDraft(subtitles.translation, subtitles.history);
  if (translation === null) return previews;
  // Never stack a second copy of the same text (same-language or
  // original-target sessions translate into the recognized language).
  if (subtitles.translation.text.trim() === subtitles.source.text.trim()) {
    return previews;
  }
  // Providers that identify their utterances stamp both lines with the source
  // id. Stack only when the stamps agree, or when neither line carries one:
  // a translation whose utterance is unknown (or a different one) still answers
  // the previous sentence, so the original stays alone until its own arrives.
  const sourceUtterance = subtitles.source.utteranceId ?? null;
  const translationUtterance = subtitles.translation.utteranceId ?? null;
  const bothUnstamped = sourceUtterance === null && translationUtterance === null;
  const sameUtterance =
    sourceUtterance !== null && sourceUtterance === translationUtterance;
  if (!bothUnstamped && !sameUtterance) {
    return previews;
  }
  return [...previews, { ...translation, kind: "translation" }];
}

export interface LanguageStatus {
  source: string;
  separator: string;
  target: string;
}

export function languageStatus(
  settings: SettingsSnapshot,
  detectedLanguage: string | null,
): LanguageStatus | null {
  const sourceName = settings.sourceLanguage === "zh_en" ? speechLanguageGuidance(settings).optionLabel(settings.sourceLanguage) : settings.audioInput === "both" && settings.sourceLanguage === "auto"
    ? SOURCE_LANGUAGE_DISPLAY_NAMES.auto : sourceLanguageStatusDisplayName(
    settings.sourceLanguage,
    detectedLanguage,
    settings.targetLanguage,
  );

  if (settings.targetLanguage === "original") {
    return {
      source: sourceName,
      separator: I18N.overlay.dotSeparator,
      target: I18N.overlay.original,
    };
  }
  return {
    source: sourceName,
    separator: I18N.overlay.separator,
    target: targetLanguageOptionLabel(settings, settings.targetLanguage),
  };
}

export function hasSubtitleContent(subtitles: SubtitleSnapshot): boolean {
  return (
    (subtitles.tracks?.some(track => hasSubtitleContent(track)) ?? false) ||
    subtitles.source.text !== "" ||
    subtitles.translation.text !== "" ||
    (subtitles.displayPair != null &&
      (subtitles.displayPair.source.trim() !== "" || subtitles.displayPair.translation.trim() !== "")) ||
    (subtitles.previewPair !== undefined && subtitles.previewPair !== null &&
      (subtitles.previewPair.source.trim() !== "" || subtitles.previewPair.translation.trim() !== "")) ||
    subtitles.history.length > 0
  );
}
