import type { ReactNode, RefObject } from "react";
import { Icon } from "../../../components/Icon";
import { I18N, type ResolvedUiLanguage } from "../../../lib/i18n";
import { isTauri } from "../../../lib/ipc";
import type {
  AudioSource,
  SessionHistoryItem,
  TranscriptExportOptions,
  TranscriptPage,
} from "../../../lib/types";
import { SettingsActionMenu } from "../controls/SettingsActionMenu";
import { InlineFeedback, SettingsSelect } from "../controls/SettingsPrimitives";
import { SessionAudioPlayer } from "./SessionAudioPlayer";
import type { SessionWorkspaceStrings } from "./SessionHistoryLibrary";
import type { useSessionAudioPreview } from "./useSessionAudioPreview";

interface SessionTranscriptReaderProps {
  readerHeading: RefObject<HTMLHeadingElement | null>;
  sessionName: string;
  selected: SessionHistoryItem | undefined;
  selectedId: string | null;
  selectedAudioSource: AudioSource | undefined;
  active: boolean;
  busy: boolean;
  disabled: boolean;
  readError: boolean;
  pageVisible: boolean;
  hasTranscript: boolean;
  hasAudio: boolean;
  canExportTranscript: boolean;
  canExportAudio: boolean;
  exportDisabled: boolean;
  availableCount: number;
  displayedTranscript: TranscriptPage | undefined;
  transcriptError: boolean;
  transcriptLoading: boolean;
  query: string;
  reading: TranscriptExportOptions["content"];
  contentOptions: { value: string; label: string }[];
  audioPreview: ReturnType<typeof useSessionAudioPreview>;
  sourceControl: ReactNode;
  language: ResolvedUiLanguage;
  copy: SessionWorkspaceStrings;
  onBack: () => void;
  onOpenExport: () => void;
  onOpenRename: () => void;
  onRequestDelete: () => void;
  onClearCurrentArchive: () => void;
  onQueryChange: (query: string) => void;
  onReadingChange: (reading: TranscriptExportOptions["content"]) => void;
  onRetryTranscript: () => void;
  onCopyPassage: (source: string, translation: string) => void;
  onChangePage: (page: number) => void;
}

