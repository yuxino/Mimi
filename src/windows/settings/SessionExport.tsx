import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
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
  sessionArchiveAudio,
  sessionHistoryDelete,
  sessionHistoryRename,
} from "../../lib/ipc";
import { useStore } from "../../lib/store";
import { useDocumentVisibility } from "../../lib/useDocumentVisibility";
import type { AudioSource, SessionArchiveState, SessionHistoryItem, SettingsDraft, TranscriptPage, TranscriptExportOptions } from "../../lib/types";
import { writeDiagnosticClipboard } from "../../lib/diagnosticClipboard";
import {
  InlineFeedback,
  SettingsRow,
  SettingsSelect,
  SettingsSection,
} from "./SettingsPrimitives";
import { monitorSessionArchive, sameSessionArchiveState } from "./sessionArchiveMonitor";
import { SettingsConfirmation } from "./DestructiveConfirmation";
import { useSettingsToast } from "./useSettingsToast";
import { SettingsActionMenu } from "./SettingsActionMenu";
import { SessionAudioPlayer } from "./SessionAudioPlayer";
import { SettingsHelp } from "./SettingsHelp";
import { sessionWorkspaceCopy } from "./sessionWorkspaceCopy";
import "./session-workspace.css";

function archivePresentationKey(hasDates: boolean): string {
  // Polling also owns calendar refresh. Keep midnight, OS time-zone and locale
  // changes visible even when archive sizes and limits have not changed.
  return hasDates
    ? `${effectiveUiLanguage()}:${new Date().toDateString()}:${new Intl.DateTimeFormat().resolvedOptions().timeZone}`
    : effectiveUiLanguage();
}

