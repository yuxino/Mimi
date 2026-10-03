import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { hexToRgba } from "../../lib/types";
import { I18N } from "../../lib/i18n";
import { subtitleColorHex } from "../../lib/subtitleColor";
import type { SettingsSnapshot, SubtitleAlignment, SubtitleColor } from "../../lib/types";
import { observeTimelineResize } from "./timelineResize";
import { TimelineScroll } from "./timelineScroll";
import { rowHorizontalPadding } from "./alignment";
import { compactRepetition } from "./compactRepetition";
import {
  subtitleLaneBudget,
  subtitleSourceScale,
  SUBTITLE_LINE_HEIGHT,
  SUBTITLE_SOURCE_SCALE,
  timelineClassName,
  type SubtitleBlock,
} from "./overlayModel";

const ACCENT = "#7AA8FF";
const MONO_FONT =
  '"SF Mono", Menlo, Consolas, "Courier New", monospace';
const IMMERSIVE_TEXT_SHADOW =
  "0 2px 5px rgba(0,0,0,0.98), 0 0 2px rgba(0,0,0,0.95), 0 0 12px rgba(0,0,0,0.72)";
/** Vertical rhythm: lines of one utterance sit close, sentences breathe. */
const LANE_GAP = 2;
const BLOCK_PADDING_Y = 2;
const LAST_BLOCK_PADDING_Y = 3;
/** Separator gap for the card presentation; immersive mode uses space only. */
const SEPARATOR_MARGIN_Y = 7;
const IMMERSIVE_BLOCK_GAP = 6;
interface TimelineProps {
  blocks: SubtitleBlock[];
  fontSize: number;
  alignment: SubtitleAlignment;
  color: SubtitleColor;
  /** Lane hierarchy follows the display mode: bilingual keeps the recognized
   * original as a neutral reference lane, single-language modes read in the
   * user's subtitle color. */
  displayMode: SettingsSnapshot["subtitleDisplayMode"];
  /** Optional metadata; hidden by default so sentence boundaries lead. */
  showTimestamps?: boolean;
  showSubtitleDividers?: boolean;
  keepTextOpaque?: boolean;
  microphoneColor?: SubtitleColor;
  blendsWithBackground?: boolean;
  /** Resolved motion setting: gates the roll-up glide. */
  motionEnabled?: boolean;
  /** A new request explicitly returns a history reader to the live tail. */
  followTailRequest?: number;
  onReadingHistoryChange?: (reading: boolean) => void;
}

/** Scrolling sentence blocks; auto-scrolls to the newest block. Memoized:
 * during live streaming the overlay re-renders on every session-state event,
 * but the timeline DOM only needs rebuilding when its blocks actually
 * change. */
