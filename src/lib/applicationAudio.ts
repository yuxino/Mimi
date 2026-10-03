import { effectiveUiLanguage } from "./i18n";

export interface AudioApplication { id: string; name: string }
export interface ApplicationSnapshot { supported: boolean; applications: AudioApplication[] }
const copy = {
  en: {
    stop: "Stop subtitles to change the application.", title: "Capture sound from", all: "All applications", app: "One application", application: "Application",
    choose: "Choose an application", search: "Search applications", refresh: "Refresh", loading: "Loading applications…",
    empty: "No applications found. Open the app, then refresh.", noMatch: "No matching applications.", missing: "Unavailable",
    unavailable: "The selected application is no longer available. Stop subtitles, open the app and select it again.",
    unsupported: "Application audio capture is available on macOS and Windows 11 (build 20348 or later).",
    failed: "Could not load applications. Check audio capture permissions, then refresh.",
    saveFailed: "Could not save the application. Try again.",
    help: "Captures only this application’s audio. Browser tabs share an application. Selecting an app does not mute it. On Windows, select it again after reopening it.",
  },
  zh: {
    stop: "请停止字幕后更换应用。", title: "采集范围", all: "全部应用", app: "指定应用", application: "应用",
    choose: "选择应用", search: "搜索应用", refresh: "刷新", loading: "正在加载应用…",
    empty: "没有找到应用，请打开目标应用后刷新。", noMatch: "没有匹配的应用。", missing: "不可用",
    unavailable: "所选应用已不可用。请停止字幕，打开应用后重新选择。",
    unsupported: "应用声音采集支持 macOS 和 Windows 11（build 20348 及以上）。",
    failed: "无法加载应用，请检查声音采集权限后刷新。", saveFailed: "应用未能保存，请重试。",
    help: "仅采集所选应用的声音，浏览器标签页属于同一个应用。选择应用不会将它静音。Windows 上重新打开应用后需要重新选择。",
  },
  ja: {
    stop: "アプリを変更するには字幕を停止してください。", title: "音声の取得範囲", all: "すべてのアプリ", app: "指定したアプリ", application: "アプリ",
    choose: "アプリを選択", search: "アプリを検索", refresh: "更新", loading: "アプリを読み込み中…",
    empty: "アプリが見つかりません。対象のアプリを開いて更新してください。", noMatch: "一致するアプリがありません。", missing: "利用不可",
    unavailable: "選択したアプリを利用できません。字幕を停止し、アプリを開いて再選択してください。",
    unsupported: "アプリ音声の取得は macOS と Windows 11（ビルド 20348 以降）に対応しています。",
    failed: "アプリを読み込めません。音声取得の権限を確認して更新してください。", saveFailed: "アプリを保存できませんでした。再試行してください。",
    help: "選択したアプリの音声だけを取得します。ブラウザーのタブは同じアプリに属します。選択してもアプリはミュートされません。Windows ではアプリを開き直した後に再選択が必要です。",
  },
};
export function applicationAudioCopy(language = effectiveUiLanguage()) { return copy[language]; }
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
