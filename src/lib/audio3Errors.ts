import { SUPPLEMENTAL_EN } from "./locales/supplemental-schema";
import { supplemental } from "./locales/supplemental";
import { effectiveUiLanguage } from "./i18n";

const copy = {
  en: SUPPLEMENTAL_EN.audio3Errors_copy,
  zh: {
    summary: {
      timeout: "语音识别服务超时。",
      authentication: "语音识别服务拒绝访问。",
      local_overload: "本地语音识别未能跟上。",
      local_timeout: "本地语音识别超时。",
      unsupported_language: "语音识别服务不支持所选源语言。",
      request: "语音识别服务拒绝了请求。",
      service: "语音识别服务出错。",
      rate_limit: "语音识别请求受限。",
      task_failed: "语音识别意外中断。",
    },
    setup: "启动语音识别时：", recognition: "识别声音时：", connection: "连接服务时：",
    timeout: "服务超时。请检查声音状态和网络连接，再重新开启字幕。",
    authentication: "服务拒绝了访问。请在设置中检查 API 密钥和访问权限。",
    local_overload: "本地语音识别未能跟上。请停止其他本地模型或选择更小的模型后重试。",
    local_timeout: "本地语音识别超时。请等待本地服务就绪后重试；若反复发生，可换用更小的模型。",
    unsupported_language: "当前语音识别服务不支持所选源语言。请更换源语言或识别服务。",
    request: "服务拒绝了此次请求。请检查服务设置后重试。",
    service: "服务报告了错误，请稍后重试。",
    rate_limit: "服务正在限制请求，请稍等再重新开启字幕。",
    task_failed: "服务结束了任务，原因尚不明确。请检查声音状态后重试。",
  },
  ja: {
    summary: {
      timeout: "音声認識がタイムアウトしました。",
      authentication: "音声認識へのアクセスが拒否されました。",
      local_overload: "ローカル音声認識の処理が追いつきません。",
      local_timeout: "ローカル音声認識がタイムアウトしました。",
      unsupported_language: "音声認識サービスがこの入力言語に対応していません。",
      request: "音声認識サービスがリクエストを拒否しました。",
      service: "音声認識サービスでエラーが発生しました。",
      rate_limit: "音声認識のリクエストが制限されています。",
      task_failed: "音声認識が予期せず停止しました。",
    },
    setup: "音声認識の開始時：", recognition: "音声認識中：", connection: "サービスへの接続時：",
    timeout: "サービスがタイムアウトしました。音声の状態とネットワークを確認して、字幕を再開してください。",
    authentication: "サービスがアクセスを拒否しました。設定で API キーとアクセス権限を確認してください。",
    local_overload: "ローカル音声認識の処理が追いつきません。他のローカルモデルを停止するか、より小さいモデルを選んで再試行してください。",
    local_timeout: "ローカル音声認識がタイムアウトしました。サービスの準備ができてから再試行し、繰り返す場合は小さいモデルをお試しください。",
    unsupported_language: "この音声認識サービスは選択した入力言語に対応していません。入力言語または音声認識サービスを変更してください。",
    request: "サービスがリクエストを拒否しました。サービスの設定を確認して再試行してください。",
    service: "サービスがエラーを報告しました。後で再試行してください。",
    rate_limit: "サービスがリクエストを制限しています。少し待ってから字幕を再開してください。",
    task_failed: "サービスがタスクを終了しましたが、原因はまだ不明です。音声の状態を確認して再試行してください。",
  },
  ...supplemental.audio3Errors_copy,
};

const audio3ErrorPattern = /^audio3_error\.(setup|recognition|connection)\.(timeout|authentication|request|service|rate_limit|task_failed|unsupported_language|local_overload|local_timeout)\.(CLIENT_ERROR|SERVER_ERROR|INVALID_API_KEY|UNAUTHORIZED|REQUEST_TIMEOUT|THROTTLED|OTHER|LOCAL_TIMEOUT|HTTP_AUTH|UNSUPPORTED_LANGUAGE|LOCAL_ASR_OVERLOADED|LOCAL_ASR_TIMEOUT)$/;

export function audio3ErrorMessage(message: string): string | null {
  const match = audio3ErrorPattern.exec(message);
  if (!match) return null;
  const text = copy[effectiveUiLanguage()];
  return text[match[1] as "setup" | "recognition" | "connection"] + text[match[2] as "timeout" | "authentication" | "request" | "service" | "rate_limit" | "task_failed" | "unsupported_language" | "local_overload" | "local_timeout"];
}

/** The compact overlay and detailed panel share the same allowlisted cause. */
export function audio3ErrorSummary(message: string): string | null {
  const match = audio3ErrorPattern.exec(message);
  return match ? copy[effectiveUiLanguage()].summary[match[2] as keyof typeof copy.en.summary] : null;
}

/** Exact sanitized setup errors need a configuration change, not blind retry. */
export function audio3ErrorRequiresConfiguration(message: string): boolean {
  return message === "audio3_error.setup.request.CLIENT_ERROR" || /^audio3_error\.(setup|recognition|connection)\.(unsupported_language\.UNSUPPORTED_LANGUAGE|authentication\.(INVALID_API_KEY|UNAUTHORIZED|HTTP_AUTH))$/.test(message);
}
