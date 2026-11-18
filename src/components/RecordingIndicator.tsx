import type { ResolvedUiLanguage } from "../lib/i18n";
import type { AudioRecordingStatus } from "../lib/types";
import { recordingStatusCopy } from "../lib/audio/recordingStatus";
import { Icon } from "./Icon";
import "./RecordingIndicator.css";

export interface RecordingIndicatorProps {
  status?: AudioRecordingStatus | null;
  /** The collapsed caption toolbar keeps only the semantic mark. */
  compact?: boolean;
  language: ResolvedUiLanguage;
}

/** Static recording feedback inside existing chrome; never a capture control. */
export function RecordingIndicator({ status, compact = false, language }: RecordingIndicatorProps) {
  if (!status || status === "off") return null;
  const text = recordingStatusCopy[language][status];
  return (
    <span className={`recording-indicator${compact ? " recording-indicator--compact" : ""}`}
      data-recording-status={status} role="img" aria-label={text.label} title={text.label}>
      <span className="recording-indicator__mark" aria-hidden="true">
        {status === "error"
          ? <Icon name="exclamation-triangle" />
          : <span className="recording-indicator__dot" />}
      </span>
      {!compact && <span className="recording-indicator__label" aria-hidden="true">{text.short}</span>}
    </span>
  );
}
