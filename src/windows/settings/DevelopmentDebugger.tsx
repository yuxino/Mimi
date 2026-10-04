import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "../../components/Icon";
import { effectiveUiLanguage } from "../../lib/i18n";
import { isTauri } from "../../lib/ipc";
import { useStore } from "../../lib/store";
import type { AudioSource, SessionStateEvent, SettingsSnapshot } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";
import { useSettingsToast } from "./useSettingsToast";
import { buildMultiSourceSubtitleBlocks, buildSubtitleBlocks, usesAtomicSubtitlePreview, visibleLiveSubtitles } from "../overlay/overlayModel";
import { Timeline } from "../overlay/Timeline";
import "./development-debugger.css";

export interface DebugEntry { id: number; elapsedMs: number; event: { kind: string; [field: string]: unknown } }
export interface DebuggerSnapshot {
  trace: { available: boolean; enabled: boolean; entryLimit: number; recorded: number; evicted: number; frontendDropped: number; entries: DebugEntry[] };
  route: Record<string, unknown>;
  audio: { enabled?: boolean; limited?: boolean; failedStorage?: boolean; error?: string; sources?: { source: AudioSource; sampleRateHz: number; bytes: number; successfulChunks: number; failedChunks: number; cancelledChunks: number; droppedChunks: number }[] };
  caseId: string | null; replaySnapshots: number; privateEvents?: number; contentBytes: number; contentDropped: number; contentLimited: boolean; contentFailed: boolean;
  tracePersistenceEnabled?: boolean; persistedTraceEntries?: number; traceBytes?: number;
  traceDropped?: number; traceLimited?: boolean; traceFailed?: boolean;
}
interface SavedCase { id: string; createdAtUnixMs: number; snapshots: number; provider: string | null }
interface PrivateEvent { elapsedMs: number; event: { kind: string; [field: string]: unknown } }
function failureMessage(error: unknown, fallback: string): string {
  const language = effectiveUiLanguage();
  const reasons: [string[], [string, string, string]][] = [
    [["development_evidence_storage_limit"], ["本地案例存储已达上限。请先导出并清理不再需要的案例。", "Local case storage is full. Export and remove cases you no longer need.", "ケースの保存上限に達しました。不要なケースをエクスポートして削除してください。"]],
    [["development_debug_busy", "development_debug_already_running"], ["另一项调试操作正在进行，请稍后重试。", "Another debugger operation is in progress. Try again shortly.", "別のデバッグ操作が実行中です。少し待って再試行してください。"]],
    [["development_evidence_trace_busy"], ["事件写入尚未完成，案例仍保留。请稍后再次停止取证。", "Event writes are still pending; the case is preserved. Retry stopping recording shortly.", "イベントの書込みが継続中です。ケースは保持されています。少し待って記録の停止を再試行してください。"]],
    [["development_recording_requires_stop", "development_evidence_requires_stop", "development_export_requires_stop"], ["请先停止字幕并完成取证，再执行此操作。", "Stop subtitles and finish recording before this operation.", "字幕と記録を停止してから実行してください。"]],
    [["development_evidence_case_changed", "development_case_invalid"], ["案例已切换或文件无效，请重新打开案例。", "The case changed or its files are invalid. Open the case again.", "ケースが変更されたか無効です。ケースを開き直してください。"]],
    [["development_evidence_storage_failed", "development_evidence_writer_failed", "development_audio_stop_failed", "development_export_failed"], ["案例文件保存失败，请检查本地磁盘与文件权限后重试。", "Saving case files failed. Check local disk space and file permissions, then retry.", "ケースを保存できませんでした。空き容量とファイル権限を確認してください。"]],
  ];
  const match = reasons.find(([codes]) => typeof error === "string" && codes.includes(error));
  return match ? match[1][language === "zh" ? 0 : language === "ja" ? 2 : 1] : fallback;
}
const copy = {
  zh: { title: "开发调试器", start: "开始追踪", evidence: "录制音频与字幕", stop: "停止追踪", export: "导出案例", help: "仅 dev 可用。追踪记录不含正文；录制按钮会明确保存选中输入发给服务的音频及字幕快照到私有本地文件。重启后默认关闭。", live: "当前识别与译文", timeline: "事件时间线", all: "全部阶段", provider: "服务事件", reduced: "字幕处理", frontend: "前端", pipeline: "音频与请求", snapshot: "快照", loss: "证据缺失", audio: "发送给服务的音频", listen: "载入试听", audioHelp: "从实际音频消息解码，包含本机 socket 发送完成的 PCM。未证明服务收到或消费。失败、取消及取证丢失需查看发送记录。停止字幕后试听，避免播放声再次被采集。", system: "系统声音", microphone: "麦克风", stopFirst: "停止字幕后可试听", replay: "字幕快照回放", previous: "上一步", next: "下一步", raw: "原文", translated: "译文", empty: "尚无记录", idle: "已停止", active: "追踪中", saved: "案例已导出", failed: "操作失败，请重试。", details: "事件字段", workspace: "工作区", route: "本次配置", recordHelp: "音频与字幕取证默认关闭，案例包含私人内容；不会自动分享。每次录制生成独立文件，不覆盖旧案例。", replayHelp: "离线重放记录的后端快照，使用相同的字幕选择与排版逻辑。逐步查看不会模拟当时的稳定器时间；原始提交时序和裁切以时间线为准。", final: "已确认字幕", clear: "清空", pause: "暂停", pending: "等待翻译", events: "事件", noSubtitles: "暂无识别或译文" },
  en: { title: "Development debugger", start: "Start trace", evidence: "Record audio and subtitles", stop: "Stop trace", export: "Export case", help: "Dev only. Traces omit content; recording explicitly saves selected-input sent audio and subtitle snapshots to private local files. Off after restart.", live: "Current recognition and translation", timeline: "Event timeline", all: "All stages", provider: "Provider events", reduced: "Subtitle reduction", frontend: "Frontend", pipeline: "Audio and requests", snapshot: "Snapshots", loss: "Missing evidence", audio: "Audio sent to the service", listen: "Load audio", audioHelp: "PCM decoded from actual audio messages whose local socket send completed. This does not prove server receipt or consumption. Check failures, cancellation and evidence loss. Stop subtitles before listening to avoid recapture.", system: "System audio", microphone: "Microphone", stopFirst: "Stop subtitles to listen", replay: "Subtitle snapshot replay", previous: "Previous", next: "Next", raw: "Original", translated: "Translation", empty: "No records yet", idle: "Stopped", active: "Tracing", saved: "Case exported", failed: "Operation failed. Try again.", details: "Event fields", workspace: "Workspace", route: "Captured configuration", recordHelp: "Audio and subtitle evidence is off by default and contains private content. It is never shared automatically. Each recording creates independent files and preserves older cases.", replayHelp: "Offline replay uses the same subtitle projection and layout. Stepping does not reproduce the original stabilizer timing; use the trace for original commits and clipping.", final: "Confirmed subtitles", clear: "Clear", pause: "Paused", pending: "Translation pending", events: "Events", noSubtitles: "No recognition or translation yet" },
  ja: { title: "開発デバッガー", start: "追跡を開始", evidence: "音声と字幕を記録", stop: "追跡を停止", export: "ケースをエクスポート", help: "dev 専用。追跡に本文は含みません。記録では選択入力から送信した音声と字幕をローカルに保存します。再起動後は無効です。", live: "現在の認識と翻訳", timeline: "イベント時系列", all: "全段階", provider: "サービスイベント", reduced: "字幕処理", frontend: "フロントエンド", pipeline: "音声とリクエスト", snapshot: "スナップショット", loss: "欠落した証拠", audio: "サービスへ送信した音声", listen: "音声を読み込む", audioHelp: "ローカル socket の送信が完了した音声メッセージから PCM を復元します。サーバーの受信を証明するものではありません。失敗、取消、記録の欠落を確認してください。再取得を防ぐため字幕を停止してから試聴します。", system: "システム音声", microphone: "マイク", stopFirst: "試聴するには字幕を停止", replay: "字幕の再生", previous: "前へ", next: "次へ", raw: "原文", translated: "翻訳", empty: "記録なし", idle: "停止", active: "追跡中", saved: "エクスポート完了", failed: "操作に失敗しました。再試行してください。", details: "イベント詳細", workspace: "ワークスペース", route: "記録時の設定", recordHelp: "音声と字幕の記録は初期状態で無効です。個人の内容を含み、自動共有しません。毎回別のファイルを作成し、以前のケースを保持します。", replayHelp: "記録済みの状態を同じ字幕選択とレイアウトで表示します。ステップ操作では当時の安定化時間を再現しません。元の描画と切り取りは時系列で確認してください。", final: "確定字幕", clear: "消去", pause: "一時停止", pending: "翻訳待ち", events: "イベント", noSubtitles: "認識・翻訳はまだありません" },
};

