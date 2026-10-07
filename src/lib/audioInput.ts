import { SUPPLEMENTAL_EN } from "./locales/supplemental-schema";
import { supplemental } from "./locales/supplemental";
import { effectiveUiLanguage, I18N } from "./i18n";
import type { AudioInput, SystemAudioTarget } from "./types";

export function audioInputLabel(input: AudioInput = "system", target?: SystemAudioTarget): string {
  const systemLabel = target?.kind === "application" ? target.name : I18N.settings.audioInputSystem;
  return input === "both"
    ? `${systemLabel} + ${I18N.settings.audioInputMicrophone}`
    : input === "microphone" ? I18N.settings.audioInputMicrophone : systemLabel;
}

const copy = {
  en: SUPPLEMENTAL_EN.audioInput_copy,
  zh: {
    unavailable: "麦克风输入暂时不可用，请使用系统声音。",
    missing: "没有可用的麦克风。请连接麦克风，并将其设为系统默认输入。",
    permission: "麦克风权限被拒绝。请在系统隐私设置中允许 Mimi 使用麦克风，然后重试。",
    start: "无法启动麦克风。请检查系统默认输入和麦克风权限，然后重试。",
    running: "音频采集已在运行。", cancelled: "音频采集已取消。",
    stopping: "音频采集仍在停止，请稍后重试。",
    format: "无法处理此音频格式，请在系统声音设置中检查所选输入。",
    stopped: "音频采集意外停止，请检查所选输入，然后重新开启字幕。",
    timeout: "音频采集启动超时，请检查所选输入后重试。",
    server: "请连接支持 PulseAudio 的 PipeWire 或 PulseAudio 服务，然后重试。",
    switchBusy: "请等待当前连接操作完成后重试。",
    switchSuperseded: "字幕状态已改变，请确认当前状态后重试。",
    switchSave: "音频输入未能保存，请重试。",
    switchStop: "之前的音频输入未能停止，请先停止字幕再重试。",
  },
  ja: {
    unavailable: "マイク入力は一時的に利用できません。システム音声を使用してください。",
    missing: "利用できるマイクがありません。接続して、システムの既定の入力に設定してください。",
    permission: "マイクへのアクセスが拒否されました。システムのプライバシー設定で Mimi のマイク利用を許可し、再試行してください。",
    start: "マイクを開始できませんでした。システムの既定の入力とマイクの権限を確認し、再試行してください。",
    running: "音声の取得はすでに実行中です。", cancelled: "音声の取得をキャンセルしました。",
    stopping: "音声の取得を停止中です。しばらくしてから再試行してください。",
    format: "この音声形式を処理できません。システムのサウンド設定で選択した入力を確認してください。",
    stopped: "音声の取得が予期せず停止しました。選択した入力を確認し、字幕を再開してください。",
    timeout: "音声の取得開始がタイムアウトしました。選択した入力を確認し、再試行してください。",
    server: "PulseAudio、または PulseAudio 対応の PipeWire に接続して、再試行してください。",
    switchBusy: "現在の接続処理が完了してから再試行してください。",
    switchSuperseded: "字幕の状態が変わりました。状態を確認してから再試行してください。",
    switchSave: "音声入力を保存できませんでした。再試行してください。",
    switchStop: "以前の音声入力を停止できませんでした。字幕を停止してから再試行してください。",
  },
  ...supplemental.audioInput_copy,
};

const errors: Record<string, keyof typeof copy.en> = {
  microphone_input_unavailable: "unavailable",
  audio_input_switch_busy: "switchBusy",
  audio_input_switch_superseded: "switchSuperseded",
  audio_input_switch_save_failed: "switchSave",
  audio_input_switch_stop_failed: "switchStop",
  "No default microphone is available.": "missing",
  "Microphone capture permission was denied.": "permission",
  "Microphone capture could not be started.": "start",
  "Audio capture is already running.": "running",
  "Audio capture start was cancelled.": "cancelled",
  "The previous audio capture is still stopping.": "stopping",
  "Audio capture could not process the device audio format.": "format",
  "Audio capture stopped unexpectedly.": "stopped",
  "Audio capture setup timed out.": "timeout",
  "Connect to PulseAudio or PipeWire with PulseAudio support to capture audio.": "server",
};

/** Match only safe native labels; never reinterpret arbitrary provider messages. */
export function audioInputErrorMessage(message: string): string | null {
  const key = errors[message];
  return key ? copy[effectiveUiLanguage()][key] : null;
}