export const Timeline = memo(function Timeline({
  blocks,
  fontSize,
  alignment,
  color,
  displayMode,
  blendsWithBackground = false,
  motionEnabled = true,
  showTimestamps = false,
  showSubtitleDividers = false,
  keepTextOpaque = false,
  microphoneColor = "yellow",
  followTailRequest = 0,
  onReadingHistoryChange,
}: TimelineProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  // Keep the newest content pinned to the bottom: the block count changes when
  // an utterance is committed, and the live lanes grow while streaming (the
  // last block grows taller without changing the block count).
  const lastTextLength = useMemo(() => {
    const last = blocks[blocks.length - 1];
    return (last?.source?.length ?? 0) + (last?.translation?.length ?? 0);
  }, [blocks]);
  const blockLayoutKey = useMemo(() => JSON.stringify(blocks.map(block => block.id)), [blocks]);
  const liveBlockCount = blocks.filter(block => block.presentation === "live").length;
  const prevBlockCountRef = useRef(blocks.length);
  const previousModeRef = useRef(displayMode);
  const modeChangedRef = useRef(false);
  const [scroll] = useState(() => new TimelineScroll());
  const [reading, setReading] = useState({ active: false, request: followTailRequest });
  const readingHistory = reading.active && reading.request === followTailRequest;
  const previousFollowRequest = useRef(followTailRequest);
  const setReadingHistory = (active: boolean) => setReading(previous =>
    previous.active === active && previous.request === followTailRequest
      ? previous : { active, request: followTailRequest });
  const [viewportHeight, setViewportHeight] = useState<number | null>(null);
  const [laneMeasurements, setLaneMeasurements] = useState({ blockId: "", source: 0, translation: 0 });
  const touchStartYRef = useRef<number | null>(null);
  const tight = viewportHeight !== null && viewportHeight < 80;
  const laneGap = tight ? 1 : LANE_GAP;
  const paddingTop = tight ? 0 : blendsWithBackground ? IMMERSIVE_BLOCK_GAP : BLOCK_PADDING_Y;
  const paddingBottom = tight ? 1 : LAST_BLOCK_PADDING_Y;

  useEffect(() => { onReadingHistoryChange?.(readingHistory); }, [onReadingHistoryChange, readingHistory]);
  useEffect(() => () => onReadingHistoryChange?.(false), [onReadingHistoryChange]);

  useLayoutEffect(() => {
    if (containerRef.current) scroll.reflow(containerRef.current);
  }, [readingHistory, viewportHeight, showSubtitleDividers, scroll]);

  useLayoutEffect(() => {
    if (previousModeRef.current === displayMode) return;
    previousModeRef.current = displayMode;
    modeChangedRef.current = true;
    if (containerRef.current) scroll.displayChanged(containerRef.current);
  }, [displayMode, scroll]);

  // Explicit follow intent wins even if a display-mode change shares this
  // render. Later row-size observations keep it pinned as compact lanes settle.
  useLayoutEffect(() => {
    if (previousFollowRequest.current === followTailRequest) return;
    previousFollowRequest.current = followTailRequest;
    if (containerRef.current) {
      scroll.followTail(containerRef.current);
      containerRef.current.focus({ preventScroll: true });
    }
  }, [followTailRequest, scroll]);

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const newBlock = blocks.length !== prevBlockCountRef.current;
    prevBlockCountRef.current = blocks.length;
    // Do not overwrite the new mode's sentence anchor with a second passive
    // effect in the same render. Real subsequent text still follows the tail.
    if (modeChangedRef.current) { modeChangedRef.current = false; return; }
    scroll.contentChanged(element, newBlock && motionEnabled ? "smooth" : "instant");
  }, [blocks.length, lastTextLength, fontSize, alignment, blendsWithBackground, motionEnabled, displayMode, scroll]);

  useLayoutEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const resized = () => {
      const height = Math.round(element.clientHeight);
      if (height > 0) setViewportHeight(height);
      scroll.reflow(element);
    };
    resized();
    return observeTimelineResize(element, resized);
  }, [scroll, blockLayoutKey]);

  return (
    <div
      ref={containerRef}
      tabIndex={0}
      onWheel={(event) => {
        if (event.deltaY < 0) {
          scroll.beginReading(event.currentTarget);
          setReadingHistory(true);
        } else if (event.deltaY > 0) {
          scroll.userIntent(event.currentTarget, "down");
          // A downward gesture at the bottom also closes history when short
          // content has no scrollbar and would not emit a scroll event.
          scroll.followIfAtTail(event.currentTarget);
          setReadingHistory(!scroll.isFollowing());
        }
      }}
      onTouchStart={(event) => {
        touchStartYRef.current = event.touches[0]?.clientY ?? null;
        scroll.endUserIntent();
      }}
      onTouchMove={(event) => {
        const y = event.touches[0]?.clientY;
        if (touchStartYRef.current !== null && y !== undefined && y - touchStartYRef.current > 2) {
          scroll.beginReading(event.currentTarget);
          setReadingHistory(true);
          touchStartYRef.current = y;
        } else if (touchStartYRef.current !== null && y !== undefined && y - touchStartYRef.current < -2) {
          scroll.userIntent(event.currentTarget, "down");
          scroll.followIfAtTail(event.currentTarget);
          setReadingHistory(!scroll.isFollowing());
          touchStartYRef.current = y;
        }
      }}
      onTouchEnd={() => { touchStartYRef.current = null; scroll.endUserIntent(); }}
      onTouchCancel={() => { touchStartYRef.current = null; scroll.endUserIntent(); }}
      onPointerDown={(event) => scroll.userIntent(event.currentTarget)}
      onPointerMove={(event) => { if (event.buttons !== 0) scroll.userIntent(event.currentTarget); }}
      onPointerUp={() => scroll.endUserIntent()}
      onPointerCancel={() => scroll.endUserIntent()}
      onKeyDown={(event) => {
        if (event.key === "Home") {
          event.preventDefault();
          scroll.readFromStart(event.currentTarget);
          setReadingHistory(true);
        } else if (["ArrowUp", "PageUp"].includes(event.key) || (event.key === " " && event.shiftKey)) {
          scroll.beginReading(event.currentTarget);
          setReadingHistory(true);
        } else if (event.key === "End") {
          event.preventDefault();
          scroll.followTail(event.currentTarget);
          setReadingHistory(false);
        } else if (["ArrowDown", "PageDown", " "].includes(event.key)) {
          scroll.userIntent(event.currentTarget, "down");
          scroll.followIfAtTail(event.currentTarget);
          setReadingHistory(!scroll.isFollowing());
        }
      }}
      onScroll={(event) => {
        scroll.scrolled(event.currentTarget);
        setReadingHistory(!scroll.isFollowing());
      }}
      className={`overlay-timeline ${timelineClassName(blendsWithBackground)}`}
      style={{
        display: "flex", flexDirection: "column",
        overscrollBehavior: "contain", overflowAnchor: "none",
      }}
    >
      {blocks.map((block, index) => {
        const isLast = index === blocks.length - 1;
        // Following keeps completed long utterances in the same bounded tail
        // as the live sentence. Deliberate reading opens either kind of row;
        // it does not confirm or retain a replaceable live draft.
        const compact = !readingHistory;
        // Only a sentence that appears for the first time animates in. A
        // committed utterance replaces the live row it was already visible as,
        // so animating it again would blink the text the user is reading.
        const sourceColor = block.audioSource === "microphone" ? microphoneColor : color;
        const entering = !keepTextOpaque && block.presentation === "live";
        // Two independent live sources share the visible lane budget, so a
        // long microphone preview cannot push system speech out of view.
        const blockViewportHeight = viewportHeight === null ? null
          : compact && block.presentation === "live" && liveBlockCount > 1 ? viewportHeight / liveBlockCount : viewportHeight;
        const availableLaneHeight = blockViewportHeight === null ? null
          : blockViewportHeight - paddingTop - paddingBottom - (block.source !== null && block.translation !== null ? laneGap : 0);
        // A bilingual original keeps its reference font while waiting for MT.
        // Only its line budget changes when the translation takes its space.
        const sourceScale = subtitleSourceScale(blockViewportHeight === null ? null
          : blockViewportHeight - paddingTop - paddingBottom - (displayMode === "bilingual" ? laneGap : 0));
        // When the original has not arrived, the translation owns the full
        // viewport rather than reserving height for an absent reference lane.
        const budgetMode = displayMode === "bilingual" && block.source === null ? "translation" : displayMode;
        const budget = subtitleLaneBudget(budgetMode, block.translation !== null, availableLaneHeight, fontSize,
          isLast && laneMeasurements.blockId === block.id ? laneMeasurements : null, sourceScale);
        const measureLane = (kind: "source" | "translation", height: number) => {
          if (!isLast) return;
          setLaneMeasurements(previous => {
            const current = previous.blockId === block.id ? previous : { blockId: block.id, source: 0, translation: 0 };
            return current[kind] === height ? previous : { ...current, [kind]: height };
          });
        };
        // A new sentence must not collapse the previous phrase to its last
        // word. Every following row keeps the viewport's bounded lane budget;
        // the timeline's outer scroll chooses which rows remain in view.
        return (
          <div
            key={block.id}
            data-utterance-id={block.id}
            className={entering ? "relative subtitle-block" : "relative"}
            style={{
              // Short output stays at the reading edge instead of hanging
              // beneath the controls. Auto margin yields to zero once the
              // history overflows, keeping every sentence scrollable.
              marginTop: index === 0 ? "auto" : undefined,
              flexShrink: 0,
              paddingLeft: rowHorizontalPadding(
                alignment,
                "left",
                blendsWithBackground || !showTimestamps,
              ),
              paddingRight: rowHorizontalPadding(
                alignment,
                "right",
                blendsWithBackground || !showTimestamps,
              ),
              paddingTop,
              paddingBottom: tight ? 1 : isLast ? LAST_BLOCK_PADDING_Y : BLOCK_PADDING_Y,
              // New blocks settle in with a brief rise-and-fade; the class runs
              // the animation once on mount (the key is stable per block, so
              // streaming text updates do not re-trigger it) and is skipped
              // when motion is off.
            }}
          >
            {showTimestamps && !blendsWithBackground && block.createdAt !== null ? (
              <span
                className="subtitle-timestamp"
                style={{
                  position: "absolute",
                  left: 18,
                  top: isLast ? 12 : 10,
                  width: 31,
                  textAlign: "right",
                  fontSize: 9,
                  fontWeight: 500,
                  fontFamily: MONO_FONT,
                  fontVariantNumeric: "tabular-nums",
                  color: hexToRgba(ACCENT, 0.46),
                }}
              >
                {formatTimestamp(block.createdAt)}
              </span>
            ) : null}
            <div style={{ display: "flex", gap: block.audioSource ? 10 : 0, alignItems: "flex-start" }}>
              {block.audioSource && <span className="subtitle-audio-source" style={{
                fontSize, lineHeight: SUBTITLE_LINE_HEIGHT, fontWeight: 500, flexShrink: 0,
                color: hexToRgba(subtitleColorHex(sourceColor), keepTextOpaque ? 1 : 0.78), textShadow: blendsWithBackground ? IMMERSIVE_TEXT_SHADOW : undefined,
              }}>{block.audioSource === "system" ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone}</span>}
            <div style={{ display: "flex", flexDirection: "column", gap: laneGap, minWidth: 0, flex: 1 }}>
              {block.source !== null && displayMode !== "translation" ? (
                <Lane
                  text={block.source}
                  kind="source"
                  lines={compact && budget.source > 0 ? budget.source : null}
                  fontSize={fontSize}
                  alignment={alignment}
                  displayMode={displayMode}
                  color={sourceColor}
                  tintReference={block.audioSource != null}
                  blendsWithBackground={blendsWithBackground}
                  motionEnabled={motionEnabled}
                  entering={entering}
                  keepTextOpaque={keepTextOpaque}
                  sourceScale={sourceScale}
                  onMeasure={height => measureLane("source", height)}
                />
              ) : null}
              {block.translation !== null && displayMode !== "original" ? (
                <Lane
                  text={block.translation}
                  kind="translation"
                  lines={compact && budget.translation > 0 ? budget.translation : null}
                  fontSize={fontSize}
                  alignment={alignment}
                  displayMode={displayMode}
                  color={sourceColor}
                  tintReference={block.audioSource != null}
                  blendsWithBackground={blendsWithBackground}
                  motionEnabled={motionEnabled}
                  entering={entering}
                  keepTextOpaque={keepTextOpaque}
                  onMeasure={height => measureLane("translation", height)}
                />
              ) : null}
            </div>
            </div>
            {/* The separator scrolls with the sentence above;
                Immersive Mode keeps space only. */}
            {showSubtitleDividers && !isLast && !blendsWithBackground ? (
              <div
                aria-hidden="true"
                className="subtitle-separator"
                style={{
                  height: 1,
                  width: "100%",
                  margin: `${SEPARATOR_MARGIN_Y}px auto`,
                }}
              />
            ) : null}
          </div>
        );
      })}
    </div>
  );
});

