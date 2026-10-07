import { SUPPLEMENTAL_EN } from "./locales/supplemental-schema";
import { supplemental } from "./locales/supplemental";
import { applicationAudioCopy } from "./applicationAudio";
import { effectiveUiLanguage } from "./i18n";
import type { AudioInput, AudioSource, SessionStateEvent } from "./types";

interface CaptureStatusFields {
  kind: "windows_output" | "macos_system_mix" | "linux_output_monitor" | "microphone" | "both" | "unknown" | "application";
  strategy: "follow_system" | "manual_output" | "platform_capture" | "default_input" | "independent_inputs" | "selected_application";
  actualDeviceName: string | null;
  /** Output context is independent of a macOS mixed-audio capture route. */
  systemOutputDeviceName?: string | null;
  observation: { pcmDataRecent: boolean; soundRecent: boolean } | null;
}
export interface CaptureSourceStatus extends CaptureStatusFields { audioSource: AudioSource }
export interface CaptureStatus extends CaptureStatusFields { sources?: CaptureSourceStatus[] }
export type CaptureLifecycle = Pick<SessionStateEvent, "status" | "isActive" | "isPaused">;

const copy = {
  en: SUPPLEMENTAL_EN.captureStatus_copy,
  zh: {
    title: "音频输入", system: "系统声音", microphone: "麦克风", output: "系统输出", device: "采集设备",
    idle: "尚未采音", paused: "已暂停", connecting: "连接中", stopping: "停止中", error: "出错",
    unobserved: "等待音频", sound: "收到声音", silent: "暂无声音", noData: "未收到音频",
    systemHelp: "采集应用播放的系统声音。", microphoneHelp: "使用系统默认麦克风。",
    silentHelp: "已收到音频数据，但暂未检测到明显声音。",
    systemNoData: "请播放声音，并检查应用的音频输出。", microphoneNoData: "请检查系统默认麦克风及其输入音量。",
    errorHelp: "请查看字幕窗口中的错误，然后重试。",
    autoOutput: "跟随系统默认输出。", manualOutput: "采集手动选择的输出。", linuxOutput: "使用开始字幕时选定的输出监听。",
    off: "已关闭", minimum: "至少保留一路开启。要更换来源，请先开启另一路。",
    switchFailed: "音频输入未能切换，请查看字幕状态后重试。",
    switchHelp: "切换后录音关闭，暂停状态保留。",
    dualHelp: "双路分别识别，用量分别计算，同时开启时自动降低外放回声。仍有重复收音时可佩戴耳机。",
  },
  ja: {
    title: "音声入力", system: "システム音声", microphone: "マイク", output: "システム出力", device: "取得デバイス",
    idle: "取得前", paused: "一時停止中", connecting: "接続中", stopping: "終了中", error: "エラー",
    unobserved: "音声待ち", sound: "音声を受信中", silent: "音声なし", noData: "データなし",
    systemHelp: "アプリが再生するシステム音声を取得します。", microphoneHelp: "システムの既定のマイクを使用します。",
    silentHelp: "音声データを受信していますが、明確な音は検出されていません。",
    systemNoData: "音を再生し、アプリの音声出力を確認してください。", microphoneNoData: "システムの既定のマイクと入力音量を確認してください。",
    errorHelp: "字幕ウィンドウのエラーを確認し、再試行してください。",
    autoOutput: "システムの既定の出力に合わせます。", manualOutput: "選択した出力を取得します。", linuxOutput: "字幕の開始時に選択した出力モニターを使用します。",
    off: "オフ", minimum: "入力を一つ以上オンにしてください。切り替えるには、先にもう一方をオンにします。",
    switchFailed: "音声入力を切り替えられませんでした。字幕の状態を確認して再試行してください。",
    switchHelp: "切替で録音はオフになります。一時停止は維持します。",
    dualHelp: "両方を個別に認識し、利用量も個別に発生します。同時にオンのときは自動でエコーを低減します。スピーカーの音が残る場合は、ヘッドホンを使用してください。",
  },
  ...supplemental.captureStatus_copy,
};

/** Legacy single-source snapshots are safe to reuse; an aggregate is never
 * evidence that each selected input has delivered data or sound. */
export function captureStatusForSource(value: CaptureStatus | null, input: AudioInput, source: AudioSource): CaptureStatusFields | null {
  if (value?.sources) return value.sources.find(item => item.audioSource === source) ?? null;
  if (input === "both" || value?.kind === "both") return null;
  if (!value || (value.kind === "microphone") !== (source === "microphone")) return null;
  return value;
}

export function capturePresentation(value: CaptureStatusFields | null, lifecycle: CaptureLifecycle,
  language = effectiveUiLanguage(), input: AudioSource = "system", enabled = true) {
  const text = copy[language];
  const microphone = input === "microphone";
  const source = microphone ? text.microphone : text.system;
  const kind = lifecycle.status.kind;
  const observation = !enabled ? text.off : kind === "error" ? text.error : kind === "stopping" ? text.stopping
    : kind === "connecting" ? text.connecting : lifecycle.isPaused ? text.paused
    : !lifecycle.isActive || kind === "idle" ? text.idle : !value?.observation ? text.unobserved
    : value.observation.soundRecent && value.observation.pcmDataRecent ? text.sound
    : value.observation.pcmDataRecent ? text.silent : text.noData;
  const device = !enabled ? null : microphone ? value?.actualDeviceName : value?.systemOutputDeviceName ?? value?.actualDeviceName;
  const route = value?.kind === "application" ? applicationAudioCopy(language).help : microphone ? text.microphoneHelp : value?.kind === "windows_output"
    ? value.strategy === "follow_system" ? text.autoOutput : text.manualOutput
    : value?.kind === "linux_output_monitor" ? text.linuxOutput : text.systemHelp;
  const detail = !enabled ? null : kind === "error" ? text.errorHelp : kind === "listening" && lifecycle.isActive && !lifecycle.isPaused
    ? observation === text.noData ? microphone ? text.microphoneNoData : text.systemNoData
      : observation === text.silent ? text.silentHelp : null
    : null;
  const devicePrefix = value?.kind === "application" ? applicationAudioCopy(language).application : microphone || !value?.systemOutputDeviceName ? text.device : text.output;
  const deviceCharacters = Array.from(device ?? "");
  const shortDevice = deviceCharacters.length > 40 ? `${deviceCharacters.slice(0, 39).join("")}…` : device;
  const deviceDescription = device ? `${devicePrefix}: ${device}` : undefined;
  const help = [route, shortDevice ? `${devicePrefix}: ${shortDevice}` : null, detail]
    .filter(Boolean).join("\n");
  return { title: text.title, source, observation, help, deviceDescription };
}

export function captureSwitchCopy(language = effectiveUiLanguage()) {
  const { minimum, switchHelp, dualHelp, switchFailed } = copy[language];
  return { minimum, switchHelp, dualHelp, switchFailed };
}
