import { I18N } from "../../lib/i18n";
import type { SessionStateEvent } from "../../lib/types";
import { formatLatency, latencyTone } from "./latencyFormat";
import "./OverlayLatency.css";

/** Actual observations plus explicit pending/recovery states, without polling. */
export function OverlayLatency({ session, translationRequired = true, showApiLatency = true }: { session: SessionStateEvent; translationRequired?: boolean; showApiLatency?: boolean }) {
  if (!session.isActive || (!showApiLatency && !translationRequired)) return null;
  const current = session.status.kind === "listening" && !session.isPaused;
  const follow = session.translationLatencyKind === "follow";
  const apiSample = formatLatency(current ? session.apiLatencyMs : null);
  const translationSample = formatLatency(current ? session.translationLatencyMs : null);
  const api = current && apiSample === "—" ? I18N.overlay.latencyWaiting : apiSample;
  const recovery = current ? session.translationRecovery : null;
  const translationPending = current && (session.isTranslationPending || session.isTranslationPreviewPending);
  const translation = recovery ? recovery.retryScheduled === false
    ? (recovery.reason === "rateLimited" ? I18N.overlay.translationLimited : I18N.overlay.translationUnavailable)
    : (recovery.reason === "rateLimited" ? I18N.overlay.translationRateLimited : I18N.overlay.translationRetrying)
    : translationPending ? I18N.overlay.translating
    : current && translationSample === "—" ? I18N.overlay.latencyWaiting : translationSample;
  const translationLabel = follow ? I18N.overlay.translationFollowLatency : I18N.overlay.translationLatency;
  const translationHelp = follow ? I18N.overlay.translationFollowLatencyHelp : I18N.overlay.translationLatencyHelp;
  const apiTone = latencyTone(current ? session.apiLatencyMs : null, "api");
  const translationTone = latencyTone(current && !recovery && !translationPending ? session.translationLatencyMs : null, "translation");
  const description = (help: string, value: string) => `${help} ${value === "—" ? I18N.overlay.latencyUnavailable : value}`;
  return (
    <div className="overlay-latency" data-testid="overlay-latency">
      {showApiLatency && <span title={description(I18N.overlay.apiLatencyHelp, api)} aria-label={`${I18N.overlay.apiLatency}: ${api}`}>
        <span>{I18N.overlay.apiLatencyShort}</span><strong data-tone={apiTone}>{api}</strong>
      </span>}
      {translationRequired && <>
      {showApiLatency && <span className="overlay-latency__separator" aria-hidden="true">·</span>}
      <span role={recovery ? "status" : undefined} title={description(translationHelp, translation)} aria-label={`${translationLabel}: ${translation}`}>
        <span>{translationLabel}</span><strong data-tone={translationTone}>{translation}</strong>
      </span>
      </>}
    </div>
  );
}