interface LaneProps {
  text: string;
  kind: "source" | "translation";
  /** Visual lines to keep, newest text first in view; `null` renders the whole
   * confirmed sentence when the user is reading history. */
  lines: number | null;
  fontSize: number;
  alignment: SubtitleAlignment;
  displayMode: SettingsSnapshot["subtitleDisplayMode"];
  color: SubtitleColor;
  blendsWithBackground: boolean;
  motionEnabled: boolean;
  /** Runs the lane fade only for text that was not on screen before. */
  entering: boolean;
  hidden?: boolean;
  onMeasure?: (height: number) => void;
  sourceScale?: number;
  tintReference?: boolean;
  keepTextOpaque?: boolean;
}

function Lane({
  text,
  kind,
  lines,
  fontSize,
  alignment,
  displayMode,
  color,
  blendsWithBackground,
  motionEnabled,
  entering,
  hidden = false,
  onMeasure,
  sourceScale = SUBTITLE_SOURCE_SCALE,
  tintReference = false,
  keepTextOpaque = false,
}: LaneProps) {
  const isSource = kind === "source";
  // In bilingual mode the recognized original is the reference lane: neutral
  // white, slightly smaller. In a single-language mode the visible lane is the
  // reading target and uses the user's subtitle color.
  const isReference = isSource && displayMode === "bilingual";
  const laneFontSize = isReference ? Math.max(12, fontSize * sourceScale) : fontSize;
  const lineHeightPx = Math.ceil(laneFontSize * SUBTITLE_LINE_HEIGHT);
  const textStyle = {
    fontSize: laneFontSize,
    fontWeight: isReference ? 400 : 500,
    color: hexToRgba(isReference && !tintReference ? "#FFFFFF" : subtitleColorHex(color), isReference && !keepTextOpaque ? 0.86 : 1),
    lineHeight: `${lineHeightPx}px`,
    overflowWrap: "break-word" as const,
    textShadow: blendsWithBackground ? IMMERSIVE_TEXT_SHADOW : undefined,
  };

  if (lines === null) {
    return (
      <span
        {...(__MIMI_DEVELOPMENT_BUILD__ ? { "data-debug-lane": kind } : {})}
        className={entering ? "block min-w-0 subtitle-lane" : "block min-w-0"}
        hidden={hidden}
        style={{ textAlign: alignment, ...textStyle, display: hidden ? "none" : undefined }}
      >
        {text}
      </span>
    );
  }

  return (
    <CompactLane
      text={text}
      kind={kind}
      lines={lines}
      lineHeightPx={lineHeightPx}
      alignment={alignment}
      textStyle={textStyle}
      motionEnabled={motionEnabled}
      entering={entering}
      hidden={hidden}
      onMeasure={onMeasure}
    />
  );
}

