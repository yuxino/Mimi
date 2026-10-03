import { effectiveUiLanguage } from "./i18n";

export interface AudioApplication { id: string; name: string }
export interface ApplicationSnapshot { supported: boolean; applications: AudioApplication[] }
const copy = {
  en: {
    title: "Capture sound from", all: "All applications", application: "Application",
    search: "Search applications", refresh: "Refresh applications", loading: "Loading applications…",
    empty: "No applications found. Open the app, then refresh.", noMatch: "No matching applications.", missing: "Unavailable",
    unavailable: "The selected application is no longer available. Open it and select it again, or choose All applications.",
    unsupported: "Application audio capture is available on macOS and Windows 11 (build 20348 or later).",
    failed: "Could not load applications. Check audio capture permissions, then refresh.",
    saveFailed: "Could not switch the captured application. Check subtitle status, then try again.",
    help: "Captures only this application’s audio. Browser tabs share an application. Selecting an app does not mute it. On Windows, select it again after reopening it.",
  },
  zh: {
    title: "采集应用", all: "全部应用", application: "应用",
    search: "搜索应用", refresh: "刷新应用列表", loading: "正在加载应用…",
    empty: "没有找到应用，请打开目标应用后刷新。", noMatch: "没有匹配的应用。", missing: "不可用",
    unavailable: "所选应用已不可用，请打开应用后重新选择，或选择全部应用。",
    unsupported: "应用声音采集支持 macOS 和 Windows 11（build 20348 及以上）。",
    failed: "无法加载应用，请检查声音采集权限后刷新。", saveFailed: "采集应用未能切换，请查看字幕状态后重试。",
    help: "仅采集所选应用的声音，浏览器标签页属于同一个应用。选择应用不会将它静音。Windows 上重新打开应用后需要重新选择。",
  },
  ja: {
    title: "音声を取得するアプリ", all: "すべてのアプリ", application: "アプリ",
    search: "アプリを検索", refresh: "アプリ一覧を更新", loading: "アプリを読み込み中…",
    empty: "アプリが見つかりません。対象のアプリを開いて更新してください。", noMatch: "一致するアプリがありません。", missing: "利用不可",
    unavailable: "選択したアプリを利用できません。アプリを開いて選び直すか、すべてのアプリを選択してください。",
    unsupported: "アプリ音声の取得は macOS と Windows 11（ビルド 20348 以降）に対応しています。",
    failed: "アプリを読み込めません。音声取得の権限を確認して更新してください。", saveFailed: "音声を取得するアプリを切り替えられませんでした。字幕の状態を確認して再試行してください。",
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