export function SessionTranscriptReader({
  readerHeading,
  sessionName,
  selected,
  selectedId,
  selectedAudioSource,
  active,
  busy,
  disabled,
  readError,
  pageVisible,
  hasTranscript,
  hasAudio,
  canExportTranscript,
  canExportAudio,
  exportDisabled,
  availableCount,
  displayedTranscript,
  transcriptError,
  transcriptLoading,
  query,
  reading,
  contentOptions,
  audioPreview,
  sourceControl,
  language,
  copy,
  onBack,
  onOpenExport,
  onOpenRename,
  onRequestDelete,
  onClearCurrentArchive,
  onQueryChange,
  onReadingChange,
  onRetryTranscript,
  onCopyPassage,
  onChangePage,
}: SessionTranscriptReaderProps) {
  let transcriptTimeFormatter: Intl.DateTimeFormat | undefined;
  const formatTranscriptTime = (timestamp: number) =>
    (transcriptTimeFormatter ??= new Intl.DateTimeFormat(language, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    })).format(timestamp);

  return (
    <div className="session-transcript">
      <button type="button" className="session-workspace__back" onClick={onBack}>
        <Icon name="chevron-left" />
        {copy.back}
      </button>
      <div className="session-transcript__heading">
        <div>
          <h3 ref={readerHeading} tabIndex={-1}>{sessionName}</h3>
          <span className="session-workspace__reader-meta">
            {selected && (
              <time>
                {new Intl.DateTimeFormat(language, { dateStyle: "medium", timeStyle: "short" }).format(selected.startedAtMs)}
              </time>
            )}
            {availableCount > 0 && <span>{I18N.settings.transcriptCount(availableCount)}</span>}
          </span>
        </div>
        <div className="session-workspace__reader-actions">
          <button
            type="button"
            className="settings-button settings-button--quiet"
            disabled={exportDisabled || (!canExportTranscript && !canExportAudio)}
            onClick={onOpenExport}
          >
            <Icon name="download" />
            {copy.export}
          </button>
          <SettingsActionMenu
            label={I18N.settings.profileActionMore}
            name={sessionName}
            disabled={busy || !pageVisible}
            actions={
              selected
                ? [
                  { id: "rename", label: copy.rename, icon: "captions-bubble", onSelect: onOpenRename },
                  { id: "delete", label: I18N.settings.historyDelete, icon: "trash", destructive: true, onSelect: onRequestDelete },
                ]
                : [
                  {
                    id: "clear",
                    label: I18N.settings.clearSessionArchive,
                    icon: "eraser",
                    destructive: true,
                    disabled: disabled || readError || (!hasTranscript && !hasAudio),
                    onSelect: onClearCurrentArchive,
                  },
                ]
            }
          />
        </div>
      </div>
      {canExportAudio && pageVisible && (
        <SessionAudioPlayer
          key={`${selectedId ?? "current"}:${selectedAudioSource}:${active}`}
          src={active ? null : audioPreview.url}
          loading={audioPreview.loading}
          readError={audioPreview.readError}
          disabled={active || !isTauri || (!selected && readError)}
          onLoad={audioPreview.load}
          sourceControl={sourceControl}
          labels={{
            play: I18N.settings.historyPlayAudio,
            pause: copy.pause,
            loading: copy.loadingAudio,
            retry: copy.retryAudio,
            seek: copy.seek,
            playFailed: copy.playFailed,
            ready: copy.audioReady,
            label: I18N.settings.historyAudio,
          }}
        />
      )}
      {canExportAudio && active && <InlineFeedback tone="info">{copy.stopPlayback}</InlineFeedback>}
      {availableCount > 0 && (
        <div className="session-transcript__toolbar">
          <input
            type="search"
            value={query}
            maxLength={120}
            placeholder={I18N.settings.transcriptSearch}
            aria-label={I18N.settings.transcriptSearch}
            onChange={(event) => onQueryChange(event.target.value)}
          />
          <SettingsSelect
            label={copy.reading}
            value={reading}
            options={contentOptions}
            onChange={(value) => onReadingChange(value as TranscriptExportOptions["content"])}
          />
          {query && displayedTranscript && (
            <span className="session-workspace__search-count">
              {I18N.settings.transcriptMatches(displayedTranscript.total)}
            </span>
          )}
        </div>
      )}
      <div className="session-transcript__list" aria-live="polite">
        {availableCount && transcriptError ? (
          <InlineFeedback tone="error">
            {I18N.settings.transcriptReadFailed}{" "}
            <button
              type="button"
              className="settings-link"
              disabled={!pageVisible || transcriptLoading}
              aria-busy={transcriptLoading || undefined}
              onClick={onRetryTranscript}
            >
              {I18N.settings.retryUpdate}
            </button>
          </InlineFeedback>
        ) : displayedTranscript?.entries.length ? (
          displayedTranscript.entries.map((entry) => (
            <article className="session-transcript__entry" key={entry.index}>
              <div className="session-transcript__meta">
                <span>#{entry.index}</span>
                <time dateTime={new Date(entry.createdAtMs).toISOString()}>
                  {formatTranscriptTime(entry.createdAtMs)}
                </time>
                {entry.audioSource && (
                  <span className="session-transcript__audio-source">
                    {entry.audioSource === "system" ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone}
                  </span>
                )}
                <button
                  type="button"
                  className="session-workspace__copy"
                  aria-label={`${copy.copy} #${entry.index}`}
                  title={copy.copy}
                  disabled={!(reading === "source" ? entry.source : reading === "translation" ? entry.translation : entry.source || entry.translation)}
                  onClick={() => onCopyPassage(entry.source, entry.translation)}
                >
                  <Icon name="copy" />
                </button>
              </div>
              <div>
                <div className={`session-transcript__pair${reading !== "bilingual" ? " session-transcript__pair--single" : ""}`}>
                  {reading !== "translation" && (
                    <div>
                      <span>{I18N.settings.transcriptSource}</span>
                      <p>{entry.source || "—"}</p>
                    </div>
                  )}
                  {reading !== "source" && (
                    <div>
                      <span>{I18N.settings.transcriptTranslation}</span>
                      <p>{entry.translation || "—"}</p>
                    </div>
                  )}
                </div>
              </div>
            </article>
          ))
        ) : (
          <p className="session-transcript__empty">
            {!availableCount
              ? canExportAudio ? I18N.settings.historyAudioOnly : I18N.settings.transcriptEmpty
              : query && !transcriptLoading ? I18N.settings.transcriptNoMatches : I18N.settings.transcriptLoading}
          </p>
        )}
      </div>
      {displayedTranscript && displayedTranscript.total > 30 && (
        <nav className="session-transcript__pagination" aria-label={I18N.settings.transcriptPages}>
          <button
            type="button"
            disabled={displayedTranscript.page === 0}
            onClick={() => onChangePage(displayedTranscript.page - 1)}
            aria-label={I18N.settings.transcriptPrevious}
          >
            <Icon name="chevron-left" />
          </button>
          <span>{I18N.settings.transcriptPage(displayedTranscript.page + 1, Math.ceil(displayedTranscript.total / 30))}</span>
          <button
            type="button"
            disabled={(displayedTranscript.page + 1) * 30 >= displayedTranscript.total}
            onClick={() => onChangePage(displayedTranscript.page + 1)}
            aria-label={I18N.settings.transcriptNext}
          >
            <Icon name="chevron-right" />
          </button>
        </nav>
      )}
    </div>
  );
}
