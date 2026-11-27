import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../../../components/Icon";
import { Switch } from "../../../components/inputs/Switch";
import { writeDiagnosticClipboard } from "../../../lib/clipboard/diagnosticClipboard";
import { effectiveUiLanguage, I18N } from "../../../lib/i18n";
import {
  isTauri,
  sessionArchiveClear,
  sessionArchiveState,
  sessionExport,
  sessionHistoryDelete,
  sessionHistoryDeleteMany,
  sessionHistoryList,
  sessionHistoryPage,
  sessionHistoryRename,
  sessionTranscriptPage,
} from "../../../lib/ipc";
import { useStore } from "../../../lib/store";
import type {
  AudioSource,
  SessionArchiveState,
  SessionHistoryDeleteFailureReason,
  SessionHistoryDeleteResult,
  SessionHistoryItem,
  SettingsDraft,
  TranscriptExportOptions,
  TranscriptPage,
} from "../../../lib/types";
import { useDocumentVisibility } from "../../../lib/useDocumentVisibility";
import {
  InlineFeedback,
  SettingsRow,
  SettingsSection,
  SettingsSelect,
} from "../controls/SettingsPrimitives";
import { useSettingsToast } from "../feedback/useSettingsToast";
import { monitorSessionArchive, sameSessionArchiveState } from "./sessionArchiveMonitor";
import { SessionExportDialogs, type PendingBatchDelete } from "./SessionExportDialogs";
import { SessionHistoryLibrary } from "./SessionHistoryLibrary";
import { SessionTranscriptReader } from "./SessionTranscriptReader";
import { sessionWorkspaceCopy } from "./sessionWorkspaceCopy";
import { useSessionAudioPreview } from "./useSessionAudioPreview";
import "./session-workspace.css";

type DeleteFailureReasonKey = keyof typeof sessionWorkspaceCopy.en.deleteFailureReasons;

const DELETE_FAILURE_REASON_MAP: Record<SessionHistoryDeleteFailureReason, DeleteFailureReasonKey> = {
  active_session: "activeSession",
  history_disabled: "historyDisabled",
  not_found: "notFound",
  permission_denied: "permissionDenied",
  invalid_id: "invalidId",
  io_error: "ioError",
};

const DELETE_ERROR_CODE_MAP: Record<string, DeleteFailureReasonKey> = {
  history_delete_active_session: "activeSession",
  history_delete_disabled: "historyDisabled",
  history_delete_not_found: "notFound",
  history_delete_permission_denied: "permissionDenied",
  history_delete_invalid_id: "invalidId",
  history_delete_io_error: "ioError",
};

function resolveDeleteFailureReason(error: unknown, reportReason?: SessionHistoryDeleteFailureReason): DeleteFailureReasonKey | null {
  if (reportReason && Object.hasOwn(DELETE_FAILURE_REASON_MAP, reportReason)) {
    return DELETE_FAILURE_REASON_MAP[reportReason];
  }
  const message = typeof error === "string" ? error : error instanceof Error ? error.message : "";
  if (!message) return null;
  if (Object.hasOwn(DELETE_ERROR_CODE_MAP, message)) return DELETE_ERROR_CODE_MAP[message];
  // Match Tauri v2 ACL rejections without interpolating raw runtime strings.
  if (/\bnot allowed\b/i.test(message) || /\bpermissions? associated with this command\b/i.test(message)) {
    return "commandDenied";
  }
  return null;
}

