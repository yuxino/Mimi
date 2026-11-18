import { SUPPLEMENTAL_EN } from "../locales/supplemental-schema";
import { supplemental } from "../locales/supplemental";
import { effectiveUiLanguage } from "../i18n";

export interface AudioApplication { id: string; name: string; iconDataUrl?: string | null }
export interface ApplicationSnapshot { supported: boolean; applications: AudioApplication[] }
const copy = {
  en: SUPPLEMENTAL_EN.applicationAudio_copy,
  zh: {
    title: "采集应用", all: "全部应用", application: "应用",
    search: "搜索应用", refresh: "刷新应用列表", loading: "正在加载应用…",
    empty: "没有找到应用，请打开目标应用后刷新。", noMatch: "没有匹配的应用。", missing: "不可用",
    unavailable: "所选应用已不可用，请打开应用后重新选择，或选择全部应用。",
    unsupported: "应用声音采集支持 macOS 和 Windows 11（build 20348 及以上）。",
    failed: "无法加载应用，请刷新重试。", saveFailed: "采集应用未能切换，请查看字幕状态后重试。",
    help: "仅采集所选应用的声音，浏览器标签页属于同一个应用。选择应用不会将它静音。Windows 上重新打开应用后需要重新选择。",
  },
  ja: {
    title: "音声を取得するアプリ", all: "すべてのアプリ", application: "アプリ",
    search: "アプリを検索", refresh: "アプリ一覧を更新", loading: "アプリを読み込み中…",
    empty: "アプリが見つかりません。対象のアプリを開いて更新してください。", noMatch: "一致するアプリがありません。", missing: "利用不可",
    unavailable: "選択したアプリを利用できません。アプリを開いて選び直すか、すべてのアプリを選択してください。",
    unsupported: "アプリ音声の取得は macOS と Windows 11（ビルド 20348 以降）に対応しています。",
    failed: "アプリを読み込めません。一覧を更新して再試行してください。", saveFailed: "音声を取得するアプリを切り替えられませんでした。字幕の状態を確認して再試行してください。",
    help: "選択したアプリの音声だけを取得します。ブラウザーのタブは同じアプリに属します。選択してもアプリはミュートされません。Windows ではアプリを開き直した後に再選択が必要です。",
  },
  ...supplemental.applicationAudio_copy,
};
const playbackStreamCopy = {
  en: SUPPLEMENTAL_EN.playbackStream_copy,
  zh: {
    title: "采集音频流", all: "全部系统声音", application: "音频流",
    search: "搜索音频流", refresh: "刷新音频流", loading: "正在加载音频流…",
    empty: "没有找到音频流，请先播放声音再刷新。", noMatch: "没有匹配的音频流。", missing: "不可用",
    unavailable: "所选音频流已不可用，请播放声音后重新选择，或选择全部系统声音。",
    unsupported: "音频流采集需要 Linux 上的 PulseAudio 或 PipeWire-Pulse。",
    failed: "无法加载音频流，请刷新重试。", saveFailed: "音频流未能切换，请查看字幕状态后重试。",
    help: "每次采集一条播放流，同一应用的多条流分别列出。不改变播放路由或音量。音频流关闭或 Mimi 重启后需要重新选择。",
  },
  ja: {
    title: "取得する音声ストリーム", all: "すべてのシステム音声", application: "音声ストリーム",
    search: "音声ストリームを検索", refresh: "音声ストリームを更新", loading: "音声ストリームを読み込み中…",
    empty: "音声ストリームがありません。音を再生して更新してください。", noMatch: "一致する音声ストリームがありません。", missing: "利用不可",
    unavailable: "この音声ストリームは利用できません。音を再生して選び直すか、すべてのシステム音声を選択してください。",
    unsupported: "音声ストリームの取得には Linux の PulseAudio または PipeWire-Pulse が必要です。",
    failed: "音声ストリームを読み込めません。更新して再試行してください。", saveFailed: "音声ストリームを切り替えられません。字幕の状態を確認して再試行してください。",
    help: "一つの再生ストリームを取得します。同じアプリの複数のストリームは個別に表示されます。再生先や音量は変更しません。ストリーム終了後や Mimi の再起動後は再選択が必要です。",
  },
  ...supplemental.playbackStream_copy,
};
/** Platform changes wording only; native capability decides whether selection is available. */
export function usesPlaybackStreams() { return typeof navigator !== "undefined" && /Linux/i.test(navigator.userAgent); }
export function applicationAudioCopy(language = effectiveUiLanguage()) { return usesPlaybackStreams() ? playbackStreamCopy[language] : copy[language]; }
export function applicationAudioError(message: string): string | null {
  const text = applicationAudioCopy();
  switch (message) {
    case "application_audio_unavailable": return text.unavailable;
    case "application_audio_unsupported": return text.unsupported;
    case "application_audio_list_failed": return text.failed;
    case "application_audio_invalid_target": return text.saveFailed;
    default: return null;
  }
}
