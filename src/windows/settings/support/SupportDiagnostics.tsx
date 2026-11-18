import { openAIRealtimeErrorMessage } from "../../../lib/locales/openaiRealtimeErrors";
import { SUPPLEMENTAL_EN } from "../../../lib/locales/supplemental-schema";
import { supplemental } from "../../../lib/locales/supplemental";
import { SettingsHelp } from "../SettingsHelp";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { AlertCircle, ChevronRight, Copy, ExternalLink, RotateCw } from "lucide-react";
import { useSettingsToast } from "../useSettingsToast";
import { invoke } from "@tauri-apps/api/core";
import { effectiveUiLanguage, I18N, subscribeUiLanguage, type ResolvedUiLanguage } from "../../../lib/i18n";
import { isTauri } from "../../../lib/ipc";
import { useDocumentVisibility } from "../../../lib/useDocumentVisibility";
import { writeDiagnosticClipboard } from "../../../lib/diagnosticClipboard";
import { diagnosticAudioState, diagnosticNextStep, supportDiagnosticSummary, type DiagnosticFailureCategory, type DiagnosticStatus, type DiagnosticSummaryEvent } from "./supportDiagnosticSummary";
import { diagnosticsCopy, type DiagnosticsCopy } from "./diagnosticsCopy";
import "./support-diagnostics.css";

const copy = {
  en: SUPPLEMENTAL_EN.SupportDiagnostics_copy,
  zh: {
    title: "诊断", copy: "复制诊断信息", issue: "GitHub 反馈",
    refresh: "刷新诊断", refreshing: "正在准备诊断信息…", refreshed: "诊断信息已刷新。",
    status: "字幕状态", lastError: "最近错误", recent: "最近事件", sinceStart: "距应用启动", noEvents: "还没有事件",
    failures: { recognition: "语音识别失败", unknown: "未分类错误", configuration: "需要调整配置", authentication: "身份验证失败", credential_storage: "凭据存储不可用", device_unavailable: "声音输出不可用", timeout: "服务超时", request_rejected: "请求被拒绝", service_error: "服务不可用", rate_limit: "服务限流", backlog: "处理速度落后", transport: "连接中断", stopped: "音频捕获停止", processing: "音频处理失败", size_limit: "字幕响应过长" },
    lifecycle: { start_requested: "请求启动", start_busy: "启动处理中", start_already_active: "已在运行", start_superseded: "启动已取消", stop_requested: "请求停止", pause_requested: "请求暂停", resume_requested: "请求继续" },
    recovery: { retrying: "正在重连", recovered: "连接已恢复", retries_exhausted: "自动重试次数已用尽", user_stopped: "用户已停止" },
    copied: "复制成功", failed: "复制失败，请重试或手动复制下方文字。",
    prepareFailed: "暂时无法准备诊断信息，请重试。",
    opened: "已打开 GitHub。请检查内容、描述问题后再提交。",
    paste: "已打开 GitHub。请先复制诊断信息，再粘贴到反馈正文后提交。",
    openFailed: "暂时无法打开 GitHub，请复制诊断信息并前往 Mimi 仓库。", preview: "原始诊断数据",
  },
  ja: {
    title: "診断", copy: "診断情報をコピー", issue: "GitHub で報告",
    refresh: "診断を更新", refreshing: "診断情報を準備中…", refreshed: "診断情報を更新しました。",
    status: "字幕の状態", lastError: "直近のエラー", recent: "最近のイベント", sinceStart: "アプリ起動から", noEvents: "イベントはまだありません",
    failures: { recognition: "音声認識に失敗", unknown: "未分類のエラー", configuration: "設定の変更が必要", authentication: "認証に失敗", credential_storage: "認証情報ストアを利用不可", device_unavailable: "音声出力を利用不可", timeout: "サービスがタイムアウト", request_rejected: "リクエストが拒否されました", service_error: "サービスを利用不可", rate_limit: "サービスの利用制限", backlog: "処理が遅れています", transport: "接続が中断", stopped: "音声の取得が停止", processing: "音声処理に失敗", size_limit: "字幕の応答が長すぎます" },
    lifecycle: { start_requested: "開始を要求", start_busy: "開始処理中", start_already_active: "すでに実行中", start_superseded: "開始をキャンセル", stop_requested: "停止を要求", pause_requested: "一時停止を要求", resume_requested: "再開を要求" },
    recovery: { retrying: "再接続中", recovered: "接続が復旧", retries_exhausted: "自動再試行の上限に到達", user_stopped: "ユーザーが停止" },
    copied: "コピー成功", failed: "コピーできませんでした。再試行するか、下のテキストを手動でコピーしてください。",
    prepareFailed: "診断情報を準備できませんでした。再試行してください。",
    opened: "GitHub を開きました。内容を確認し、問題を説明してから送信してください。",
    paste: "GitHub を開きました。診断情報をコピーして本文に貼り付けてから送信してください。",
    openFailed: "GitHub を開けませんでした。診断情報をコピーし、Mimi リポジトリにアクセスしてください。", preview: "診断の生データ",
  },
  ...supplemental.SupportDiagnostics_copy,
};
const isFailure = (value: Feedback) => value === "failed" || value === "prepareFailed" || value === "openFailed";
type Feedback = "copied" | "refreshed" | "failed" | "prepareFailed" | "opened" | "paste" | "openFailed" | null;
interface SupportIssue { report: string; requiresPaste: boolean }
type DiagnosticAction = "copy" | "issue" | "refresh";

