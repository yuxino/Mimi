import { SUPPLEMENTAL_EN } from "./locales/supplemental-schema";
import { supplemental } from "./locales/supplemental";
import { effectiveUiLanguage } from "./i18n";

/** Follow-system sources. Empty string follows audible outputs with communications/media/console priority. */
export const FOLLOW_SYSTEM = "";
export const ROLE_COMMUNICATIONS = "role:communications";
export const ROLE_MULTIMEDIA = "role:multimedia";
/** Capture whichever device is currently producing sound. */
export const FOLLOW_AUDIBLE = "follow:audible";

/** True for the follow-system family (never a concrete endpoint id). */
export function isRoleSource(value: string): boolean {
  return [FOLLOW_SYSTEM, ROLE_COMMUNICATIONS, ROLE_MULTIMEDIA, "role:console", FOLLOW_AUDIBLE].includes(value);
}

/** True when the value names a device the snapshot can be checked against. */
export function isDeviceSource(value: string): boolean {
  return value !== "" && !isRoleSource(value);
}

export interface AudioSourceSnapshot {
  devices: { id: string; name: string }[];
  currentDevice: string | null;
  receivingSound: boolean;
  receivingAudioData: boolean;
}

const copy = {
  en: SUPPLEMENTAL_EN.windowsAudioSource_copy,
  zh: {
    title: "输出设备", system: "跟随系统", unavailable: "不可用的输出设备",
    communications: "跟随通信设备（通话耳机）",
    multimedia: "跟随媒体设备",
    audible: "跟随当前有声音的设备",
    help: "选择应用实际播放声音的耳机或扬声器。",
    stop: "更换声音来源前，请先停止字幕。",
    missing: "此输出设备不可用，请选择其他声音来源。",
    receiving: "收到声音", silent: "已采到声音数据，但当前没有明显声音。请播放声音，或检查应用的声音输出。",
    noData: "暂未采到声音数据。请播放声音并检查所选声音来源。",
    idle: "开启字幕并播放声音，即可检查是否收到声音。",
    failed: "无法加载声音输出设备，请重新打开设置。",
  },
  ja: {
    title: "出力デバイス", system: "システムに合わせる", unavailable: "利用できない出力先",
    communications: "通信デバイスに合わせる（通話用ヘッドセット）",
    multimedia: "メディアデバイスに合わせる",
    audible: "音が出ているデバイスに合わせる",
    help: "アプリが音声を再生しているヘッドホンやスピーカーを選んでください。",
    stop: "取得元を変更する前に、字幕を停止してください。",
    missing: "この出力先は利用できません。別の取得元を選んでください。",
    receiving: "音声を受信中", silent: "音声データは届いていますが、明確な音がありません。音声を再生するか、アプリの音声出力先を確認してください。",
    noData: "まだ音声データがありません。音声を再生して、選択した取得元を確認してください。",
    idle: "字幕を開始して音声を再生すると、受信を確認できます。",
    failed: "音声出力先を読み込めませんでした。設定を開き直してください。",
  },
  ...supplemental.windowsAudioSource_copy,
};


export function audioSourceCopy() { return copy[effectiveUiLanguage()]; }

export function audioSourceErrorMessage(message: string): string | null {
  return message === "The selected sound output is unavailable. Stop subtitles and choose another sound source in Settings."
    ? audioSourceCopy().missing : null;
}