function formatHistoryDeleteFailure(error: unknown, report?: SessionHistoryDeleteResult): string {
  const strings = sessionWorkspaceCopy[effectiveUiLanguage()];
  const reasonKey = resolveDeleteFailureReason(error, report?.failureReason);
  const reason = reasonKey ? strings.deleteFailureReasons[reasonKey] : null;
  if (report && report.deleted.length > 0 && report.failed.length > 0) {
    return reason
      ? strings.deletePartialFailedWithReason(report.deleted.length, report.failed.length, reason)
      : strings.deletePartialFailed(report.deleted.length, report.failed.length);
  }
  return reason ? strings.deleteFailedWithReason(reason) : I18N.settings.historyDeleteFailed;
}

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
  const retainHistory = useStore((state) => state.settings.retainSessionHistory);
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
  const [selectMode, setSelectMode] = useState(false);
  const [selectedHistoryIds, setSelectedHistoryIds] = useState<string[]>([]);
  const [pendingBatchDelete, setPendingBatchDelete] = useState<PendingBatchDelete>(null);
  const [recordingSource, setRecordingSource] = useState<AudioSource>("system");
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
  const interaction = useRef(0);
  const selectedHistoryId = useRef<string | null>(null);
  const selected = history.find((item) => item.id === selectedId);
  const audioSources = selected ? selected.audioSources ?? ["system" as const] : archive?.audioSources ?? ["system" as const];
  const selectedAudioSource = audioSources.includes(recordingSource) ? recordingSource : audioSources[0];
  const audioPreview = useSessionAudioPreview({
    active,
    pageVisible,
    readerOpen,
    selectedId,
    audioSource: selectedAudioSource,
    recordAudio,
  });
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
    audioPreview.clear();
    setReaderOpen(false);
    clearToast();
  }

  function clearSelectedSessionState() {
    selectedHistoryId.current = null;
    transcriptRequest.current += 1;
    audioPreview.clear();
    setTranscriptError(false);
    setConfirmDelete(false);
    setRenameOpen(false);
    setExportOpen(false);
    setManualCopy(null);
    setSelectedId(null);
    setQuery("");
    setPage(0);
    setTranscript(undefined);
    setReaderOpen(false);
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
    audioPreview.clear();
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

  useEffect(() => {
    lifetime.current += 1;
    return () => {
      lifetime.current += 1;
      transcriptRequest.current += 1;
      historyRequest.current += 1;
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
      if (result && lifetime.current === currentLifetime && interaction.current === currentInteraction) {
        notify(result === "saved" ? I18N.settings.sessionExportSaved : I18N.settings.sessionArchiveCleared);
      }
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
        audioPreview.clear();
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
      if (selectedHistoryId.current === id) clearSelectedSessionState();
      if (interaction.current === currentInteraction) {
        notify(sessionWorkspaceCopy[effectiveUiLanguage()].deletedSessions(1));
      }
    } catch (error) {
      if (lifetime.current === currentLifetime && interaction.current === currentInteraction) {
        notify(formatHistoryDeleteFailure(error), true);
      }
    } finally {
      operation.current = false;
      if (lifetime.current === currentLifetime) setBusy(false);
    }
  }

  async function deleteHistoryMany(mode: "selected" | "all", ids: string[]) {
    if (!isTauri || operation.current || ids.length === 0) return;
    const currentLifetime = lifetime.current;
    const currentInteraction = interaction.current;
    operation.current = true;
    setBusy(true);
    const notify = beginToast();
    try {
      const result = await sessionHistoryDeleteMany(ids);
      if (lifetime.current !== currentLifetime) return;
      const deletedSet = new Set(result.deleted);
      if (deletedSet.size > 0) {
        historyRequest.current += 1;
        setHistory((items) => items.filter((item) => !deletedSet.has(item.id)));
        setHistoryRefresh(value => value + 1);
        setSelectedHistoryIds((items) => items.filter((id) => !deletedSet.has(id)));
        if (selectedHistoryId.current && deletedSet.has(selectedHistoryId.current)) {
          clearSelectedSessionState();
        }
      }
      if (result.failed.length === 0) {
        setPendingBatchDelete(null);
        setSelectMode(false);
        setSelectedHistoryIds([]);
        if (interaction.current === currentInteraction && result.deleted.length > 0) {
          notify(sessionWorkspaceCopy[effectiveUiLanguage()].deletedSessions(result.deleted.length));
        }
      } else if (result.deleted.length > 0) {
        setPendingBatchDelete({ mode, ids: result.failed });
        if (selectMode) setSelectedHistoryIds(result.failed);
        if (interaction.current === currentInteraction) {
          notify(formatHistoryDeleteFailure(null, result), true);
        }
      } else if (interaction.current === currentInteraction) {
        notify(formatHistoryDeleteFailure(null, result), true);
      }
    } catch (error) {
      if (lifetime.current === currentLifetime && interaction.current === currentInteraction) {
        notify(formatHistoryDeleteFailure(error), true);
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
  const language = effectiveUiLanguage();
  const copy = sessionWorkspaceCopy[language];
  const contentOptions = [
    { value: "bilingual", label: copy.bilingual },
    { value: "source", label: copy.source },
    { value: "translation", label: copy.translation },
  ];
  const exportDisabled = !isTauri || busy || (!selected && (active || readError));
  const sessionName = selected ? selected.title || I18N.settings.historySelectedTitle : I18N.settings.historyCurrent;
  const sourceOptions = audioSources.map(source => ({
    value: source,
    label: source === "system" ? I18N.settings.audioInputSystem : I18N.settings.audioInputMicrophone,
  }));
  const sourceControl = audioSources.length > 1 ? (
    <SettingsSelect
      label={I18N.settings.recordingSource}
      value={selectedAudioSource ?? "system"}
      disabled={busy}
      options={sourceOptions}
      onChange={(value) => {
        audioPreview.clear();
        setRecordingSource(value as AudioSource);
      }}
    />
  ) : (
    <span className="session-workspace__audio-source">{sourceOptions[0]?.label}</span>
  );

  return (
    <SettingsSection id="session-export" title={I18N.settings.sessionExportTitle} hideHeading>
      <div ref={workspace}>
        {!readerOpen && (
          <>
            <SettingsRow
              label={I18N.settings.retainSessionHistory}
              description={I18N.settings.retainSessionHistoryHelp}
              align="start"
            >
              <Switch
                checked={retainHistory}
                disabled={disabled}
                aria-label={I18N.settings.retainSessionHistory}
                onChange={(retainSessionHistory) => changeSetting({ retainSessionHistory })}
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
                onChange={(recordSessionAudio) => changeSetting({ recordSessionAudio })}
              />
            </SettingsRow>
          </>
        )}
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
            <InlineFeedback tone="info">{I18N.settings.sessionTranscriptLimit}</InlineFeedback>
          )}
          {archive?.audioLimited && (
            <InlineFeedback tone="info">{I18N.settings.sessionAudioLimit}</InlineFeedback>
          )}
          {archive?.historySaveError && (
            <InlineFeedback tone="error">{I18N.settings.historySaveFailed}</InlineFeedback>
          )}
          {!readerOpen ? (
            <SessionHistoryLibrary
              history={history}
              archive={archive}
              selectedId={selectedId}
              showCurrent={showCurrent}
              hasTranscript={hasTranscript}
              hasAudio={hasAudio}
              busy={busy}
              pageVisible={pageVisible}
              historyError={historyError}
              historyLoading={historyLoading}
              historyQuery={historyQuery}
              historyFilter={historyFilter}
              selectMode={selectMode}
              selectedHistoryIds={selectedHistoryIds}
              language={language}
              copy={copy}
              onHistoryQueryChange={setHistoryQuery}
              onHistoryFilterChange={setHistoryFilter}
              onSelectHistory={selectHistory}
              onRetryHistory={() => setHistoryRefresh(value => value + 1)}
              onSelectModeChange={setSelectMode}
              onSelectedHistoryIdsChange={setSelectedHistoryIds}
              onOpenBatchDelete={(mode, ids) => setPendingBatchDelete({ mode, ids })}
              onClearBatchDelete={() => setPendingBatchDelete(null)}
            />
          ) : (
            <SessionTranscriptReader
              readerHeading={readerHeading}
              sessionName={sessionName}
              selected={selected}
              selectedId={selectedId}
              selectedAudioSource={selectedAudioSource}
              active={active}
              busy={busy}
              disabled={disabled}
              readError={readError}
              pageVisible={pageVisible}
              hasTranscript={hasTranscript}
              hasAudio={hasAudio}
              canExportTranscript={canExportTranscript}
              canExportAudio={canExportAudio}
              exportDisabled={exportDisabled}
              availableCount={availableCount}
              displayedTranscript={displayedTranscript}
              transcriptError={transcriptError}
              transcriptLoading={transcriptLoading}
              query={query}
              reading={reading}
              contentOptions={contentOptions}
              audioPreview={audioPreview}
              sourceControl={sourceControl}
              language={language}
              copy={copy}
              onBack={returnToLibrary}
              onOpenExport={() => {
                setExportAudio(!canExportTranscript);
                setExportOptions(options => ({ ...options, content: reading }));
                setExportOpen(true);
              }}
              onOpenRename={() => {
                if (!selected) return;
                setNameDraft(selected.title ?? "");
                setRenameError(false);
                setRenameOpen(true);
              }}
              onRequestDelete={() => setConfirmDelete(true)}
              onClearCurrentArchive={clearCurrentArchive}
              onQueryChange={(nextQuery) => {
                setQuery(nextQuery);
                setPage(0);
                setTranscript(undefined);
                setTranscriptError(false);
              }}
              onReadingChange={setReading}
              onRetryTranscript={() => setTranscriptRefresh(value => value + 1)}
              onCopyPassage={copyPassage}
              onChangePage={(nextPage) => {
                setPage(nextPage);
                setTranscript(undefined);
              }}
            />
          )}
          <SessionExportDialogs
            visible={visible}
            readerOpen={readerOpen}
            busy={busy}
            selected={selected}
            copy={copy}
            confirmDelete={confirmDelete}
            onCancelDelete={() => setConfirmDelete(false)}
            onConfirmDelete={(id) => { void deleteHistory(id); }}
            pendingBatchDelete={pendingBatchDelete}
            onCancelBatchDelete={() => setPendingBatchDelete(null)}
            onConfirmBatchDelete={(mode, ids) => { void deleteHistoryMany(mode, ids); }}
            renameOpen={renameOpen}
            nameDraft={nameDraft}
            renameError={renameError}
            onNameDraftChange={(value) => {
              setNameDraft(value);
              setRenameError(false);
            }}
            onCancelRename={() => setRenameOpen(false)}
            onConfirmRename={(id) => { void renameHistory(id); }}
            exportOpen={exportOpen}
            exportDisabled={exportDisabled}
            exportAudio={exportAudio}
            canExportAudio={canExportAudio}
            canExportTranscript={canExportTranscript}
            audioSourceCount={audioSources.length}
            sourceControl={sourceControl}
            exportOptions={exportOptions}
            contentOptions={contentOptions}
            onExportAudioChange={setExportAudio}
            onExportOptionsChange={setExportOptions}
            onCancelExport={() => setExportOpen(false)}
            onConfirmExport={() => {
              setExportOpen(false);
              void perform(
                async () =>
                  (await sessionExport(
                    exportAudio ? "audio" : "transcript",
                    selected?.id,
                    exportAudio ? selectedAudioSource : undefined,
                    exportAudio ? undefined : exportOptions,
                  ))
                    ? "saved"
                    : null,
                I18N.settings.sessionExportFailed,
                !selected,
              );
            }}
            manualCopy={manualCopy}
            onCloseManualCopy={() => setManualCopy(null)}
          />
          {!isTauri && <p className="settings-help">{I18N.settings.sessionExportNativeOnly}</p>}
          {readError && <InlineFeedback tone="error">{I18N.settings.sessionArchiveReadFailed}</InlineFeedback>}
        </div>
        {!readerOpen && (
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
        )}
      </div>
    </SettingsSection>
  );
}