function diagnosticReport(value: unknown): string {
  if (typeof value !== "string" || new TextEncoder().encode(value).length > 16_384) throw new Error("invalid_diagnostics");
  return value;
}

function statusLabel(status: DiagnosticStatus): string {
  return {
    idle: I18N.settings.sessionIdle, connecting: I18N.settings.sessionConnecting,
    stopping: I18N.settings.sessionStopping, listening: I18N.settings.sessionListening,
    paused: I18N.settings.sessionPaused, error: I18N.settings.sessionError,
  }[status];
}

function failureLabel(category: DiagnosticFailureCategory, text: typeof copy.en | typeof copy.zh | typeof copy.ja, added: DiagnosticsCopy, code?: string): string {
  if (code) {
    const message = openAIRealtimeErrorMessage(code);
    if (message) return message;
  }
  if (category === "permission") return added.permission;
  if (category === "user_stopped") return added.userStopped;
  if (category === "start") return added.startFailed;
  if (category === "device_unavailable") return added.deviceUnavailable;
  return text.failures[category];
}

function eventLabel(event: DiagnosticSummaryEvent, text: typeof copy.en | typeof copy.zh | typeof copy.ja, added: DiagnosticsCopy): string {
  switch (event.kind) {
    case "status": return statusLabel(event.status);
    case "lifecycle": return text.lifecycle[event.action];
    case "failure": return failureLabel(event.category, text, added, event.code);
    case "recovery": return text.recovery[event.action];
    case "translation_backoff": return event.reason === "rateLimited"
      ? event.retryScheduled ? I18N.overlay.translationRateLimited : I18N.overlay.translationLimited
      : event.retryScheduled ? I18N.overlay.translationRetrying : I18N.overlay.translationUnavailable;
  }
}

function relativeTime(milliseconds: number, language: ResolvedUiLanguage): string {
  const seconds = Math.floor(milliseconds / 1000);
  const unit = seconds >= 3600 ? "hour" : seconds >= 60 ? "minute" : "second";
  const value = unit === "hour" ? Math.floor(seconds / 3600) : unit === "minute" ? Math.floor(seconds / 60) : seconds;
  return new Intl.RelativeTimeFormat(language, { numeric: "auto" }).format(-value, unit);
}

export interface SupportDiagnosticsProps {
  visible?: boolean;
  onOpenAudio?: () => void;
  onOpenServices?: () => void;
}