function traceEntryLabel(entry: DebugEntry): string {
  const event = entry.event;
  if (event.kind === "pipeline") return String(event.label);
  if (event.kind === "stopped") return `stopped · unflushedWindows=${JSON.stringify(event.unflushedWindows)}`;
  if (event.kind === "provider" || event.kind === "frontend") {
    const observation = event.observation as Record<string, unknown>;
    return event.kind === "provider"
      ? `${observation.source} · ${observation.eventKind} → ${observation.admission} · S:${observation.sourceCharacters} T:${observation.translationCharacters}`
      : `${observation.window} · ${observation.stage} · #${observation.snapshotId} · S:${observation.sourceCharacters} T:${observation.translationCharacters}${observation.projection ? ` · ${observation.projection}` : ""}`;
  }
  if (event.kind === "reduced") {
    const before = event.before as Record<string, number>; const after = event.after as Record<string, number>;
    return `${event.source} · changed=${event.changed} · S:${before.sourceCharacters}→${after.sourceCharacters} T:${before.translationCharacters}→${after.translationCharacters} H:${before.historyEntries}→${after.historyEntries}`;
  }
  return `${event.kind} · #${event.snapshotId}${event.kind === "published" ? ` · delivered=${event.delivered}` : ""}`;
}

function ReplayTimeline({ session, settings, route }: { session: SessionStateEvent; settings: SettingsSnapshot; route: Record<string, unknown> }) {
  const mode = ["original", "translation", "bilingual"].includes(String(route.subtitleDisplayMode)) ? route.subtitleDisplayMode as SettingsSnapshot["subtitleDisplayMode"] : settings.subtitleDisplayMode;
  const showTimestamps = typeof route.showSubtitleTimestamps === "boolean" ? route.showSubtitleTimestamps : settings.showSubtitleTimestamps ?? false;
  const blendsWithBackground = typeof route.blendsWithBackground === "boolean" ? route.blendsWithBackground : settings.subtitleBlendsWithBackground;
  const replaySettings = { ...settings, showIntermediateSubtitles: typeof route.showIntermediateSubtitles === "boolean" ? route.showIntermediateSubtitles : settings.showIntermediateSubtitles, subtitleDisplayMode: mode, sourceLanguage: (route.sourceLanguage ?? settings.sourceLanguage) as SettingsSnapshot["sourceLanguage"], targetLanguage: (route.targetLanguage ?? settings.targetLanguage) as SettingsSnapshot["targetLanguage"] };
  const audioInput = route.audioInput ?? settings.audioInput;
  const primaryAudioSource = session.subtitles.tracks?.[0]?.audioSource ?? (audioInput === "microphone" ? "microphone" : "system");
  const tracks = session.subtitles.tracks?.length ? session.subtitles.tracks : [{ ...session.subtitles, audioSource: primaryAudioSource, detectedLanguage: session.detectedLanguage, isTranslationPending: session.isTranslationPending, isTranslationTimedOut: session.isTranslationTimedOut }];
  const atomic = usesAtomicSubtitlePreview(route.provider);
  const tail = (subtitles: typeof session.subtitles, signals: Pick<SessionStateEvent, "detectedLanguage" | "isTranslationPending" | "isTranslationTimedOut">) => {
    const selected = visibleLiveSubtitles(subtitles, replaySettings, signals.detectedLanguage, signals.isTranslationPending, signals.isTranslationTimedOut, atomic && subtitles.previewPair !== undefined);
    return { source: selected.find(p => p.kind === "source")?.text ?? null, translation: selected.find(p => p.kind === "translation")?.text ?? null, utteranceId: selected[0]?.utteranceId, isStreaming: false };
  };
  const dual = new Set([...tracks.map(track => track.audioSource), ...session.subtitles.history.map(pair => pair.audioSource ?? "system")]).size > 1;
  const blocks = dual ? buildMultiSourceSubtitleBlocks(session.subtitles.history, mode, tracks.map(track => ({ audioSource: track.audioSource, history: track.history, tail: tail(track, track) }))) : buildSubtitleBlocks(session.subtitles.history, mode, tail(session.subtitles, session)).map(block => ({ ...block, audioSource: block.audioSource ?? primaryAudioSource }));
  return <div className="development-debugger__replay"><Timeline blocks={blocks} fontSize={Number(route.fontSize) || settings.fontSize} alignment={(route.subtitleAlignment ?? settings.subtitleAlignment) as SettingsSnapshot["subtitleAlignment"]} color={(route.subtitleColor ?? settings.subtitleColor) as SettingsSnapshot["subtitleColor"]} microphoneColor={(route.microphoneSubtitleColor ?? settings.microphoneSubtitleColor) as SettingsSnapshot["microphoneSubtitleColor"]} displayMode={mode} showTimestamps={showTimestamps} showAudioSources={dual} blendsWithBackground={blendsWithBackground} motionEnabled={false} keepTextOpaque /></div>;
}