interface CompactLaneProps {
  text: string;
  kind: "source" | "translation";
  lines: number;
  lineHeightPx: number;
  motionEnabled: boolean;
  entering: boolean;
  alignment: SubtitleAlignment;
  hidden?: boolean;
  onMeasure?: (height: number) => void;
  textStyle: {
    fontSize: number;
    fontWeight: number;
    color: string;
    lineHeight: string;
    overflowWrap: "break-word";
    textShadow: string | undefined;
  };
}

/**
 * A lane that keeps only its newest `lines` visual lines. The full text is laid
 * out by the browser and anchored to the bottom, so the line breaker, CJK
 * wrapping, and font metrics stay the platform's job; the viewport clips the
 * old content from the top. A continuation marker appears only once the text
 * actually overflows, and the accessible name stays the full sentence.
 */
function CompactLane({
  text,
  kind,
  lines,
  lineHeightPx,
  motionEnabled,
  entering,
  alignment,
  textStyle,
  hidden = false,
  onMeasure,
}: CompactLaneProps) {
  // Compact presentation can abbreviate extreme exact repetition. Keep the
  // original text for accessibility and the unabridged reading-mode lane.
  const displayText = useMemo(() => compactRepetition(text), [text]);
  const viewportRef = useRef<HTMLDivElement>(null);
  const innerRef = useRef<HTMLSpanElement>(null);
  const { overflowed, innerHeight } = useLaneOverflow(viewportRef, onMeasure);
  const maximumHeight = Math.round(lines * lineHeightPx);
  // Let the text glide upward when a new line pushes it instead of jumping.
  useRollupGlide(innerRef, innerHeight, motionEnabled && innerHeight > maximumHeight + 2);
  return (
    <div
      ref={viewportRef}
      {...(__MIMI_DEVELOPMENT_BUILD__ ? { "data-debug-lane": kind } : {})}
      aria-label={text}
      aria-hidden={hidden || undefined}
      hidden={hidden}
      style={{
        display: hidden ? "none" : undefined,
        position: "relative",
        // Reserve the limit only until the browser supplies its first layout;
        // short sentences then take exactly the space their actual lines need.
        height: innerHeight > 0 ? Math.min(innerHeight, maximumHeight) : maximumHeight,
        overflow: "hidden",
      }}
    >
      <span
        ref={innerRef}
        className={entering ? "subtitle-lane" : undefined}
        aria-hidden="true"
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          // Text starts on the first line while it fits; once it fills the
          // budget the same element stays pinned to the bottom so new content
          // grows upward and the old content rolls off the top.
          ...(overflowed ? { bottom: 0 } : { top: 0 }),
          textAlign: alignment,
          ...textStyle,
        }}
      >
        {displayText}
      </span>
    </div>
  );
}