export function SupportDiagnostics({ visible = false, onOpenAudio, onOpenServices }: SupportDiagnosticsProps) {
  const language = useSyncExternalStore(subscribeUiLanguage, effectiveUiLanguage);
  const documentVisible = useDocumentVisibility();
  const pageVisible = visible && documentVisible;
  const [report, setReport] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState<DiagnosticAction | null>(null);
  const { beginToast, clearToast } = useSettingsToast();
  const [manualCopy, setManualCopy] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [eventsExpanded, setEventsExpanded] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<Date | null>(null);
  const [readFailed, setReadFailed] = useState(false);
  const operation = useRef(false);
  const request = useRef(0);
  const lifetime = useRef(0);
  const viewGeneration = useRef(0);
  const queuedEntryRefresh = useRef<(() => void) | null>(null);
  useEffect(() => {
    const clear = () => { lifetime.current += 1; clearToast(); };
    const hidden = () => { if (document.hidden) clear(); };
    window.addEventListener("hashchange", clear);
    window.addEventListener("blur", clear);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      lifetime.current += 1;
      request.current += 1;
      operation.current = false;
      clearToast();
      window.removeEventListener("hashchange", clear);
      window.removeEventListener("blur", clear);
      document.removeEventListener("visibilitychange", hidden);
    };
  }, [clearToast]);
  const perform = useCallback(async (action: DiagnosticAction, showSuccess = true) => {
    if (operation.current) return;
    operation.current = true;
    const currentRequest = ++request.current;
    const currentView = viewGeneration.current;
    setPendingAction(action);
    setManualCopy(false);
    const showToast = beginToast();
    const currentLifetime = lifetime.current;
    const notify = (value: Feedback) => {
      if (value && currentRequest === request.current && currentLifetime === lifetime.current) showToast(copy[effectiveUiLanguage()][value], isFailure(value));
    };
    const applyReport = (value: string) => {
      if (currentRequest !== request.current || currentView !== viewGeneration.current) return;
      setReport(value);
      setUpdatedAt(new Date());
      setReadFailed(false);
    };
    const failedRead = () => {
      if (currentRequest === request.current && currentView === viewGeneration.current) setReadFailed(true);
    };
    try {
      if (action === "issue") {
        // Backend builds the fixed public destination from typed whitelist facts.
        const issue = await invoke<SupportIssue>("app_open_support_issue");
        const value = diagnosticReport(issue.report);
        applyReport(value);
        notify(issue.requiresPaste ? "paste" : "opened");
      } else {
        const value = invoke<string>("support_diagnostics").then((result) => {
          const report = diagnosticReport(result);
          applyReport(report);
          return report;
        });
        if (action === "refresh") {
          await value;
          if (showSuccess) notify("refreshed");
        } else {
          // Preserve the click gesture for WebKit while preparing a fresh report.
          try { await writeDiagnosticClipboard(value); notify("copied"); }
          catch {
            try {
              await value;
              if (currentRequest === request.current && currentLifetime === lifetime.current) setManualCopy(true);
              notify("failed");
            } catch { failedRead(); notify("prepareFailed"); }
          }
        }
      }
    } catch { if (action !== "issue") failedRead(); notify(action === "issue" ? "openFailed" : "prepareFailed"); }
    finally {
      if (currentRequest === request.current) {
        operation.current = false;
        setPendingAction(null);
        const refresh = queuedEntryRefresh.current;
        queuedEntryRefresh.current = null;
        refresh?.();
      }
    }
  }, [beginToast]);
  useEffect(() => {
    viewGeneration.current += 1;
    if (!pageVisible) {
      lifetime.current += 1;
      clearToast();
      return;
    }
    if (!isTauri) return;
    let disposed = false;
    const refresh = () => {
      if (disposed) return;
      // A fresh visit needs a fresh snapshot. Keep native reads serial and
      // coalesce repeated visits while the previous operation is completing.
      if (operation.current) queuedEntryRefresh.current = refresh;
      else void perform("refresh", false);
    };
    queueMicrotask(refresh);
    return () => {
      disposed = true;
      viewGeneration.current += 1;
      if (queuedEntryRefresh.current === refresh) queuedEntryRefresh.current = null;
    };
  }, [pageVisible, perform, clearToast]);
  const text = copy[language];
  const added = diagnosticsCopy[language];
  const summary = useMemo(() => supportDiagnosticSummary(report), [report]);
  const busy = pendingAction !== null;
  const nextStep = summary && diagnosticNextStep(summary);
  const nextAction = nextStep === "audio" ? onOpenAudio : nextStep === "services" ? onOpenServices : undefined;
  const nextLabel = nextStep === "audio" ? added.openAudio : added.openServices;
  const nextHelp = nextStep === "audio" ? added.audioAction : added.servicesAction;
  const audioState = summary ? diagnosticAudioState(summary) : "unknown";
  const audioLabel = { unavailable: added.unavailable, past: added.pastAudio, unknown: added.notObserved, sound: added.sound, quiet: added.quiet, noAudio: added.noAudio }[audioState];
  const live = summary?.status === "listening";
  const translationState = !live ? added.notObserved : summary?.translationRecovery
    ? eventLabel({ kind: "translation_backoff", sequence: 0, elapsedMs: 0, ...summary.translationRecovery }, text, added)
    : summary?.translationTimedOut ? added.timedOut
      : summary?.translationPending ? added.waiting
        : summary?.translationPreviewPending ? added.previewWaiting
          : summary?.translationPending === false && summary.translationPreviewPending === false ? added.noPending : added.notObserved;
  const events = summary ? eventsExpanded ? summary.allEvents : summary.recentEvents : [];
  if (!isTauri) return null;
  return <section className="settings-support-diagnostics settings-diagnostics-page support-diagnostics" aria-label={text.title} aria-busy={busy}>
    {summary && <header className="diagnostics-overview">
      <div className="diagnostics-overview__heading"><span>{added.snapshot}</span><SettingsHelp text={added.snapshotHelp} label={I18N.settings.helpLabel} /></div>
      <div className="diagnostics-overview__body">
        <dl className="settings-diagnostic-summary"><div><dt>{text.status}</dt><dd className={`settings-diagnostic-status${summary.status === "error" ? " is-error" : ""}`}><span aria-hidden="true" />{statusLabel(summary.status)}</dd></div></dl>
        {updatedAt && <span className="diagnostics-updated">{added.updated} <time dateTime={updatedAt.toISOString()}>{updatedAt.toLocaleTimeString(language)}</time></span>}
      </div>
      {readFailed && <p className="diagnostics-read-state" role="alert">{added.staleRead}</p>}
      {summary.lastError && <div className={`diagnostics-last-error${summary.status === "error" && !summary.recoveredAfterError && summary.lastError !== "user_stopped" ? " is-current" : ""}`}>
        <AlertCircle size={16} aria-hidden="true" /><div><span>{added.recentError}{summary.lastErrorAgeMs !== null && <> · {relativeTime(summary.lastErrorAgeMs, language)}</>}</span><strong>{failureLabel(summary.lastError, text, added, summary.lastErrorCode)}</strong>{summary.recoveredAfterError && <span>{added.recoveredAfter}</span>}</div>
      </div>}
    </header>}
    {!summary && (busy || readFailed || report !== null) && <div className="diagnostics-read-state" role={readFailed && !busy ? "alert" : undefined}>
      {busy ? text.refreshing : readFailed ? text.prepareFailed : added.unsupported}
    </div>}
    {summary && nextAction && !readFailed && <div className="diagnostics-next-step">
      <div><span>{added.nextStep}</span><p>{nextHelp}</p></div>
      <button type="button" className="settings-button settings-button--quiet settings-button--compact" data-action={`open-${nextStep}`} disabled={busy} onClick={nextAction}>{nextLabel}<ChevronRight size={14} aria-hidden="true" /></button>
    </div>}
    <div className="settings-diagnostic-toolbar">
      <div className="settings-support-diagnostics__actions">
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" data-action="refresh" disabled={busy} aria-busy={pendingAction === "refresh" || undefined}
          onClick={() => void perform("refresh")}><RotateCw size={14} aria-hidden="true" /><span>{pendingAction === "refresh" ? text.refreshing : text.refresh}</span></button>
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" data-action="issue" disabled={busy}
          onClick={() => void perform("issue")}><ExternalLink size={14} aria-hidden="true" /><span>{text.issue}</span></button>
        <button type="button" className="settings-button settings-button--quiet settings-button--compact" data-action="copy" disabled={busy}
          onClick={() => void perform("copy")}><Copy size={14} aria-hidden="true" /><span>{text.copy}</span></button>
      </div>
    </div>
    {summary && <section className="diagnostics-facts" aria-label={added.facts}>
      <h2>{added.facts}</h2>
      <dl>
        <div className="diagnostics-fact" data-fact="audio"><dt>{added.audio}<SettingsHelp text={added.audioHelp} label={I18N.settings.helpLabel} /></dt><dd>{audioLabel}{audioState === "past" && summary.capture?.ageMs !== null && summary.capture?.ageMs !== undefined && <span className="diagnostics-secondary">{relativeTime(summary.capture.ageMs, language)}</span>}</dd></div>
        <div className="diagnostics-fact" data-fact="translation"><dt>{added.translation}</dt><dd>{translationState}</dd></div>
        <div className="diagnostics-fact" data-fact="records"><dt>{added.records}<SettingsHelp text={added.recordsHelp} label={I18N.settings.helpLabel} /></dt><dd className="diagnostics-counts">
          {([[added.sourceEvents, summary.textEvents.source], [added.translationEvents, summary.textEvents.translation], [added.confirmedEvents, summary.textEvents.confirmed]] as const).map(([label, value]) => <span key={label}><span>{label}</span><strong>{value === null ? "—" : value.toLocaleString(language)}</strong></span>)}
        </dd></div>
        <div className="diagnostics-fact" data-fact="measurements"><dt>{added.measurements}<SettingsHelp text={added.measurementsHelp} label={I18N.settings.helpLabel} /></dt><dd className="diagnostics-measurements">
          {live && summary.apiLatencyMs !== null && <span><span>{I18N.overlay.apiLatency}<SettingsHelp text={I18N.overlay.apiLatencyHelp} label={I18N.settings.helpLabel} /></span><strong>{summary.apiLatencyMs.toLocaleString(language)} ms</strong></span>}
          {live && summary.translationLatencyMs !== null && <span><span>{summary.translationLatencyKind === "follow" ? I18N.overlay.translationFollowLatency : I18N.overlay.translationLatency}<SettingsHelp text={summary.translationLatencyKind === "follow" ? I18N.overlay.translationFollowLatencyHelp : I18N.overlay.translationLatencyHelp} label={I18N.settings.helpLabel} /></span><strong>{summary.translationLatencyMs.toLocaleString(language)} ms</strong></span>}
          {(!live || summary.apiLatencyMs === null && summary.translationLatencyMs === null) && added.notObserved}
        </dd></div>
      </dl>
    </section>}
    {summary && <section className="settings-diagnostic-events" aria-label={text.recent}>
      <div className="settings-diagnostic-events__heading"><h2>{text.recent}</h2><SettingsHelp text={added.eventsHelp} label={I18N.settings.helpLabel} /></div>
      {events.length > 0 ? <ol id="diagnostic-recent-events">{events.map((event, index) => <li key={`${event.sequence}-${index}`} className={event.kind === "failure" && event.category !== "user_stopped" ? "is-error" : undefined}><span className="settings-diagnostic-events__time" title={`${text.sinceStart}: ${(event.elapsedMs / 1000).toFixed(1)} s`}>{summary.elapsedMs !== null && summary.elapsedMs >= event.elapsedMs ? relativeTime(summary.elapsedMs - event.elapsedMs, language) : `${(event.elapsedMs / 1000).toFixed(1)} s`}</span><span className="settings-diagnostic-events__rail" aria-hidden="true" /><span>{eventLabel(event, text, added)}</span></li>)}</ol> : <p className="settings-diagnostic-events__empty">{text.noEvents}</p>}
      {(summary.allEvents.length > 6 || (summary.omittedEvents ?? 0) > 0) && <div className="diagnostics-event-footer">
        {(summary.omittedEvents ?? 0) > 0 && <span>{added.omitted}: {summary.omittedEvents!.toLocaleString(language)}</span>}
        {summary.allEvents.length > 6 && <button type="button" className="settings-link" aria-expanded={eventsExpanded} aria-controls="diagnostic-recent-events" onClick={() => setEventsExpanded(value => !value)}>{eventsExpanded ? added.showLess : added.showAll}</button>}
      </div>}
    </section>}
    {report !== null && <details className="settings-diagnostic-preview" open={manualCopy || detailsOpen} onToggle={(event) => { const open = event.currentTarget.open; setDetailsOpen(open); if (!open) setManualCopy(false); }}>
      <summary><ChevronRight size={16} aria-hidden="true" /><span>{text.preview}</span></summary>
      {(manualCopy || detailsOpen) && (manualCopy ? <textarea aria-label={text.preview} value={report} readOnly rows={10} wrap="off" spellCheck={false} /> : <pre tabIndex={0} aria-label={text.preview}>{report}</pre>)}
    </details>}
  </section>;
}
