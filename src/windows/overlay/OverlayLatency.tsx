import { I18N } from "../../lib/i18n";
import type { SessionStateEvent } from "../../lib/types";
import { formatLatency, latencyTone } from "./latencyFormat";
import "./OverlayLatency.css";

/** Show observations only after measurement; service work remains a separate status. */
export function OverlayLatency({ session, translationRequired = true }: { session: SessionStateEvent; translationRequired?: boolean }) {
  if (!session.isActive || session.status.kind !== "listening" || session.isPaused) return null;
  const api = formatLatency(session.apiLatencyMs);
  const kind = session.translationLatencyKind;
  const translationSample = formatLatency(kind === "request" || kind === "follow" ? session.translationLatencyMs : null);
  const recovery = session.translationRecovery;
  const pending = session.isTranslationPending || session.isTranslationPreviewPending;
  const translationStatus = recovery ? recovery.retryScheduled === false
    ? (recovery.reason === "rateLimited" ? I18N.overlay.translationLimited : I18N.overlay.translationUnavailable)
    : (recovery.reason === "rateLimited" ? I18N.overlay.translationRateLimited : I18N.overlay.translationRetrying)
    : pending ? I18N.overlay.translating : null;
  const showApi = api !== "—";
  const showTranslation = translationRequired && (translationStatus !== null || translationSample !== "—");
  if (!showApi && !showTranslation) return null;
  const follow = kind === "follow";
  const translationLabel = follow ? I18N.overlay.translationFollowLatency : I18N.overlay.translationLatency;
  const translationHelp = follow ? I18N.overlay.translationFollowLatencyHelp : I18N.overlay.translationLatencyHelp;
  return (
    <div className="overlay-latency" data-testid="overlay-latency">
      {showApi && <span title={`${I18N.overlay.apiLatencyHelp} ${api}`} aria-label={`${I18N.overlay.apiLatency}: ${api}`}>
        <span>{I18N.overlay.apiLatencyShort}</span><strong data-tone={latencyTone(session.apiLatencyMs, "api")}>{api}</strong>
      </span>}
      {showApi && showTranslation && <span className="overlay-latency__separator" aria-hidden="true">·</span>}
      {showTranslation && <span role={translationStatus ? "status" : undefined}
        title={translationStatus ?? `${translationHelp} ${translationSample}`}
        aria-label={translationStatus ?? `${translationLabel}: ${translationSample}`}>
        {!translationStatus && <span>{translationLabel}</span>}
        <strong data-tone={translationStatus ? "neutral" : latencyTone(session.translationLatencyMs, "translation")}>{translationStatus ?? translationSample}</strong>
      </span>}
    </div>
  );
}