/** Whether the lane's text overflows its budget, plus its laid-out height. */
function useLaneOverflow(viewportRef: RefObject<HTMLDivElement | null>, onMeasure?: (height: number) => void): {
  overflowed: boolean;
  innerHeight: number;
} {
  const [measured, setMeasured] = useState({ overflowed: false, innerHeight: 0 });
  const reportHeight = useRef(onMeasure);
  useLayoutEffect(() => { reportHeight.current = onMeasure; });
  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (viewport === null) return;
    const measure = () => {
      const inner = viewport.firstElementChild;
      if (inner === null) return;
      const height = Math.round(inner.getBoundingClientRect().height);
      reportHeight.current?.(height);
      // Two pixels of hysteresis keep the marker from flickering on the exact
      // boundary while the text streams in.
      const overflowed = height > viewport.clientHeight + 2;
      setMeasured(previous => previous.innerHeight === height && previous.overflowed === overflowed
        ? previous
        : { overflowed, innerHeight: height });
    };
    measure();
    const observer = new ResizeObserver(measure);
    // The viewport pins the height, so streaming text growth shows up on the
    // inner element; a font or width change shows up on the viewport.
    observer.observe(viewport);
    const inner = viewport.firstElementChild;
    if (inner !== null) observer.observe(inner);
    return () => observer.disconnect();
  }, [viewportRef]);
  return measured;
}

