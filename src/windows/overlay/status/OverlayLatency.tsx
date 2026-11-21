import { I18N } from "../../../lib/i18n";
import { Icon } from "../../../components/Icon";
import type { SessionStateEvent } from "../../../lib/types";
import { formatLatency, latencyTone } from "./latencyFormat";
import "./OverlayLatency.css";

export type OverlayLatencySession = Pick<SessionStateEvent,
  "isActive" | "isPaused" | "apiLatencyMs" | "translationLatencyMs" | "translationLatencyKind" |
  "translationRecovery"
> & { status: { kind: SessionStateEvent["status"]["kind"] } };

function translationStatusLabel(session: OverlayLatencySession, translating: boolean) {
  const recovery = session.translationRecovery;
  if (recovery) {
    if (recovery.retryScheduled === false) {
      return recovery.reason === "rateLimited" ? I18N.overlay.translationLimited : I18N.overlay.translationUnavailable;
    }
    return recovery.reason === "rateLimited" ? I18N.overlay.translationRateLimited : I18N.overlay.translationRetrying;
  }
  return translating ? I18N.overlay.translating : null;
}

export function OverlayLatency({ session, translationRequired = true, translating = false, surface = "panel" }: {
  session: OverlayLatencySession;
  translationRequired?: boolean;
  translating?: boolean;
  surface?: "canvas" | "panel";
}) {
  if (!session.isActive || session.status.kind !== "listening" || session.isPaused) return null;
  const api = formatLatency(session.apiLatencyMs);
  const kind = session.translationLatencyKind;
  const translationSample = formatLatency(kind === "request" || kind === "follow" ? session.translationLatencyMs : null);
  const canvas = surface === "canvas";
  const recovery = translationRequired && Boolean(session.translationRecovery);
  const status = translationRequired ? translationStatusLabel(session, translating) : null;
  const showApi = api !== "—" && !(canvas && recovery);
  // The hover readout can show the last completed sample while the next request runs.
  const showTranslation = translationRequired && translationSample !== "—" && !recovery && (canvas || !status);
  if (!status && !showApi && !showTranslation) return null;
  const follow = kind === "follow";
  const translationLabel = follow ? I18N.overlay.translationFollowLatency : I18N.overlay.translationLatency;
  const translationHelp = follow ? I18N.overlay.translationFollowLatencyHelp : I18N.overlay.translationLatencyHelp;
  const separator = <span className="overlay-latency__separator" aria-hidden="true">·</span>;
  return (
    <div className={`overlay-latency${canvas && !recovery ? " overlay-hover-control" : ""}`} data-testid="overlay-latency">
      {showApi && <span className="overlay-latency__measurement" title={`${I18N.overlay.apiLatencyHelp} ${api}`} aria-label={`${I18N.overlay.apiLatency}: ${api}`}>
        <span>{I18N.overlay.apiLatencyShort}</span><strong data-tone={latencyTone(session.apiLatencyMs, "api")}>{api}</strong>
      </span>}
      {showApi && (status || showTranslation) && separator}
      {status && <span role="status" title={status}>
        <strong data-tone="neutral">{status}</strong>
      </span>}
      {status && showTranslation && separator}
      {showTranslation && <span className="overlay-latency__measurement" title={`${translationHelp} ${translationSample}`}
        aria-label={`${translationLabel}: ${translationSample}`}>
        {canvas ? <Icon name="languages" /> : <span>{translationLabel}</span>}
        <strong data-tone={latencyTone(session.translationLatencyMs, "translation")}>{translationSample}</strong>
      </span>}
    </div>
  );
}
