import { useMemo } from "react";
import { Icon } from "../../../components/Icon";
import { I18N, type ResolvedUiLanguage } from "../../../lib/i18n";
import type { SessionArchiveState, SessionHistoryItem } from "../../../lib/types";
import { InlineFeedback, SettingsSelect } from "../controls/SettingsPrimitives";
import { SettingsHelp } from "../feedback/SettingsHelp";
import type { sessionWorkspaceCopy } from "./sessionWorkspaceCopy";

export type SessionWorkspaceStrings = (typeof sessionWorkspaceCopy)[ResolvedUiLanguage];

interface SessionHistoryLibraryProps {
  history: SessionHistoryItem[];
  archive: SessionArchiveState | undefined;
  selectedId: string | null;
  showCurrent: boolean;
  hasTranscript: boolean;
  hasAudio: boolean;
  busy: boolean;
  pageVisible: boolean;
  historyError: boolean;
  historyLoading: boolean;
  historyQuery: string;
  historyFilter: string;
  selectMode: boolean;
  selectedHistoryIds: string[];
  language: ResolvedUiLanguage;
  copy: SessionWorkspaceStrings;
  onHistoryQueryChange: (query: string) => void;
  onHistoryFilterChange: (filter: string) => void;
  onSelectHistory: (id: string | null) => void;
  onRetryHistory: () => void;
  onSelectModeChange: (selectMode: boolean) => void;
  onSelectedHistoryIdsChange: (updater: string[] | ((current: string[]) => string[])) => void;
  onOpenBatchDelete: (mode: "selected" | "all", ids: string[]) => void;
  onClearBatchDelete: () => void;
}