export function DevelopmentDebugger({ visible }: { visible: boolean }) {
  const session = useStore(s => s.session); const settings = useStore(s => s.settings);
  const text = copy[effectiveUiLanguage()];
  const [report, setReport] = useState<DebuggerSnapshot | null>(null);
  const [pending, setPending] = useState(false);
  const { beginToast } = useSettingsToast();
  const [filter, setFilter] = useState("all"); const [audio, setAudio] = useState<Partial<Record<AudioSource, string>>>({});
  const [replayIndex, setReplayIndex] = useState(0); const [replay, setReplay] = useState<{ elapsedMs: number; snapshot: SessionStateEvent; projectionSettings?: Record<string, unknown> } | null>(null);
  const [replayTarget, setReplayTarget] = useState(1);
  const [savedCases, setSavedCases] = useState<SavedCase[]>([]); const [selectedCase, setSelectedCase] = useState("");
  const [privateEvents, setPrivateEvents] = useState<PrivateEvent[]>([]); const [privateOffset, setPrivateOffset] = useState(0);
  const [savedTrace, setSavedTrace] = useState<DebugEntry[]>([]);
  const [savedTraceOffset, setSavedTraceOffset] = useState(0); const [showSavedTrace, setShowSavedTrace] = useState(false);
  const operation = useRef(false); const lifetime = useRef(0); const audioUrls = useRef<Partial<Record<AudioSource, string>>>({});
  const playbackEpoch = useRef(0); const currentCase = useRef<string | null>(null);
  const resetCaseView = useCallback(() => {
    playbackEpoch.current += 1;
    Object.values(audioUrls.current).forEach(url => URL.revokeObjectURL(url)); audioUrls.current = {}; setAudio({}); setReplay(null); setReplayIndex(0); setReplayTarget(1); setPrivateEvents([]); setPrivateOffset(0);
    setSavedTrace([]); setSavedTraceOffset(0); setShowSavedTrace(false);
  }, []);
  const applyReport = useCallback((result: DebuggerSnapshot) => {
    if (currentCase.current !== result.caseId) { currentCase.current = result.caseId; resetCaseView(); }
    setReport(result);
  }, [resetCaseView]);
  useEffect(() => {
    return () => { lifetime.current += 1; Object.values(audioUrls.current).forEach(url => URL.revokeObjectURL(url)); };
  }, []);
  useEffect(() => {
    if (!visible || !isTauri) return;
    let disposed = false; let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      const epoch = playbackEpoch.current;
      try { const result = await invoke<DebuggerSnapshot>("development_debug_snapshot"); if (!disposed && epoch === playbackEpoch.current) applyReport(result); }
      catch { /* Unsupported hosts keep this dev-only surface hidden. */ }
      if (!disposed) timer = setTimeout(() => { void refresh(); }, 500);
    };
    void refresh(); return () => { disposed = true; clearTimeout(timer); };
  }, [visible, applyReport]);
  useEffect(() => useStore.subscribe((state, previous) => {
    if (!state.session.isActive || previous.session.isActive) return;
    playbackEpoch.current += 1;
    Object.values(audioUrls.current).forEach(url => URL.revokeObjectURL(url)); audioUrls.current = {};
    setAudio({});
  }), []);
  async function perform(action: (notify: (message: string, failure?: boolean) => void) => Promise<void>) {
    if (operation.current) return; operation.current = true; setPending(true);
    const notify = beginToast();
    const generation = lifetime.current;
    try { await action(notify); if (generation === lifetime.current) applyReport(await invoke<DebuggerSnapshot>("development_debug_snapshot")); }
    catch (error) { if (generation === lifetime.current) notify(failureMessage(error, text.failed), true); }
    finally { operation.current = false; if (generation === lifetime.current) setPending(false); }
  }
  async function start(withAudio: boolean) {
    await invoke("development_debug_start", { withAudio });
    resetCaseView();
  }
  async function finish() {
    if (report?.caseId && useStore.getState().session.isActive) await invoke("session_stop");
    await invoke("development_debug_stop");
  }
  async function loadAudio(source: AudioSource) {
    if (useStore.getState().session.isActive || report?.trace.enabled || !report?.caseId) return;
    const generation = lifetime.current;
    const epoch = playbackEpoch.current;
    const bytes = await invoke<ArrayBuffer>("development_debug_audio", { source, caseId: report.caseId });
    if (generation !== lifetime.current || epoch !== playbackEpoch.current || useStore.getState().session.isActive) return;
    const url = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
    const previous = audioUrls.current[source]; if (previous) URL.revokeObjectURL(previous);
    audioUrls.current[source] = url; setAudio(previous => ({ ...previous, [source]: url }));
  }
  async function loadReplay(index: number) {
    const result = await invoke<{ elapsedMs: number; snapshot: SessionStateEvent; projectionSettings?: Record<string, unknown> }>("development_debug_replay", { index, caseId: report?.caseId });
    setReplay(result); setReplayIndex(index); setReplayTarget(index + 1);
  }
  async function listCases() {
    const cases = await invoke<SavedCase[]>("development_debug_cases"); setSavedCases(cases); setSelectedCase(cases[0]?.id ?? "");
  }
  async function openCase() {
    await invoke("development_debug_open_case", { caseId: selectedCase }); resetCaseView();
  }
  async function loadPrivateEvents(offset: number) {
    const result = await invoke<PrivateEvent[]>("development_debug_private_events", { caseId: report?.caseId, offset }); setPrivateEvents(result); setPrivateOffset(offset);
  }
  async function loadSavedTrace(offset: number) {
    const caseId = report?.caseId; const generation = lifetime.current;
    if (!caseId || report?.trace.enabled) return;
    const result = await invoke<DebugEntry[]>("development_debug_trace_events", { caseId, offset, limit: 64 });
    if (generation !== lifetime.current || currentCase.current !== caseId) return;
    setSavedTrace([...result].sort((a, b) => a.id - b.id)); setSavedTraceOffset(offset); setShowSavedTrace(true);
  }
  if (!report?.trace.available) return null;
  const audioLoss = (report.audio.sources ?? []).reduce((total, source) => total + source.droppedChunks, 0);
  const durableTrace = report.tracePersistenceEnabled === true;
  const loss = (durableTrace ? 0 : report.trace.evicted) + report.trace.frontendDropped + report.contentDropped + audioLoss;
  const liveTracks = session.subtitles.tracks?.length ? session.subtitles.tracks : [{ ...session.subtitles, audioSource: "system" as const }];
  const entries = (showSavedTrace ? savedTrace : report.trace.entries).filter(entry => filter === "all" || entry.event.kind === filter || filter === "snapshot" && entry.event.kind === "published");
  const traceLabels = effectiveUiLanguage() === "zh" ? {
    saved: "落盘事件", load: "查看落盘事件", recent: "最近事件", help: "最近事件只保留内存尾部，移出不代表已保存案例丢失。落盘分页按入库位置读取，每页按事件编号排序；完整复盘按编号归序。落盘缺失、上限和写入失败会单独提示。",
  } : effectiveUiLanguage() === "ja" ? {
    saved: "保存済みイベント", load: "保存済みイベントを表示", recent: "最近のイベント", help: "最近のイベントはメモリ末尾のみです。メモリからの削除は保存済みケースの欠落ではありません。保存ページは書込み位置で読み、ページ内はイベント番号順です。完全な分析は番号順に並べます。保存の欠落、上限と失敗は別途表示します。",
  } : {
    saved: "Saved events", load: "Load saved events", recent: "Recent events", help: "Recent events retain only the memory tail. Eviction does not mean a saved case lost events. Pages use file positions and sort each page by event ID; full analysis orders all IDs. Saved-event loss, limits and failures are reported separately.",
  };
  const labels = effectiveUiLanguage() === "zh" ? { saved: "已保存案例", refresh: "查看已保存案例", open: "打开案例", private: "识别结果与翻译请求正文", load: "查看正文", raw: "记录的完整字幕状态" } : effectiveUiLanguage() === "ja" ? { saved:"保存済みケース",refresh:"保存済みケースを表示",open:"ケースを開く",private:"認識結果と翻訳リクエスト本文",load:"本文を表示",raw:"記録済み字幕状態" } : { saved:"Saved cases",refresh:"List saved cases",open:"Open case",private:"Recognition and translation request content",load:"Inspect content",raw:"Recorded subtitle state" };
  return <section className="development-debugger" aria-labelledby="development-debugger-title">
    <header className="development-debugger__header"><h2 id="development-debugger-title">{text.title}</h2><SettingsHelp text={text.help} label={text.title} /><span className={`development-debugger__status${report.trace.enabled ? " is-active" : ""}`}>{report.trace.enabled ? text.active : text.idle}</span></header>
    <div className="development-debugger__actions development-debugger__toolbar">
      {report.trace.enabled ? <button className="settings-button" disabled={pending} onClick={() => { void perform(finish); }}><Icon name="stop" />{report.caseId && session.isActive ? (effectiveUiLanguage() === "zh" ? "停止字幕并完成取证" : effectiveUiLanguage() === "ja" ? "字幕を停止して記録を終了" : "Stop subtitles and finish recording") : text.stop}</button> : <>
        <button className="settings-button" disabled={pending} onClick={() => { void perform(() => start(false)); }}><Icon name="play" />{text.start}</button>
        <span className="development-debugger__record"><button className="settings-button" disabled={pending || session.isActive} onClick={() => { void perform(() => start(true)); }}><Icon name="waves" />{session.isActive ? text.stopFirst : text.evidence}</button><SettingsHelp text={text.recordHelp} label={text.evidence} /></span>
      </>}
      <button className="settings-button" disabled={pending || report.trace.enabled || !report.trace.recorded} onClick={() => { void perform(async notify => { if (await invoke<boolean>("development_debug_export")) notify(text.saved); }); }}><Icon name="download" />{text.export}</button>
      <button className="settings-button" disabled={pending || report.trace.enabled} onClick={() => { void perform(listCases); }}>{labels.refresh}</button>
    </div>
    {savedCases.length > 0 && <div className="development-debugger__actions development-debugger__cases"><select aria-label={labels.saved} value={selectedCase} disabled={pending || report.trace.enabled} onChange={event => setSelectedCase(event.target.value)}>{savedCases.map(item => <option key={item.id} value={item.id}>{new Date(item.createdAtUnixMs).toLocaleString()} · {item.provider} · {item.snapshots} · {item.id.slice(0, 8)}</option>)}</select><button className="settings-button" disabled={pending || report.trace.enabled || !selectedCase} onClick={() => { void perform(openCase); }}>{labels.open}</button></div>}
    {report.caseId && <code className="development-debugger__case-id">{report.caseId}</code>}
    <dl className="development-debugger__metrics"><div><dt>{text.workspace}</dt><dd>{String(report.route.evidenceWorkspace ?? (report.route.evidenceWorkspaceError ? "—" : "default"))}</dd></div><div><dt>{text.events}</dt><dd>{report.trace.recorded}</dd></div><div><dt>{text.snapshot}</dt><dd>{report.replaySnapshots}</dd></div>{durableTrace && <div><dt>{traceLabels.saved}</dt><dd>{report.persistedTraceEntries ?? 0}</dd></div>}<div><dt>{text.loss}</dt><dd>{loss}</dd></div></dl>
    {(loss > 0 || report.contentLimited || report.contentFailed || report.audio.limited || report.audio.failedStorage || report.traceLimited || report.traceFailed) && <p className="development-debugger__warning" role="alert">{text.loss}: evicted={report.trace.evicted} frontend={report.trace.frontendDropped} content={report.contentDropped} audio={audioLoss}{durableTrace && ` savedTrace=${report.traceDropped ?? 0}`} limited={String(report.contentLimited || report.audio.limited || report.traceLimited || false)} failed={String(report.contentFailed || report.audio.failedStorage || report.traceFailed || false)}</p>}
    <details className="development-debugger__disclosure"><summary><Icon name="chevron-right" />{text.route}</summary><pre>{JSON.stringify(report.route, null, 2)}</pre></details>
    {report.caseId && <div className="development-debugger__section"><h3>{text.audio}<SettingsHelp text={text.audioHelp} label={text.audio} /></h3>
      {(["system", "microphone"] as const).filter(source => source === report.route.audioInput || report.route.audioInput === "both" || report.audio.sources?.some(item => item.source === source && item.successfulChunks + item.failedChunks + item.cancelledChunks + item.droppedChunks > 0)).map(source => {
        const evidence = report.audio.sources?.find(item => item.source === source);
        const seconds = evidence?.sampleRateHz ? evidence.bytes / (2 * evidence.sampleRateHz) : 0;
        return <div className="development-debugger__audio" key={source}><span>{source === "system" ? text.system : text.microphone} · {seconds.toFixed(2)} s · sent={evidence?.successfulChunks ?? 0} failed={evidence?.failedChunks ?? 0} cancelled={evidence?.cancelledChunks ?? 0} missing={evidence?.droppedChunks ?? 0}</span><button className="settings-button" disabled={pending || session.isActive || report.trace.enabled || !evidence?.bytes} onClick={() => { void perform(() => loadAudio(source)); }}>{session.isActive ? text.stopFirst : text.listen}</button>{audio[source] && !session.isActive && <audio controls src={audio[source]} aria-label={source === "system" ? text.system : text.microphone} />}</div>;
      })}
      <details className="development-debugger__disclosure"><summary><Icon name="chevron-right" />{text.details}</summary><pre>{JSON.stringify(report.audio, null, 2)}</pre></details>
    </div>}
    <div className="development-debugger__section"><h3>{text.live}</h3>{liveTracks.some(track => track.source.text || track.translation.text) ? liveTracks.map(track => <div className="development-debugger__text" key={track.audioSource}><strong>{track.audioSource === "system" ? text.system : text.microphone}</strong><dl><dt>{text.raw}</dt><dd>{track.source.text || "—"}</dd><dt>{text.translated}</dt><dd>{track.translation.text || "—"}</dd></dl></div>) : <p className="development-debugger__empty">{text.noSubtitles}</p>}</div>
    {report.replaySnapshots > 0 && <div className="development-debugger__section"><h3>{text.replay}<SettingsHelp text={text.replayHelp} label={text.replay} /></h3><div className="development-debugger__actions"><button className="settings-button" disabled={pending || replayIndex === 0 && replay !== null} onClick={() => { void perform(() => loadReplay(Math.max(0, replayIndex - 1))); }}><Icon name="chevron-left" />{text.previous}</button><span>{replayIndex + 1} / {report.replaySnapshots} {replay && `· ${replay.elapsedMs} ms · #${replay.snapshot.debugSnapshotId}`}</span><input type="number" className="development-debugger__seek" aria-label={effectiveUiLanguage() === "zh" ? "快照序号" : effectiveUiLanguage() === "ja" ? "スナップショット番号" : "Snapshot number"} min={1} max={report.replaySnapshots} value={replayTarget} disabled={pending} onChange={event => setReplayTarget(Number(event.target.value))} /><button className="settings-button" disabled={pending || !Number.isInteger(replayTarget) || replayTarget < 1 || replayTarget > report.replaySnapshots} onClick={() => { void perform(() => loadReplay(replayTarget - 1)); }}>{effectiveUiLanguage() === "zh" ? "跳转" : effectiveUiLanguage() === "ja" ? "移動" : "Go"}</button><button className="settings-button" disabled={pending || replay !== null && replayIndex + 1 >= report.replaySnapshots} onClick={() => { void perform(() => loadReplay(replay ? replayIndex + 1 : 0)); }}>{text.next}<Icon name="chevron-right" /></button></div>{replay && <ReplayTimeline session={replay.snapshot} settings={settings} route={{ ...report.route, ...replay.projectionSettings }} />}</div>}
    {(report.privateEvents ?? 0) > 0 && <div className="development-debugger__section"><h3>{labels.private}</h3><div className="development-debugger__actions"><button className="settings-button" disabled={pending || privateOffset === 0 && privateEvents.length > 0} onClick={() => { void perform(() => loadPrivateEvents(Math.max(0, privateOffset - 32))); }}>{text.previous}</button><span>{privateOffset + 1} / {report.privateEvents}</span><button className="settings-button" disabled={pending || privateEvents.length > 0 && privateOffset + 32 >= (report.privateEvents ?? 0)} onClick={() => { void perform(() => loadPrivateEvents(privateEvents.length > 0 ? privateOffset + 32 : 0)); }}>{privateEvents.length ? text.next : labels.load}</button></div>{privateEvents.map((entry, index) => <details key={`${privateOffset}-${index}`}><summary>#{privateOffset + index + 1} · {entry.elapsedMs} ms · {entry.event.kind}{entry.event.protocol ? ` · ${String(entry.event.protocol)} · ${String(entry.event.lane)} · attempt=${String(entry.event.attempt)}` : ""}</summary><pre>{JSON.stringify(entry.event, null, 2)}</pre></details>)}</div>}
    {replay && <details className="development-debugger__disclosure"><summary><Icon name="chevron-right" />{labels.raw} · #{replay.snapshot.debugSnapshotId}</summary><pre>{JSON.stringify(replay.snapshot.subtitles, null, 2)}</pre></details>}
    <div className="development-debugger__section development-debugger__trace"><div className="development-debugger__section-heading"><h3>{text.timeline}<SettingsHelp text={traceLabels.help} label={text.timeline} /></h3><select aria-label={text.timeline} value={filter} onChange={event => setFilter(event.target.value)}>{["all", "provider", "reduced", "frontend", "pipeline", "snapshot"].map(value => <option value={value} key={value}>{text[value as keyof typeof text]}</option>)}</select></div>
      {durableTrace && <div className="development-debugger__actions">
        <button className="settings-button" disabled={pending || !showSavedTrace} onClick={() => setShowSavedTrace(false)}>{traceLabels.recent}</button>
        <button className="settings-button" disabled={pending || report.trace.enabled || !report.persistedTraceEntries} onClick={() => { void perform(() => loadSavedTrace(showSavedTrace ? savedTraceOffset : 0)); }}>{traceLabels.load}</button>
        {showSavedTrace && <><button className="settings-button" disabled={pending || savedTraceOffset === 0} onClick={() => { void perform(() => loadSavedTrace(Math.max(0, savedTraceOffset - 64))); }}>{text.previous}</button><span>{savedTraceOffset + 1}–{Math.min(savedTraceOffset + savedTrace.length, report.persistedTraceEntries ?? 0)} / {report.persistedTraceEntries}</span><button className="settings-button" disabled={pending || savedTraceOffset + 64 >= (report.persistedTraceEntries ?? 0)} onClick={() => { void perform(() => loadSavedTrace(savedTraceOffset + 64)); }}>{text.next}</button></>}
      </div>}
      <div className={`development-debugger__events${entries.length ? "" : " is-empty"}`}>{entries.length ? entries.map(entry => <details key={entry.id}><summary><span>#{entry.id}</span><time>{entry.elapsedMs} ms</time><span>{traceEntryLabel(entry)}</span><Icon name="chevron-right" /></summary><pre>{JSON.stringify(entry.event, null, 2)}</pre></details>) : <p className="development-debugger__empty">{text.empty}</p>}</div>
    </div>
  </section>;
}
