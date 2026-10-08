import { useEffect, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { Switch } from "../../components/Switch";
import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import {
  isTauri,
  sessionArchiveClear,
  sessionArchiveState,
  sessionExport,
  sessionTranscriptPage,
  sessionHistoryList,
  sessionHistoryPage,
  sessionHistoryAudio,
  sessionHistoryDelete,
} from "../../lib/ipc";
import { useStore } from "../../lib/store";
import type { AudioSource, SessionArchiveState, SessionHistoryItem, SettingsDraft, TranscriptPage } from "../../lib/types";
import {
  InlineFeedback,
  SettingsRow,
  SettingsSelect,
  SettingsSection,
} from "./SettingsPrimitives";
import { monitorSessionArchive } from "./sessionArchiveMonitor";
import { SettingsConfirmation } from "./DestructiveConfirmation";
import { useSettingsToast } from "./useSettingsToast";

export function SessionExport({ visible }: { visible: boolean }) {
  const active = useStore((state) => state.session.isActive);
  const retainHistory = useStore(
    (state) => state.settings.retainSessionHistory,
  );
  const recordAudio = useStore((state) => state.settings.recordSessionAudio);
  const audioInput = useStore((state) => state.settings.audioInput);
  const saveSettings = useStore((state) => state.saveSettings);
  const [archive, setArchive] = useState<SessionArchiveState>();
  const [transcript, setTranscript] = useState<TranscriptPage>();
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [transcriptError, setTranscriptError] = useState(false);
  const [history, setHistory] = useState<SessionHistoryItem[]>([]);
  const [historyError, setHistoryError] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [recordingSource, setRecordingSource] = useState<AudioSource>("system");
  const [audioError, setAudioError] = useState(false);
  const [readError, setReadError] = useState(false);
  const [busy, setBusy] = useState(false);
  const { beginToast, clearToast } = useSettingsToast();
  const monitor = useRef<ReturnType<typeof monitorSessionArchive> | null>(null);
  const operation = useRef(false);
  const lifetime = useRef(0);
  const transcriptRequest = useRef(0);
  const historyRequest = useRef(0);
  const audioRequest = useRef(0);
  const interaction = useRef(0);
  const visibility = useRef(0);

  const selected = history.find((item) => item.id === selectedId);
  const audioSources = selected ? selected.audioSources ?? ["system" as const] : archive?.audioSources ?? ["system" as const];
  const selectedAudioSource = audioSources.includes(recordingSource) ? recordingSource : audioSources[0];
  const availableCount = selectedId ? (selected?.count ?? 0) : (archive?.transcriptCount ?? 0);
  const displayedTranscript = availableCount ? transcript : undefined;

  function selectHistory(id: string | null) {
    if (id === selectedId) return;
    interaction.current += 1;
    clearToast();
    setHistoryError(false);
    audioRequest.current += 1;
    setAudioUrl(null);
    setAudioError(false);
    setTranscriptError(false);
    setConfirmDelete(false);
    setSelectedId(id);
    setQuery("");
    setPage(0);
    setTranscript(undefined);
  }

  function loadAudio(id: string) {
    const request = ++audioRequest.current;
    void sessionHistoryAudio(id, selectedAudioSource).then(
      (bytes) => {
        if (request !== audioRequest.current) return;
        setAudioUrl(URL.createObjectURL(new Blob([bytes], { type: "audio/wav" })));
        setAudioError(false);
      },
      () => { if (request === audioRequest.current) setAudioError(true); },
    );
  }

  useEffect(() => {
    lifetime.current += 1;
    if (isTauri) {
      monitor.current = monitorSessionArchive(
        sessionArchiveState,
        (state) => {
          setArchive(state);
          setReadError(false);
        },
        () => setReadError(true),
      );
    }
    return () => {
      lifetime.current += 1;
      transcriptRequest.current += 1;
      historyRequest.current += 1;
      audioRequest.current += 1;
      monitor.current?.stop();
      monitor.current = null;
    };
  }, []);

  useEffect(() => {
    interaction.current += 1;
    visibility.current += 1;
  }, [visible]);

  useEffect(() => {
    monitor.current?.refresh();
  }, [active, retainHistory, recordAudio]);

  useEffect(() => {
    if (!visible || !isTauri) return;
    const request = ++historyRequest.current;
    void sessionHistoryList().then(
      (items) => {
        if (request !== historyRequest.current) return;
        setHistory(items);
        setHistoryError(false);
      },
      () => { if (request === historyRequest.current) setHistoryError(true); },
    );
    return () => { historyRequest.current += 1; };
  }, [visible, active]);

  useEffect(() => {
    return () => { if (audioUrl) URL.revokeObjectURL(audioUrl); };
  }, [audioUrl]);

  useEffect(() => {
    if (!visible || !isTauri || !availableCount) {
      transcriptRequest.current += 1;
      return;
    }
    const request = ++transcriptRequest.current;
    const timer = setTimeout(() => {
      void (selectedId ? sessionHistoryPage(selectedId, query, page) : sessionTranscriptPage(query, page)).then(
        (result) => {
          if (request !== transcriptRequest.current) return;
          setTranscript(result);
          setTranscriptError(false);
          if (result.page !== page) setPage(result.page);
        },
        () => {
          if (request === transcriptRequest.current) setTranscriptError(true);
        },
      );
    }, query ? 180 : 0);
    return () => {
      transcriptRequest.current += 1;
      clearTimeout(timer);
    };
  }, [visible, availableCount, selectedId, query, page]);

  async function perform(action: () => Promise<"saved" | "cleared" | null>, failureMessage = I18N.settings.sessionExportFailed) {
    if (!isTauri || operation.current || useStore.getState().session.isActive)
      return;
    const currentLifetime = lifetime.current;
    const currentInteraction = interaction.current;
    operation.current = true;
    setBusy(true);
    const notify = beginToast();
    try {
      const result = await action();
      if (result && lifetime.current === currentLifetime && interaction.current === currentInteraction) notify(result === "saved" ? I18N.settings.sessionExportSaved : I18N.settings.sessionArchiveCleared);
    } catch {
      if (lifetime.current === currentLifetime && interaction.current === currentInteraction) notify(failureMessage, true);
    } finally {
      operation.current = false;
      if (lifetime.current === currentLifetime) {
        setBusy(false);
        monitor.current?.refresh();
      }
    }
  }

  async function deleteHistory(id: string) {
    if (!isTauri || operation.current) return;
    const currentLifetime = lifetime.current;
    const currentInteraction = interaction.current;
    const currentVisibility = visibility.current;
    operation.current = true;
    setBusy(true);
    setHistoryError(false);
    const notify = beginToast();
    try {
      await sessionHistoryDelete(id);
      if (lifetime.current !== currentLifetime || visibility.current !== currentVisibility) return;
      setHistory((items) => items.filter((item) => item.id !== id));
      if (interaction.current === currentInteraction) selectHistory(null);
    } catch {
      if (lifetime.current === currentLifetime && interaction.current === currentInteraction) {
        notify(I18N.settings.historyDeleteFailed, true);
      }
    } finally {
      operation.current = false;
      if (lifetime.current === currentLifetime) setBusy(false);
    }
  }

  function changeSetting(draft: SettingsDraft) {
    void perform(async () => {
      await saveSettings(draft);
      return null;
    }, I18N.settings.settingSaveFailed(draft.retainSessionHistory !== undefined ? I18N.settings.retainSessionHistory : I18N.settings.recordSessionAudio));
  }

  const disabled = !isTauri || active || busy;
  const hasTranscript = (archive?.transcriptCount ?? 0) > 0;
  const hasAudio = (archive?.audioBytes ?? 0) > 0;
  const canExportTranscript = selected ? selected.count > 0 : hasTranscript;
  const canExportAudio = selected ? selected.hasAudio : hasAudio;
  const showCurrent = active || hasTranscript || hasAudio;
  const showContent = selected || showCurrent;

  return (
    <SettingsSection
      id="session-export"
      title={I18N.settings.sessionExportTitle}
      hideHeading
    >
      <SettingsRow
        label={I18N.settings.retainSessionHistory}
        description={I18N.settings.retainSessionHistoryHelp}
        align="start"
      >
        <Switch
          checked={retainHistory}
          disabled={disabled}
          aria-label={I18N.settings.retainSessionHistory}
          onChange={(retainSessionHistory) =>
            changeSetting({ retainSessionHistory })
          }
        />
      </SettingsRow>
      <div className="settings-divider" />
      <SettingsRow
        label={I18N.settings.recordSessionAudio}
        description={I18N.settings.recordSessionAudioHelp}
        align="start"
      >
        <Switch
          checked={recordAudio}
          disabled={disabled}
          aria-label={I18N.settings.recordSessionAudio}
          onChange={(recordSessionAudio) =>
            changeSetting({ recordSessionAudio })
          }
        />
      </SettingsRow>
      <p className="session-export__privacy">
        <Icon name="shield-check" />
        <span>{I18N.settings.sessionExportEphemeral}</span>
      </p>
      <div className="session-export__details">
        {active && (
          <p className="settings-help">{I18N.settings.sessionExportActive}</p>
        )}
        {active && recordAudio && (
          <InlineFeedback tone="info">
            {audioInput === "both" ? I18N.settings.sessionBothAudioEnabled : audioInput === "microphone" ? I18N.settings.sessionMicrophoneEnabled : I18N.settings.sessionAudioEnabled}
          </InlineFeedback>
        )}
        {archive?.transcriptLimited && (
          <InlineFeedback tone="info">
            {I18N.settings.sessionTranscriptLimit}
          </InlineFeedback>
        )}
        {archive?.audioLimited && (
          <InlineFeedback tone="info">
            {I18N.settings.sessionAudioLimit}
          </InlineFeedback>
        )}
        {archive?.historySaveError && (
          <InlineFeedback tone="error">{I18N.settings.historySaveFailed}</InlineFeedback>
        )}
        <div className="session-history">
          <div className="session-history__heading">
            <h3>{I18N.settings.historyTitle}</h3>
            {history.length > 0 && <span>{I18N.settings.historySessionCount(history.length)}</span>}
          </div>
          {showCurrent && <button type="button" className={`session-history__item${selectedId === null ? " is-selected" : ""}`} onClick={() => selectHistory(null)}>
            <span>{I18N.settings.historyCurrent}</span>
            {(hasTranscript || hasAudio) && <small>{hasTranscript ? I18N.settings.transcriptCount(archive?.transcriptCount ?? 0) : ""}{hasTranscript && hasAudio ? " · " : ""}{hasAudio ? I18N.settings.historyAudio : ""}</small>}
          </button>}
          {groupHistory(history).map((group) => (
            <div className="session-history__group" key={group.day}>
              <h4>{group.label}</h4>
              {group.items.map((item) => (
                <button type="button" key={item.id} className={`session-history__item${selectedId === item.id ? " is-selected" : ""}`} onClick={() => selectHistory(item.id)}>
                  <span>{new Intl.DateTimeFormat(effectiveUiLanguage(), { hour: "2-digit", minute: "2-digit" }).format(item.startedAtMs)}</span>
                  <small>{item.count > 0 ? I18N.settings.transcriptCount(item.count) : ""}{item.count > 0 && item.hasAudio ? " · " : ""}{item.hasAudio ? I18N.settings.historyAudio : ""}</small>
                </button>
              ))}
            </div>
          ))}
          {historyError && <InlineFeedback tone="error">{I18N.settings.historyReadFailed}</InlineFeedback>}
        </div>
        {showContent ? <div className="session-transcript">
          <div className="session-transcript__heading">
            <div>
              <h3>{selected ? I18N.settings.historySelectedTitle : I18N.settings.transcriptBrowseTitle}</h3>
            </div>
            <span>{I18N.settings.transcriptCount(availableCount)}</span>
          </div>
          {canExportAudio && audioSources.length > 1 && <SettingsRow label={I18N.settings.recordingSource}>
            <SettingsSelect label={I18N.settings.recordingSource} value={selectedAudioSource ?? "system"} disabled={busy}
              options={audioSources.map(source => ({ value: source, label: source === "system" ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone }))}
              onChange={value => {
                audioRequest.current += 1;
                setRecordingSource(value as AudioSource);
                setAudioUrl(null);
                setAudioError(false);
              }} />
          </SettingsRow>}
          {selected && <div className="session-history__actions" aria-busy={busy}>
            {selected.hasAudio && (audioUrl ? <audio controls src={audioUrl} aria-label={I18N.settings.historyAudio} /> : <button type="button" className="settings-button settings-button--quiet" onClick={() => loadAudio(selected.id)}>{I18N.settings.historyPlayAudio}</button>)}
            <button type="button" className="settings-button settings-button--danger settings-button--compact" disabled={busy} onClick={() => setConfirmDelete(true)}>
              <Icon name="trash" />{I18N.settings.historyDelete}
            </button>
            {confirmDelete && visible && <SettingsConfirmation
              message={I18N.settings.historyDeleteConfirm}
              confirmLabel={I18N.settings.historyDelete}
              disabled={busy}
              onCancel={() => setConfirmDelete(false)}
              onConfirm={() => { void deleteHistory(selected.id); }}
            >
              {historyError && <InlineFeedback tone="error">{I18N.settings.historyReadFailed}</InlineFeedback>}
            </SettingsConfirmation>}
          </div>}
          {audioError && <InlineFeedback tone="error">{I18N.settings.historyAudioFailed}</InlineFeedback>}
          {availableCount > 0 && <div className="session-transcript__toolbar">
            <input
              type="search"
              value={query}
              maxLength={120}
              placeholder={I18N.settings.transcriptSearch}
              aria-label={I18N.settings.transcriptSearch}
              onChange={(event) => { setQuery(event.target.value); setPage(0); setTranscript(undefined); setTranscriptError(false); }}
            />
            {displayedTranscript && displayedTranscript.total > 0 && (
              <span>{I18N.settings.transcriptMatches(displayedTranscript.total)}</span>
            )}
          </div>}
          <div className="session-transcript__list" aria-live="polite">
            {availableCount && transcriptError ? (
              <p className="session-transcript__empty">{I18N.settings.transcriptReadFailed}</p>
            ) : displayedTranscript?.entries.length ? displayedTranscript.entries.map((entry) => (
              <article className="session-transcript__entry" key={entry.index}>
                <div className="session-transcript__meta">
                  <span>#{entry.index}</span>
                  <time dateTime={new Date(entry.createdAtMs).toISOString()}>
                    {new Intl.DateTimeFormat(effectiveUiLanguage(), { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(entry.createdAtMs)}
                  </time>
                </div>
                <div>
                  {entry.audioSource && <div className="session-transcript__audio-source">{entry.audioSource === "system" ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone}</div>}
                <div className="session-transcript__pair">
                  <div><span>{I18N.settings.transcriptSource}</span><p>{entry.source}</p></div>
                  <div><span>{I18N.settings.transcriptTranslation}</span><p>{entry.translation}</p></div>
                </div>
                </div>
              </article>
            )) : (
              <p className="session-transcript__empty">
                {!availableCount ? (canExportAudio ? I18N.settings.historyAudioOnly : I18N.settings.transcriptEmpty) : query ? I18N.settings.transcriptNoMatches : I18N.settings.transcriptLoading}
              </p>
            )}
          </div>
          {displayedTranscript && displayedTranscript.total > 30 && (
            <nav className="session-transcript__pagination" aria-label={I18N.settings.transcriptPages}>
              <button type="button" disabled={displayedTranscript.page === 0} onClick={() => { setPage(displayedTranscript.page - 1); setTranscript(undefined); }} aria-label={I18N.settings.transcriptPrevious}><Icon name="chevron-left" /></button>
              <span>{I18N.settings.transcriptPage(displayedTranscript.page + 1, Math.ceil(displayedTranscript.total / 30))}</span>
              <button type="button" disabled={(displayedTranscript.page + 1) * 30 >= displayedTranscript.total} onClick={() => { setPage(displayedTranscript.page + 1); setTranscript(undefined); }} aria-label={I18N.settings.transcriptNext}><Icon name="chevron-right" /></button>
            </nav>
          )}
        </div> : <p className="session-export__empty">{history.length > 0 ? I18N.settings.historyChoose : I18N.settings.historyEmpty}</p>}
        {showContent && <div className="session-export__actions">
          <button
            type="button"
            className="settings-button settings-button--quiet"
            disabled={disabled || (!selected && readError) || !canExportTranscript}
            onClick={() =>
              void perform(async () =>
                (await sessionExport("transcript", selected?.id)) ? "saved" : null,
              )
            }
          >
            <Icon name="download" />
            {I18N.settings.exportTranscript}
          </button>
          <button
            type="button"
            className="settings-button settings-button--quiet"
            disabled={disabled || (!selected && readError) || !canExportAudio}
            onClick={() =>
              void perform(async () =>
                (await sessionExport("audio", selected?.id, selectedAudioSource)) ? "saved" : null,
              )
            }
          >
            <Icon name="download" />
            {I18N.settings.exportAudio}
          </button>
          {!selected && <button
            type="button"
            className="settings-button settings-button--text"
            disabled={disabled || readError || (!hasTranscript && !hasAudio)}
            onClick={() =>
              void perform(async () => {
                await sessionArchiveClear();
                return "cleared";
              })
            }
          >
            {I18N.settings.clearSessionArchive}
          </button>}
        </div>}
        {!isTauri && (
          <p className="settings-help">
            {I18N.settings.sessionExportNativeOnly}
          </p>
        )}
        {readError && (
          <InlineFeedback tone="error">
            {I18N.settings.sessionArchiveReadFailed}
          </InlineFeedback>
        )}
      </div>
      <details className="session-export__explanation">
        <summary>
          {I18N.settings.sessionExportDetails}
          <Icon name="chevron-down" />
        </summary>
        <div>
          <p>{I18N.settings.sessionExportPrivacy}</p>
          <p>{I18N.settings.sessionArchiveLimits}</p>
          <p>{I18N.settings.sessionTranscriptTiming}</p>
          <p>{I18N.settings.sessionAudioTiming}</p>
        </div>
      </details>
    </SettingsSection>
  );
}

function groupHistory(items: SessionHistoryItem[]) {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  const tomorrow = new Date(today);
  tomorrow.setDate(tomorrow.getDate() + 1);
  const groups = new Map<string, { day: string; label: string; items: SessionHistoryItem[] }>();
  for (const item of items) {
    const date = new Date(item.startedAtMs);
    const day = `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
    const label = date >= today && date < tomorrow ? I18N.settings.historyToday
      : date >= yesterday && date < today ? I18N.settings.historyYesterday
      : new Intl.DateTimeFormat(effectiveUiLanguage(), { year: "numeric", month: "long", day: "numeric" }).format(date);
    if (!groups.has(day)) groups.set(day, { day, label, items: [] });
    groups.get(day)!.items.push(item);
  }
  return [...groups.values()];
}
