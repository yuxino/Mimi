import { SettingsHelp } from "./SettingsHelp";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle, ChevronRight, Copy, ExternalLink, RotateCw } from "lucide-react";
import { useSettingsToast } from "./useSettingsToast";
import { invoke } from "@tauri-apps/api/core";
import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import { isTauri } from "../../lib/ipc";
import { writeDiagnosticClipboard } from "../../lib/diagnosticClipboard";
import { supportDiagnosticSummary, type DiagnosticStatus, type DiagnosticSummaryEvent } from "./supportDiagnosticSummary";

const copy = {
  en: {
    title: "Diagnostics", copy: "Copy diagnostics", issue: "GitHub feedback",
    refresh: "Refresh diagnostics", refreshing: "Preparing diagnostics…", refreshed: "Diagnostics refreshed.",
    status: "Session status", lastError: "Latest error", recent: "Recent events", sinceStart: "Since app start", noEvents: "No events yet",
    failures: { unknown: "Unclassified error", authentication: "Authentication failed", credential_storage: "Credential storage unavailable", device_unavailable: "Sound output unavailable", timeout: "Service timed out", request_rejected: "Request rejected", service_error: "Service unavailable", rate_limit: "Service rate limited", backlog: "Processing fell behind", transport: "Connection interrupted", stopped: "Audio capture stopped", processing: "Audio processing failed", size_limit: "Subtitle response too large" },
    lifecycle: { start_requested: "Start requested", start_busy: "Start in progress", start_already_active: "Already running", start_superseded: "Start cancelled", stop_requested: "Stop requested", pause_requested: "Pause requested", resume_requested: "Resume requested" },
    recovery: { retrying: "Reconnecting", recovered: "Connection restored", retries_exhausted: "Automatic retries exhausted", user_stopped: "Stopped by user" },
    copied: "Copied successfully", failed: "Could not copy. Try again or copy the report below manually.",
    prepareFailed: "Could not prepare diagnostics. Try again.",
    opened: "GitHub opened. Review the report and describe the problem before submitting.",
    paste: "GitHub opened. Copy diagnostics, then paste them into the report before submitting.",
    openFailed: "Could not open GitHub. Copy diagnostics and visit the Mimi repository.", preview: "Raw diagnostic data",
  },
  zh: {
    title: "诊断", copy: "复制诊断信息", issue: "GitHub 反馈",
    refresh: "刷新诊断", refreshing: "正在准备诊断信息…", refreshed: "诊断信息已刷新。",
    status: "字幕状态", lastError: "最近错误", recent: "最近事件", sinceStart: "距应用启动", noEvents: "还没有事件",
    failures: { unknown: "未分类错误", authentication: "身份验证失败", credential_storage: "凭据存储不可用", device_unavailable: "声音输出不可用", timeout: "服务超时", request_rejected: "请求被拒绝", service_error: "服务不可用", rate_limit: "服务限流", backlog: "处理速度落后", transport: "连接中断", stopped: "音频捕获停止", processing: "音频处理失败", size_limit: "字幕响应过长" },
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
    failures: { unknown: "未分類のエラー", authentication: "認証に失敗", credential_storage: "認証情報ストアを利用不可", device_unavailable: "音声出力を利用不可", timeout: "サービスがタイムアウト", request_rejected: "リクエストが拒否されました", service_error: "サービスを利用不可", rate_limit: "サービスの利用制限", backlog: "処理が遅れています", transport: "接続が中断", stopped: "音声の取得が停止", processing: "音声処理に失敗", size_limit: "字幕の応答が長すぎます" },
    lifecycle: { start_requested: "開始を要求", start_busy: "開始処理中", start_already_active: "すでに実行中", start_superseded: "開始をキャンセル", stop_requested: "停止を要求", pause_requested: "一時停止を要求", resume_requested: "再開を要求" },
    recovery: { retrying: "再接続中", recovered: "接続が復旧", retries_exhausted: "自動再試行の上限に到達", user_stopped: "ユーザーが停止" },
    copied: "コピー成功", failed: "コピーできませんでした。再試行するか、下のテキストを手動でコピーしてください。",
    prepareFailed: "診断情報を準備できませんでした。再試行してください。",
    opened: "GitHub を開きました。内容を確認し、問題を説明してから送信してください。",
    paste: "GitHub を開きました。診断情報をコピーして本文に貼り付けてから送信してください。",
    openFailed: "GitHub を開けませんでした。診断情報をコピーし、Mimi リポジトリにアクセスしてください。", preview: "診断の生データ",
  },
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

function eventLabel(event: DiagnosticSummaryEvent, text: typeof copy.en | typeof copy.zh | typeof copy.ja): string {
  switch (event.kind) {
    case "status": return statusLabel(event.status);
    case "lifecycle": return text.lifecycle[event.action];
    case "failure": return text.failures[event.category];
    case "recovery": return text.recovery[event.action];
    case "translation_backoff": return event.reason === "rateLimited"
      ? event.retryScheduled ? I18N.overlay.translationRateLimited : I18N.overlay.translationLimited
      : event.retryScheduled ? I18N.overlay.translationRetrying : I18N.overlay.translationUnavailable;
  }
}

export function SupportDiagnostics({ visible = false }: { visible?: boolean }) {
  const [report, setReport] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState<DiagnosticAction | null>(null);
  const { beginToast, clearToast } = useSettingsToast();
  const [manualCopy, setManualCopy] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const operation = useRef(false);
  const request = useRef(0);
  const lifetime = useRef(0);
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
    setPendingAction(action);
    setManualCopy(false);
    const showToast = beginToast();
    const currentLifetime = lifetime.current;
    const notify = (value: Feedback) => {
      if (value && currentRequest === request.current && currentLifetime === lifetime.current) showToast(copy[effectiveUiLanguage()][value], isFailure(value));
    };
    try {
      if (action === "issue") {
        // Backend builds the fixed public destination from typed whitelist facts.
        const issue = await invoke<SupportIssue>("app_open_support_issue");
        const value = diagnosticReport(issue.report);
        if (currentRequest === request.current) setReport(value);
        notify(issue.requiresPaste ? "paste" : "opened");
      } else {
        const value = invoke<string>("support_diagnostics").then((result) => {
          const report = diagnosticReport(result);
          if (currentRequest === request.current) setReport(report);
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
            } catch { notify("prepareFailed"); }
          }
        }
      }
    } catch { notify(action === "issue" ? "openFailed" : "prepareFailed"); }
    finally {
      if (currentRequest === request.current) { operation.current = false; setPendingAction(null); }
    }
  }, [beginToast]);
  useEffect(() => {
    if (!visible) {
      lifetime.current += 1;
      clearToast();
      return;
    }
    if (!isTauri) return;
    let disposed = false;
    queueMicrotask(() => { if (!disposed) void perform("refresh", false); });
    return () => { disposed = true; };
  }, [visible, perform, clearToast]);
  const text = copy[effectiveUiLanguage()];
  const summary = useMemo(() => supportDiagnosticSummary(report), [report]);
  const busy = pendingAction !== null;
  if (!isTauri) return null;
  return <section className="settings-support-diagnostics settings-diagnostics-page" aria-label={text.title} aria-busy={busy}>
    {summary && <div className="settings-diagnostic-overview">
      <dl className="settings-diagnostic-summary">
        <div><dt>{text.status}</dt><dd className={`settings-diagnostic-status${summary.status === "error" ? " is-error" : ""}`}><span aria-hidden="true" />{statusLabel(summary.status)}</dd></div>
        <div><dt>{I18N.overlay.apiLatency}<SettingsHelp text={I18N.overlay.apiLatencyHelp} label={I18N.settings.helpLabel} /></dt><dd className={summary.apiLatencyMs === null ? "is-unavailable" : undefined}>{summary.apiLatencyMs === null ? I18N.overlay.latencyUnavailable : <>{summary.apiLatencyMs}<span className="settings-diagnostic-unit"> ms</span></>}</dd></div>
        <div><dt>{summary.translationLatencyKind === "follow" ? I18N.overlay.translationFollowLatency : I18N.overlay.translationLatency}<SettingsHelp text={summary.translationLatencyKind === "follow" ? I18N.overlay.translationFollowLatencyHelp : I18N.overlay.translationLatencyHelp} label={I18N.settings.helpLabel} /></dt><dd className={summary.translationLatencyMs === null ? "is-unavailable" : undefined}>{summary.translationLatencyMs === null ? I18N.overlay.latencyUnavailable : <>{summary.translationLatencyMs}<span className="settings-diagnostic-unit"> ms</span></>}</dd></div>
      </dl>
      {summary.lastError && <p className="settings-diagnostic-error"><AlertCircle size={16} aria-hidden="true" /><span>{text.lastError}</span><strong>{text.failures[summary.lastError]}</strong></p>}
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
      <div className="settings-diagnostic-privacy"><SettingsHelp text={I18N.settings.diagnosticsHelp} label={I18N.settings.helpLabel} icon="shield-check" /></div>
    </div>
    {summary && <section className="settings-diagnostic-events" aria-label={text.recent}>
      <div className="settings-diagnostic-events__heading"><h2>{text.recent}</h2><SettingsHelp text={text.sinceStart} label={I18N.settings.helpLabel} /></div>
      {summary.recentEvents.length > 0 ? <ol>{summary.recentEvents.map((event, index) => <li key={`${event.sequence}-${index}`} className={event.kind === "failure" ? "is-error" : undefined}><span className="settings-diagnostic-events__time">{(event.elapsedMs / 1000).toFixed(1)} s</span><span className="settings-diagnostic-events__rail" aria-hidden="true" /><span>{eventLabel(event, text)}</span></li>)}</ol> : <p className="settings-diagnostic-events__empty">{text.noEvents}</p>}
    </section>}
    {report !== null && <details className="settings-diagnostic-preview" open={manualCopy || detailsOpen} onToggle={(event) => { const open = event.currentTarget.open; setDetailsOpen(open); if (!open) setManualCopy(false); }}>
      <summary><ChevronRight size={16} aria-hidden="true" /><span>{text.preview}</span></summary>
      {(manualCopy || detailsOpen) && (manualCopy ? <textarea aria-label={text.preview} value={report} readOnly rows={10} wrap="off" spellCheck={false} /> : <pre tabIndex={0} aria-label={text.preview}>{report}</pre>)}
    </details>}
  </section>;
}