/**
 * Bottom-anchored text moves up by one line whenever it grows past the
 * budget. Shifting it back by the growth and releasing that offset turns the
 * jump into a short glide, which is what makes a roll-up feel continuous.
 */
function useRollupGlide(
  innerRef: RefObject<HTMLSpanElement | null>,
  innerHeight: number,
  enabled: boolean,
): void {
  const previousHeightRef = useRef(0);
  useEffect(() => {
    const inner = innerRef.current;
    const previous = previousHeightRef.current;
    previousHeightRef.current = innerHeight;
    if (inner === null) return;
    if (!enabled) {
      inner.style.transition = "none";
      inner.style.transform = "translateY(0)";
      return;
    }
    if (previous === 0 || innerHeight <= previous) return;
    // bottom:0 has already moved existing text up by the new line's height.
    // Restore its previous position with a positive offset, then slide up.
    const shift = innerHeight - previous;
    inner.style.transition = "none";
    inner.style.transform = `translateY(${shift}px)`;
    void inner.offsetHeight;
    inner.style.transition = "transform 180ms ease-out";
    inner.style.transform = "translateY(0)";
  }, [innerRef, innerHeight, enabled]);
}

/** HH:mm in local time using a 24-hour clock. */
function formatTimestamp(createdAt: number): string {
  const date = new Date(createdAt);
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  return `${hours}:${minutes}`;
}