export function SessionHistoryLibrary({
  history,
  archive,
  selectedId,
  showCurrent,
  hasTranscript,
  hasAudio,
  busy,
  pageVisible,
  historyError,
  historyLoading,
  historyQuery,
  historyFilter,
  selectMode,
  selectedHistoryIds,
  language,
  copy,
  onHistoryQueryChange,
  onHistoryFilterChange,
  onSelectHistory,
  onRetryHistory,
  onSelectModeChange,
  onSelectedHistoryIdsChange,
  onOpenBatchDelete,
  onClearBatchDelete,
}: SessionHistoryLibraryProps) {
  const filteredHistory = history.filter((item) =>
    (!historyQuery.trim() || (item.title ?? "").toLocaleLowerCase(language).includes(historyQuery.trim().toLocaleLowerCase(language)))
    && (historyFilter === "all" || (historyFilter === "text" ? item.count > 0 : item.hasAudio)),
  );
  const existingHistoryIds = useMemo(() => new Set(history.map((item) => item.id)), [history]);
  const effectiveSelectedIds = useMemo(
    () => selectedHistoryIds.filter((id) => existingHistoryIds.has(id)),
    [selectedHistoryIds, existingHistoryIds],
  );
  const selectedIdSet = useMemo(() => new Set(effectiveSelectedIds), [effectiveSelectedIds]);
  const activeSelectMode = selectMode && history.length > 0;
  const allFilteredSelected = filteredHistory.length > 0 && filteredHistory.every((item) => selectedIdSet.has(item.id));

  function toggleSelectMode() {
    if (busy) return;
    if (activeSelectMode) {
      onSelectModeChange(false);
      onSelectedHistoryIdsChange([]);
      onClearBatchDelete();
    } else if (history.length > 0) {
      onSelectModeChange(true);
      onSelectedHistoryIdsChange([]);
    }
  }

  function toggleSelectedHistory(id: string) {
    if (busy) return;
    onSelectedHistoryIdsChange((items) => {
      const valid = items.filter((entry) => existingHistoryIds.has(entry));
      return valid.includes(id) ? valid.filter((entry) => entry !== id) : [...valid, id];
    });
  }

  function toggleSelectAllFiltered() {
    if (busy || filteredHistory.length === 0) return;
    const filteredIds = filteredHistory.map((item) => item.id);
    const filteredSet = new Set(filteredIds);
    onSelectedHistoryIdsChange((items) => {
      const valid = items.filter((entry) => existingHistoryIds.has(entry));
      if (allFilteredSelected) {
        return valid.filter((entry) => !filteredSet.has(entry));
      }
      const next = new Set(valid);
      for (const id of filteredIds) next.add(id);
      return [...next];
    });
  }

  function openBatchDelete(mode: "selected" | "all") {
    if (busy) return;
    const ids = mode === "all" ? history.map((item) => item.id) : effectiveSelectedIds;
    if (ids.length === 0) return;
    onOpenBatchDelete(mode, ids);
  }

  // Reuse each format only within this render. Later polls still observe
  // language/default-time-zone changes, and empty lists need no formatter.
  let sessionTimeFormatter: Intl.DateTimeFormat | undefined;
  const formatSessionTime = (timestamp: number) =>
    (sessionTimeFormatter ??= new Intl.DateTimeFormat(language, { hour: "2-digit", minute: "2-digit" })).format(timestamp);

  return (
    <div className="session-history">
      <div className="session-history__heading">
        <div className="session-history__heading-meta">
          <h3>
            {I18N.settings.historyTitle}
            <SettingsHelp label={I18N.settings.helpLabel} text={I18N.settings.sessionExportEphemeral} />
          </h3>
          {history.length > 0 && <span>{I18N.settings.historySessionCount(history.length)}</span>}
        </div>
        {history.length > 0 && (
          <div className="session-history__heading-actions">
            {activeSelectMode ? (
              <button
                type="button"
                className="settings-button settings-button--quiet"
                disabled={busy || !pageVisible}
                onClick={toggleSelectMode}
              >
                {copy.done}
              </button>
            ) : (
              <>
                <button
                  type="button"
                  className="settings-button settings-button--quiet"
                  disabled={busy || !pageVisible}
                  onClick={toggleSelectMode}
                >
                  {copy.select}
                </button>
                <button
                  type="button"
                  className="settings-button settings-button--danger"
                  disabled={busy || !pageVisible}
                  onClick={() => openBatchDelete("all")}
                >
                  <Icon name="trash" />
                  {copy.clearAll}
                </button>
              </>
            )}
          </div>
        )}
      </div>
      {history.length > 0 && (
        <div className="session-workspace__library-tools">
          <input
            type="search"
            value={historyQuery}
            aria-label={copy.search}
            placeholder={copy.search}
            maxLength={120}
            onChange={(event) => onHistoryQueryChange(event.target.value)}
          />
          <SettingsSelect
            label={copy.filter}
            value={historyFilter}
            onChange={onHistoryFilterChange}
            options={[
              { value: "all", label: copy.all },
              { value: "text", label: copy.text },
              { value: "audio", label: copy.audio },
            ]}
          />
        </div>
      )}
      {activeSelectMode && (
        <div className="session-workspace__selection-bar">
          <div className="session-workspace__selection-bar-group">
            <button
              type="button"
              className="settings-button settings-button--quiet"
              disabled={busy || !pageVisible || filteredHistory.length === 0}
              onClick={toggleSelectAllFiltered}
            >
              {allFilteredSelected ? copy.deselectAll : copy.selectAll}
            </button>
            <span className="session-workspace__selected-count">{copy.selectedCount(effectiveSelectedIds.length)}</span>
          </div>
          <div className="session-workspace__selection-bar-actions">
            <button
              type="button"
              className="settings-button settings-button--danger"
              disabled={busy || !pageVisible || effectiveSelectedIds.length === 0}
              onClick={() => openBatchDelete("selected")}
            >
              <Icon name="trash" />
              {copy.deleteSelected(effectiveSelectedIds.length)}
            </button>
          </div>
        </div>
      )}
      <div className="session-workspace__catalog">
        {!activeSelectMode && showCurrent && (
          <button
            type="button"
            data-history-id="current"
            aria-pressed={selectedId === null}
            className={`session-history__item${selectedId === null ? " is-selected" : ""}`}
            onClick={() => onSelectHistory(null)}
          >
            <span>{I18N.settings.historyCurrent}</span>
            {(hasTranscript || hasAudio) && (
              <small>
                {hasTranscript ? I18N.settings.transcriptCount(archive?.transcriptCount ?? 0) : ""}
                {hasTranscript && hasAudio ? " · " : ""}
                {hasAudio ? I18N.settings.historyAudio : ""}
              </small>
            )}
          </button>
        )}
        {groupHistory(filteredHistory, language).map((group) => (
          <div className="session-history__group" key={group.day}>
            <h4>{group.label}</h4>
            {group.items.map((item) =>
              activeSelectMode ? (
                <button
                  type="button"
                  key={item.id}
                  role="checkbox"
                  aria-checked={selectedIdSet.has(item.id)}
                  data-history-id={item.id}
                  disabled={busy}
                  className={`session-history__item session-history__item--selectable${selectedIdSet.has(item.id) ? " is-checked" : ""}`}
                  onClick={() => toggleSelectedHistory(item.id)}
                >
                  <span className="session-workspace__item-lead">
                    <span className="session-workspace__checkbox" aria-hidden="true">
                      {selectedIdSet.has(item.id) && <Icon name="checkmark" />}
                    </span>
                    <span className="session-workspace__session-name">
                      {item.title || formatSessionTime(item.startedAtMs)}
                      {item.title && <time>{formatSessionTime(item.startedAtMs)}</time>}
                    </span>
                  </span>
                  <small>
                    {item.count > 0 ? I18N.settings.transcriptCount(item.count) : ""}
                    {item.count > 0 && item.hasAudio ? " · " : ""}
                    {item.hasAudio ? I18N.settings.historyAudio : ""}
                  </small>
                </button>
              ) : (
                <button
                  type="button"
                  key={item.id}
                  data-history-id={item.id}
                  aria-pressed={selectedId === item.id}
                  className={`session-history__item${selectedId === item.id ? " is-selected" : ""}`}
                  onClick={() => onSelectHistory(item.id)}
                >
                  <span className="session-workspace__session-name">
                    {item.title || formatSessionTime(item.startedAtMs)}
                    {item.title && <time>{formatSessionTime(item.startedAtMs)}</time>}
                  </span>
                  <small>
                    {item.count > 0 ? I18N.settings.transcriptCount(item.count) : ""}
                    {item.count > 0 && item.hasAudio ? " · " : ""}
                    {item.hasAudio ? I18N.settings.historyAudio : ""}
                  </small>
                </button>
              ),
            )}
          </div>
        ))}
      </div>
      {history.length > 0 && filteredHistory.length === 0 && (
        <p className="session-transcript__empty">{copy.noMatches}</p>
      )}
      {historyError && (
        <InlineFeedback tone="error">
          {I18N.settings.historyReadFailed}{" "}
          <button
            type="button"
            className="settings-link"
            disabled={!pageVisible || historyLoading}
            aria-busy={historyLoading || undefined}
            onClick={onRetryHistory}
          >
            {I18N.settings.retryUpdate}
          </button>
        </InlineFeedback>
      )}
      {!showCurrent && history.length === 0 && !historyError && (
        <p className="session-export__empty">{I18N.settings.historyEmpty}</p>
      )}
    </div>
  );
}

function groupHistory(items: SessionHistoryItem[], language: string) {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const tomorrow = new Date(today);
  tomorrow.setDate(tomorrow.getDate() + 1);
  const groups = new Map<string, { day: string; label: string; items: SessionHistoryItem[] }>();
  let dateFormatter: Intl.DateTimeFormat | undefined;
  for (const item of items) {
    const date = new Date(item.startedAtMs);
    const day = `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
    if (!groups.has(day)) {
      const label = date >= today && date < tomorrow ? I18N.settings.historyToday
        : date >= yesterday && date < today ? I18N.settings.historyYesterday
        : (dateFormatter ??= new Intl.DateTimeFormat(language, { year: "numeric", month: "long", day: "numeric" })).format(date);
      groups.set(day, { day, label, items: [] });
    }
    groups.get(day)!.items.push(item);
  }
  return [...groups.values()];
}