export function SessionExport({ visible, onReaderChange }: { visible: boolean; onReaderChange?: (open: boolean) => void }) {
  const documentVisible = useDocumentVisibility();
  const pageVisible = visible && documentVisible;
  const active = useStore((state) => state.session.isActive);
  const retainHistory = useStore(
    (state) => state.settings.retainSessionHistory,
  );
  const recordAudio = useStore((state) => state.settings.recordSessionAudio);
  const audioInput = useStore((state) => state.settings.audioInput);
  const saveSettings = useStore((state) => state.saveSettings);
  const [archive, setArchive] = useState<SessionArchiveState>();
  const receivedArchive = useRef<SessionArchiveState>(undefined);
  const archiveReadFailed = useRef(false);
  const [transcript, setTranscript] = useState<{ owner: symbol | string; page: TranscriptPage }>();
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [transcriptError, setTranscriptError] = useState(false);
  const [settledTranscriptRead, setSettledTranscriptRead] = useState<object | null>(null);
  const [transcriptRefresh, setTranscriptRefresh] = useState(0);
  const [history, setHistory] = useState<SessionHistoryItem[]>([]);
  const [historyRefresh, setHistoryRefresh] = useState(0);
  const [historyError, setHistoryError] = useState(false);
  const [settledHistoryRead, setSettledHistoryRead] = useState<object | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [readerOpen, setReaderOpen] = useState(false);
  const [exportAudio, setExportAudio] = useState(false);
  const workspace = useRef<HTMLDivElement>(null);
  const readerHeading = useRef<HTMLHeadingElement>(null);
  const catalogScroll = useRef({ page: 0, list: 0 });
  const previousReaderOpen = useRef(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [audioUrl, setAudioUrl] = useState<string | null>(null);
  const [recordingSource, setRecordingSource] = useState<AudioSource>("system");
  const [audioError, setAudioError] = useState(false);
  const [audioLoading, setAudioLoading] = useState(false);
  const [readError, setReadError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [historyQuery, setHistoryQuery] = useState("");
  const [historyFilter, setHistoryFilter] = useState("all");
  const [reading, setReading] = useState<TranscriptExportOptions["content"]>("bilingual");
  const [exportOpen, setExportOpen] = useState(false);
  const [exportOptions, setExportOptions] = useState<TranscriptExportOptions>({ format: "txt", content: "bilingual", includeTimestamps: false });
  const [renameOpen, setRenameOpen] = useState(false);
  const [nameDraft, setNameDraft] = useState("");
  const [renameError, setRenameError] = useState(false);
  const [manualCopy, setManualCopy] = useState<string | null>(null);
  const { beginToast, clearToast } = useSettingsToast();
  const monitor = useRef<ReturnType<typeof monitorSessionArchive> | null>(null);
  const operation = useRef(false);
  const lifetime = useRef(0);
  const transcriptRequest = useRef(0);
  const requestedQuery = useRef("");
  const historyRequest = useRef(0);
  const audioRequest = useRef(0);
  const audioInFlight = useRef(false);
  const interaction = useRef(0);
  const selectedHistoryId = useRef<string | null>(null);
  const [renderedAudioContext, setRenderedAudioContext] = useState({ active, pageVisible, selectedId, recordAudio });
  if (renderedAudioContext.active !== active || renderedAudioContext.pageVisible !== pageVisible
    || renderedAudioContext.selectedId !== selectedId || renderedAudioContext.recordAudio !== recordAudio) {
    setRenderedAudioContext({ active, pageVisible, selectedId, recordAudio });
    if (active || !pageVisible || (!selectedId && !recordAudio)) {
      setAudioUrl(null);
      setAudioError(false);
    }
  }
  // Counts can match across a reset. Current reads belong to an exact local
  // lifecycle, while saved-history reads remain independent of live capture.
  const archiveContext = useMemo(() => Symbol(`${active}:${retainHistory}`), [active, retainHistory]);
  const archivePresentation = useRef({ hasDates: false, key: "" });
  const latestArchiveContext = useRef(archiveContext);
  useLayoutEffect(() => { latestArchiveContext.current = archiveContext; }, [archiveContext]);
  const transcriptContext = selectedId ?? archiveContext;
  const transcriptEnabled = selectedId !== null || retainHistory;
  const [renderedArchiveContext, setRenderedArchiveContext] = useState(archiveContext);
  if (renderedArchiveContext !== archiveContext) {
    setRenderedArchiveContext(archiveContext);
    if (selectedId === null) {
      setTranscript(undefined);
      setTranscriptError(false);
    }
  }

  const selected = history.find((item) => item.id === selectedId);
  const audioSources = selected ? selected.audioSources ?? ["system" as const] : archive?.audioSources ?? ["system" as const];
  const selectedAudioSource = audioSources.includes(recordingSource) ? recordingSource : audioSources[0];
  const availableCount = selectedId ? (selected?.count ?? 0) : (archive?.transcriptCount ?? 0);
  const displayedTranscript = availableCount && transcript?.owner === transcriptContext ? transcript.page : undefined;
  useLayoutEffect(() => {
    const hasDates = readerOpen ? Boolean(selected || displayedTranscript?.entries.length) : history.length > 0;
    archivePresentation.current = { hasDates, key: archivePresentationKey(hasDates) };
  });
  // A changed read is pending during render, before its effect starts. Late
  // completions can settle only that visit's identity, never a newer retry.
  const historyRead = useMemo(() => ({ pageVisible, active, historyRefresh }), [pageVisible, active, historyRefresh]);
  const transcriptReadKey = JSON.stringify([pageVisible, readerOpen, availableCount, selectedId, query, page, transcriptEnabled, transcriptRefresh]);
  const transcriptRead = useMemo(() => ({ owner: transcriptContext, key: transcriptReadKey }), [transcriptContext, transcriptReadKey]);
  const historyLoading = pageVisible && settledHistoryRead !== historyRead;
  const transcriptLoading = pageVisible && readerOpen && availableCount > 0 && transcriptEnabled && settledTranscriptRead !== transcriptRead;

  useLayoutEffect(() => {
    onReaderChange?.(readerOpen);
    if (previousReaderOpen.current === readerOpen) return;
    previousReaderOpen.current = readerOpen;
    const scroll = workspace.current?.closest(".settings-console__scroll");
    if (scroll) scroll.scrollTop = readerOpen ? 0 : catalogScroll.current.page;
    if (readerOpen) readerHeading.current?.focus({ preventScroll: true });
    else {
      const catalog = workspace.current?.querySelector(".session-workspace__catalog");
      if (catalog) catalog.scrollTop = catalogScroll.current.list;
      const target = [...workspace.current?.querySelectorAll<HTMLButtonElement>("[data-history-id]") ?? []]
        .find(node => node.dataset.historyId === (selectedHistoryId.current ?? "current"));
      target?.focus({ preventScroll: true });
    }
  }, [readerOpen, onReaderChange]);

  function returnToLibrary() {
    interaction.current += 1;
    audioRequest.current += 1;
    setAudioUrl(null);
    setAudioError(false);
    setReaderOpen(false);
    clearToast();
  }

  function selectHistory(id: string | null) {
    if (!readerOpen) {
      catalogScroll.current = {
        page: workspace.current?.closest(".settings-console__scroll")?.scrollTop ?? 0,
        list: workspace.current?.querySelector(".session-workspace__catalog")?.scrollTop ?? 0,
      };
    }
    setReaderOpen(true);
    if (id === selectedHistoryId.current) return;
    selectedHistoryId.current = id;
    interaction.current += 1;
    clearToast();
    transcriptRequest.current += 1;
    audioRequest.current += 1;
    setAudioUrl(null);
    setAudioError(false);
    setTranscriptError(false);
    setConfirmDelete(false);
    setRenameOpen(false);
    setExportOpen(false);
    setManualCopy(null);
    setSelectedId(id);
    setQuery("");
    setPage(0);
    setTranscript(undefined);
  }

  function loadAudio() {
    // One WAV may be 64 MiB. Superseding its result must not start another
    // full-file transfer while the native read is still in flight.
    if (audioInFlight.current || !pageVisible || !readerOpen || useStore.getState().session.isActive) return;
    audioInFlight.current = true;
    const currentLifetime = lifetime.current;
    const request = ++audioRequest.current;
    setAudioLoading(true);
    setAudioError(false);
    const read = selectedId ? sessionHistoryAudio(selectedId, selectedAudioSource) : sessionArchiveAudio(selectedAudioSource);
    void read.then(
      (bytes) => {
        if (request !== audioRequest.current || useStore.getState().session.isActive) return;
        setAudioUrl(URL.createObjectURL(new Blob([bytes], { type: "audio/wav" })));
        setAudioError(false);
      },
      () => { if (request === audioRequest.current) setAudioError(true); },
    ).finally(() => {
      audioInFlight.current = false;
      if (lifetime.current === currentLifetime) setAudioLoading(false);
    });
  }

  useEffect(() => {
    lifetime.current += 1;
    return () => {
      lifetime.current += 1;
      transcriptRequest.current += 1;
      historyRequest.current += 1;
      audioRequest.current += 1;
    };
  }, []);

  useEffect(() => {
    if (!pageVisible || !isTauri) return;
    const current = monitorSessionArchive(
      sessionArchiveState,
      (state) => {
        const key = archivePresentationKey(archivePresentation.current.hasDates);
        const presentationChanged = key !== archivePresentation.current.key;
        archivePresentation.current.key = key;
        if (presentationChanged || !sameSessionArchiveState(receivedArchive.current, state)) {
          receivedArchive.current = state;
          setArchive(presentationChanged ? { ...state } : state);
        }
        if (archiveReadFailed.current) {
          archiveReadFailed.current = false;
          setReadError(false);
        }
      },
      () => {
        if (!archiveReadFailed.current) {
          archiveReadFailed.current = true;
          setReadError(true);
        }
      },
    );
    monitor.current = current;
    return () => {
      current.stop();
      if (monitor.current === current) monitor.current = null;
    };
  }, [pageVisible]);

  useEffect(() => {
    interaction.current += 1;
  }, [pageVisible]);

  useEffect(() => {
    monitor.current?.refresh();
  }, [active, retainHistory, recordAudio]);

  useEffect(() => {
    if (active || !pageVisible || (!selectedId && !recordAudio)) {
      audioRequest.current += 1;
    }
  }, [active, pageVisible, selectedId, recordAudio]);

  useEffect(() => {
    if (!pageVisible || !isTauri) return;
    const request = ++historyRequest.current;
    void sessionHistoryList().then(
      (items) => {
        if (request !== historyRequest.current) return;
        setHistory(items);
        setHistoryError(false);
      },
      () => { if (request === historyRequest.current) setHistoryError(true); },
    ).finally(() => { if (request === historyRequest.current) setSettledHistoryRead(historyRead); });
    return () => { historyRequest.current += 1; };
  }, [pageVisible, historyRead]);

  useEffect(() => {
    return () => { if (audioUrl) URL.revokeObjectURL(audioUrl); };
  }, [audioUrl]);

  useEffect(() => {
    if (!pageVisible || !readerOpen || !isTauri || !availableCount || !transcriptEnabled) {
      transcriptRequest.current += 1;
      return;
    }
    const request = ++transcriptRequest.current;
    const timer = setTimeout(() => {
      requestedQuery.current = query;
      void (selectedId ? sessionHistoryPage(selectedId, query, page) : sessionTranscriptPage(query, page)).then(
        (result) => {
          if (request !== transcriptRequest.current) return;
          setTranscript({ owner: transcriptContext, page: result });
          setTranscriptError(false);
          if (result.page !== page) setPage(result.page);
        },
        () => {
          if (request === transcriptRequest.current) setTranscriptError(true);
        },
      ).finally(() => { if (request === transcriptRequest.current) setSettledTranscriptRead(transcriptRead); });
    // Coalesce typing only. Pagination, retries and archive updates for the
    // already requested query should not pay the search delay again.
    }, query && query !== requestedQuery.current ? 180 : 0);
    return () => {
      transcriptRequest.current += 1;
      clearTimeout(timer);
    };
  }, [pageVisible, readerOpen, availableCount, selectedId, query, page, transcriptEnabled, transcriptContext, transcriptRead]);

  async function perform(action: () => Promise<"saved" | "cleared" | null>, failureMessage = I18N.settings.sessionExportFailed, requiresStopped = true) {
    if (!isTauri || operation.current || (requiresStopped && useStore.getState().session.isActive))
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

  async function renameHistory(id: string) {
    if (!isTauri || operation.current) return;
    const currentLifetime = lifetime.current;
    const currentInteraction = interaction.current;
    const title = nameDraft.trim();
    operation.current = true;
    setBusy(true);
    setRenameError(false);
    const notify = beginToast();
    try {
      await sessionHistoryRename(id, title);
      if (lifetime.current !== currentLifetime) return;
      historyRequest.current += 1;
      setHistory(items => items.map(item => item.id === id ? { ...item, title: title || undefined } : item));
      setHistoryRefresh(value => value + 1);
      if (interaction.current === currentInteraction) {
        setRenameOpen(false);
        notify(copy.renamed);
      }
    } catch {
      if (lifetime.current === currentLifetime && interaction.current === currentInteraction) setRenameError(true);
    } finally {
      operation.current = false;
      if (lifetime.current === currentLifetime) setBusy(false);
    }
  }

  function clearCurrentArchive() {
    void perform(async () => {
      const currentLifetime = lifetime.current;
      await sessionArchiveClear();
      // Clearing current files must also retire their loaded or pending WAV.
      // A late completion cannot release another session's playback.
      if (lifetime.current === currentLifetime && selectedHistoryId.current === null
        && latestArchiveContext.current === archiveContext) {
        audioRequest.current += 1;
        setAudioUrl(null);
        setAudioError(false);
      }
      return "cleared";
    });
  }

  function copyPassage(source: string, translation: string) {
    const value = (reading === "source" ? [source] : reading === "translation" ? [translation] : [source, translation]).filter(Boolean).join("\n");
    const currentLifetime = lifetime.current;
    const currentInteraction = interaction.current;
    const notify = beginToast();
    void writeDiagnosticClipboard(Promise.resolve(value)).then(() => {
      if (lifetime.current === currentLifetime && interaction.current === currentInteraction) notify(copy.copied);
    }, () => {
      if (lifetime.current === currentLifetime && interaction.current === currentInteraction) setManualCopy(value);
    });
  }

  async function deleteHistory(id: string) {
    if (!isTauri || operation.current) return;
    const currentLifetime = lifetime.current;
    const currentInteraction = interaction.current;
    operation.current = true;
    setBusy(true);
    const notify = beginToast();
    try {
      await sessionHistoryDelete(id);
      if (lifetime.current !== currentLifetime) return;
      // Deletion changes the catalog even after navigation. Reject an older
      // list snapshot without disturbing another selected session's content.
      historyRequest.current += 1;
      setHistory((items) => items.filter((item) => item.id !== id));
      setHistoryRefresh(value => value + 1);
      if (selectedHistoryId.current === id) { selectHistory(null); setReaderOpen(false); }
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
  const showContent = readerOpen;
  // Reuse each format only within this render. Later polls still observe
  // language/default-time-zone changes, and empty lists need no formatter.
  const language = effectiveUiLanguage();
  const copy = sessionWorkspaceCopy[language];
  const contentOptions = [
    { value: "bilingual", label: copy.bilingual },
    { value: "source", label: copy.source },
    { value: "translation", label: copy.translation },
  ];
  const exportDisabled = !isTauri || busy || (!selected && (active || readError));
  const filteredHistory = history.filter(item =>
    (!historyQuery.trim() || (item.title ?? "").toLocaleLowerCase(language).includes(historyQuery.trim().toLocaleLowerCase(language)))
    && (historyFilter === "all" || (historyFilter === "text" ? item.count > 0 : item.hasAudio)));
  const sessionName = selected ? selected.title || I18N.settings.historySelectedTitle : I18N.settings.historyCurrent;
  const sourceOptions = audioSources.map(source => ({ value: source, label: source === "system" ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone }));
  function changeAudioSource(value: string) {
    audioRequest.current += 1;
    setRecordingSource(value as AudioSource);
    setAudioUrl(null);
    setAudioError(false);
  }
  const sourceControl = audioSources.length > 1
    ? <SettingsSelect label={I18N.settings.recordingSource} value={selectedAudioSource ?? "system"} disabled={busy} options={sourceOptions} onChange={changeAudioSource} />
    : <span className="session-workspace__audio-source">{sourceOptions[0]?.label}</span>;
  let sessionTimeFormatter: Intl.DateTimeFormat | undefined;
  let transcriptTimeFormatter: Intl.DateTimeFormat | undefined;
  const formatSessionTime = (timestamp: number) =>
    (sessionTimeFormatter ??= new Intl.DateTimeFormat(language, { hour: "2-digit", minute: "2-digit" })).format(timestamp);
  const formatTranscriptTime = (timestamp: number) =>
    (transcriptTimeFormatter ??= new Intl.DateTimeFormat(language, { hour: "2-digit", minute: "2-digit", second: "2-digit" })).format(timestamp);

  return (
    <SettingsSection
      id="session-export"
      title={I18N.settings.sessionExportTitle}
      hideHeading
    >
      <div ref={workspace}>
      {!readerOpen && <>
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
      </>}
      <div className={`session-export__details${readerOpen ? " session-export__details--reader" : ""}`}>
        {active && (!readerOpen || !selected) && (
          <InlineFeedback tone="info">{copy.active}</InlineFeedback>
        )}
        {active && recordAudio && !readerOpen && (
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
        {!readerOpen && <div className="session-history">
          <div className="session-history__heading">
            <h3>{I18N.settings.historyTitle}<SettingsHelp label={I18N.settings.helpLabel} text={I18N.settings.sessionExportEphemeral} /></h3>
            {history.length > 0 && <span>{I18N.settings.historySessionCount(history.length)}</span>}
          </div>
          {history.length > 0 && <div className="session-workspace__library-tools">
            <input type="search" value={historyQuery} aria-label={copy.search} placeholder={copy.search} maxLength={120} onChange={event => setHistoryQuery(event.target.value)} />
            <SettingsSelect label={copy.filter} value={historyFilter} onChange={setHistoryFilter} options={[{ value: "all", label: copy.all }, { value: "text", label: copy.text }, { value: "audio", label: copy.audio }]} />
          </div>}
          <div className="session-workspace__catalog">
          {showCurrent && <button type="button" data-history-id="current" aria-pressed={selectedId === null} className={`session-history__item${selectedId === null ? " is-selected" : ""}`} onClick={() => selectHistory(null)}>
            <span>{I18N.settings.historyCurrent}</span>
            {(hasTranscript || hasAudio) && <small>{hasTranscript ? I18N.settings.transcriptCount(archive?.transcriptCount ?? 0) : ""}{hasTranscript && hasAudio ? " · " : ""}{hasAudio ? I18N.settings.historyAudio : ""}</small>}
          </button>}
          {groupHistory(filteredHistory, language).map((group) => (
            <div className="session-history__group" key={group.day}>
              <h4>{group.label}</h4>
              {group.items.map((item) => (
                <button type="button" key={item.id} data-history-id={item.id} aria-pressed={selectedId === item.id} className={`session-history__item${selectedId === item.id ? " is-selected" : ""}`} onClick={() => selectHistory(item.id)}>
                  <span className="session-workspace__session-name">{item.title || formatSessionTime(item.startedAtMs)}{item.title && <time>{formatSessionTime(item.startedAtMs)}</time>}</span>
                  <small>{item.count > 0 ? I18N.settings.transcriptCount(item.count) : ""}{item.count > 0 && item.hasAudio ? " · " : ""}{item.hasAudio ? I18N.settings.historyAudio : ""}</small>
                </button>
              ))}
            </div>
          ))}
          </div>
          {history.length > 0 && filteredHistory.length === 0 && <p className="session-transcript__empty">{copy.noMatches}</p>}
          {historyError && <InlineFeedback tone="error">{I18N.settings.historyReadFailed} <button type="button" className="settings-link"
            disabled={!pageVisible || historyLoading} aria-busy={historyLoading || undefined}
            onClick={() => setHistoryRefresh(value => value + 1)}>{I18N.settings.retryUpdate}</button></InlineFeedback>}
          {!showCurrent && history.length === 0 && !historyError && <p className="session-export__empty">{I18N.settings.historyEmpty}</p>}
        </div>}
        {showContent ? <div className="session-transcript">
          <button type="button" className="session-workspace__back" onClick={returnToLibrary}><Icon name="chevron-left" />{copy.back}</button>
          <div className="session-transcript__heading">
            <div><h3 ref={readerHeading} tabIndex={-1}>{sessionName}</h3>
              <span className="session-workspace__reader-meta">{selected && <time>{new Intl.DateTimeFormat(language, { dateStyle: "medium", timeStyle: "short" }).format(selected.startedAtMs)}</time>}{availableCount > 0 && <span>{I18N.settings.transcriptCount(availableCount)}</span>}</span>
            </div>
            <div className="session-workspace__reader-actions">
              <button type="button" className="settings-button settings-button--quiet" disabled={exportDisabled || (!canExportTranscript && !canExportAudio)}
                onClick={() => { setExportAudio(!canExportTranscript); setExportOptions(options => ({ ...options, content: reading })); setExportOpen(true); }}><Icon name="download" />{copy.export}</button>
              <SettingsActionMenu label={I18N.settings.profileActionMore} name={sessionName} disabled={busy || !pageVisible}
                actions={selected ? [
                  { id: "rename", label: copy.rename, icon: "captions-bubble", onSelect: () => { setNameDraft(selected.title ?? ""); setRenameError(false); setRenameOpen(true); } },
                  { id: "delete", label: I18N.settings.historyDelete, icon: "trash", destructive: true, onSelect: () => setConfirmDelete(true) },
                ] : [{ id: "clear", label: I18N.settings.clearSessionArchive, icon: "eraser", destructive: true, disabled: disabled || readError || (!hasTranscript && !hasAudio),
                  onSelect: clearCurrentArchive }]} />
            </div>
          </div>
          {canExportAudio && pageVisible && <SessionAudioPlayer key={`${selectedId ?? "current"}:${selectedAudioSource}:${active}`}
            src={active ? null : audioUrl} loading={audioLoading} readError={audioError} disabled={active || !isTauri || (!selected && readError)} onLoad={loadAudio} sourceControl={sourceControl}
            labels={{ play: I18N.settings.historyPlayAudio, pause: copy.pause, loading: copy.loadingAudio, retry: copy.retryAudio, seek: copy.seek, playFailed: copy.playFailed, ready: copy.audioReady, label: I18N.settings.historyAudio }} />}
          {canExportAudio && active && <InlineFeedback tone="info">{copy.stopPlayback}</InlineFeedback>}
          {confirmDelete && selected && visible && <SettingsConfirmation message={I18N.settings.historyDeleteConfirm} confirmLabel={I18N.settings.historyDelete} disabled={busy}
            onCancel={() => setConfirmDelete(false)} onConfirm={() => { void deleteHistory(selected.id); }} />}
          {availableCount > 0 && <div className="session-transcript__toolbar">
            <input
              type="search"
              value={query}
              maxLength={120}
              placeholder={I18N.settings.transcriptSearch}
              aria-label={I18N.settings.transcriptSearch}
              onChange={(event) => { setQuery(event.target.value); setPage(0); setTranscript(undefined); setTranscriptError(false); }}
            />
            <SettingsSelect label={copy.reading} value={reading} options={contentOptions} onChange={value => setReading(value as TranscriptExportOptions["content"])} />
            {query && displayedTranscript && <span className="session-workspace__search-count">{I18N.settings.transcriptMatches(displayedTranscript.total)}</span>}
          </div>}
          <div className="session-transcript__list" aria-live="polite">
            {availableCount && transcriptError ? (
              <InlineFeedback tone="error">{I18N.settings.transcriptReadFailed} <button type="button" className="settings-link"
                disabled={!pageVisible || transcriptLoading} aria-busy={transcriptLoading || undefined}
                onClick={() => setTranscriptRefresh(value => value + 1)}>{I18N.settings.retryUpdate}</button></InlineFeedback>
            ) : displayedTranscript?.entries.length ? displayedTranscript.entries.map((entry) => (
              <article className="session-transcript__entry" key={entry.index}>
                <div className="session-transcript__meta">
                  <span>#{entry.index}</span>
                  <time dateTime={new Date(entry.createdAtMs).toISOString()}>
                    {formatTranscriptTime(entry.createdAtMs)}
                  </time>
                  {entry.audioSource && <span className="session-transcript__audio-source">{entry.audioSource === "system" ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone}</span>}
                  <button type="button" className="session-workspace__copy" aria-label={`${copy.copy} #${entry.index}`} title={copy.copy} disabled={!(reading === "source" ? entry.source : reading === "translation" ? entry.translation : entry.source || entry.translation)} onClick={() => copyPassage(entry.source, entry.translation)}><Icon name="copy" /></button>
                </div>
                <div>
                <div className={`session-transcript__pair${reading !== "bilingual" ? " session-transcript__pair--single" : ""}`}>
                  {reading !== "translation" && <div><span>{I18N.settings.transcriptSource}</span><p>{entry.source || "—"}</p></div>}
                  {reading !== "source" && <div><span>{I18N.settings.transcriptTranslation}</span><p>{entry.translation || "—"}</p></div>}
                </div>
                </div>
              </article>
            )) : (
              <p className="session-transcript__empty">
                {!availableCount ? (canExportAudio ? I18N.settings.historyAudioOnly : I18N.settings.transcriptEmpty) : query && !transcriptLoading ? I18N.settings.transcriptNoMatches : I18N.settings.transcriptLoading}
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
        </div> : null}
        {renameOpen && selected && visible && <SettingsConfirmation message={copy.rename} variant="default" confirmIcon="checkmark" confirmLabel={copy.save} disabled={busy || [...nameDraft.trim()].length > 120}
          onCancel={() => setRenameOpen(false)} onConfirm={() => { void renameHistory(selected.id); }}>
          <label className="session-workspace__name-field">{copy.name}<SettingsHelp label={I18N.settings.helpLabel} text={copy.nameHelp} />
            <input value={nameDraft} maxLength={240} disabled={busy} onChange={event => { setNameDraft(event.target.value); setRenameError(false); }} />
          </label>
          {[...nameDraft.trim()].length > 120 && <InlineFeedback tone="error">{copy.nameHelp}</InlineFeedback>}
          {renameError && <InlineFeedback tone="error">{copy.renameFailed}</InlineFeedback>}
        </SettingsConfirmation>}
        {exportOpen && visible && <SettingsConfirmation message={copy.export} variant="default" confirmIcon="download" confirmLabel={copy.chooseFile} disabled={exportDisabled || (exportAudio ? !canExportAudio : !canExportTranscript)}
          onCancel={() => setExportOpen(false)} onConfirm={() => {
            setExportOpen(false);
            void perform(async () => (await sessionExport(exportAudio ? "audio" : "transcript", selected?.id, exportAudio ? selectedAudioSource : undefined, exportAudio ? undefined : exportOptions)) ? "saved" : null, I18N.settings.sessionExportFailed, !selected);
          }}>
          <p className="session-workspace__export-scope">{copy.whole}</p>
          <div className="session-workspace__export-options">
          <SettingsRow label={copy.format}><SettingsSelect label={copy.format} value={exportAudio ? "wav" : exportOptions.format} onChange={value => { setExportAudio(value === "wav"); if (value !== "wav") setExportOptions(options => ({ ...options, format: value as TranscriptExportOptions["format"] })); }} options={[...(canExportTranscript ? [{ value: "txt", label: copy.plain }, { value: "markdown", label: copy.markdown }] : []), ...(canExportAudio ? [{ value: "wav", label: copy.wav }] : [])]} /></SettingsRow>
          {exportAudio ? (audioSources.length > 1 && <SettingsRow label={I18N.settings.recordingSource}>{sourceControl}</SettingsRow>) : <>
          <SettingsRow label={copy.content}><SettingsSelect label={copy.content} value={exportOptions.content} onChange={value => setExportOptions(options => ({ ...options, content: value as TranscriptExportOptions["content"] }))} options={contentOptions} /></SettingsRow>
          <SettingsRow label={copy.timestamps} description={copy.timing}><Switch checked={exportOptions.includeTimestamps} aria-label={copy.timestamps} onChange={includeTimestamps => setExportOptions(options => ({ ...options, includeTimestamps }))} /></SettingsRow></>}
          </div>
        </SettingsConfirmation>}
        {manualCopy !== null && visible && <SettingsConfirmation message={copy.copyFailed} variant="default" hideConfirm onCancel={() => setManualCopy(null)} onConfirm={() => {}}>
          <textarea className="session-workspace__manual-copy" readOnly value={manualCopy} aria-label={copy.copy} onFocus={event => event.currentTarget.select()} />
        </SettingsConfirmation>}
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
      {!readerOpen && <details className="session-export__explanation">
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
      </details>}
      </div>
    </SettingsSection>
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
