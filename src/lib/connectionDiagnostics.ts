import { SUPPLEMENTAL_EN } from "./locales/supplemental-schema";
import { supplemental } from "./locales/supplemental";
import { audio3ErrorMessage, audio3ErrorRequiresConfiguration } from "./audio3Errors";
import { effectiveUiLanguage, I18N } from "./i18n";
import type { ConnectionDiagnostic, SettingsNavigationTarget } from "./ipc";

export type DiagnosticPlatform = "macos" | "windows" | "linux";
const copy = {
  en: SUPPLEMENTAL_EN.connectionDiagnostics_copy,
  zh: {
    localDevReadOnly: "开发预设从本机 .env 读取对应服务的密钥。修改密钥后需重新打开 mimi dev；其他配置使用本机私有文件，可正常编辑。",
    localDevTranslationLocked: "此开发预设使用内置服务进行识别与翻译。",
    localDevTranslationHelp: "添加普通配置即可选择其他服务或独立文字翻译。普通配置的凭据保存在开发版自己的私有文件中，预设密钥仍由只读 .env 提供。",
    localDevUnavailable: "无法读取本机开发 .env。请检查格式、文件归属及 0600 权限，然后重新打开 mimi dev。",
    storage: "无法读取服务凭据。请检查本地文件访问权限或重新保存。",
    linuxStorage: "无法访问桌面密码存储。请确认当前桌面会话已安装并启用 Secret Service 服务（例如 GNOME Keyring）。解锁密码存储或允许系统授权提示后，重新检查。",
    serviceUnavailable: "Linux Secret Service 服务不可用。请在当前桌面会话安装或启用兼容服务，例如 GNOME Keyring。若桌面配置要求，请注销并重新登录后再检查。",
    accessDenied: "系统凭据存储已锁定或访问被拒绝。解锁密码存储或允许系统授权提示后，重新检查。",
    missing: "尚未配置凭据。请先保存服务配置。",
    invalid: "无法读取已保存的凭据，请重新保存。",
    auth: "服务拒绝了认证。请检查密钥和账号权限，再更新凭据。",
    quota: "服务限额已触发，请稍后重试或检查额度。",
    translationLimited: "服务仍在限流，请稍后重试。",
    translationTemporary: "翻译暂时不可用，请重新连接。",
    translationBacklog: "翻译未能跟上，请重新连接。",
    translationSourceUnsupported: "当前翻译模型不支持检测到的语言。",
    subtitleTooLarge: "服务返回的字幕过长，请重新连接。",
    timeout: "连接超时。检查网络或代理后重试。",
    unreachable: "安全连接失败。请检查网络、代理和系统时间。",
    speechUnreachable: "无法连接语音识别服务。请检查服务地址、网络和代理；使用本地服务时，请确认服务已启动。",
    speechTimeout: "语音识别连接超时。请确认服务已就绪，并检查服务地址、网络和代理后重试。",
    test: "检查连接", testing: "正在检查…",
    available: "连接可用", unavailable: "不可用", notTested: "尚未检查",
    checkSkipped: "界面预览模式不检查连接，请在正常模式下检查。",
    elapsed: "检查耗时",
    elapsedHelp: "识别检查测量会话建立耗时；翻译检查测量固定短句请求耗时，不代表实时字幕延迟。",
    checkFailed: "检查失败，请重试。",
    macosRecovery: "解锁钥匙串或允许访问后重试。",
    windowsRecovery: "检查凭据管理器的访问权限后重试。",
    linuxRecovery: "解锁密码存储或允许访问后重试。",
    reasons: {
      credentialsMissing: "请先保存凭据。",
      textTranslationNotConfigured: "请先选择并保存文字翻译服务。",
      credentialsUnavailable: "请检查本地文件访问权限或重新保存凭据。",
      credentialsServiceUnavailable: "启用 Secret Service（如 GNOME Keyring）后重试。",
      credentialsAccessDenied: "解锁凭据存储或允许访问。",
      invalidConfiguration: "请检查服务配置。",
      unsupportedLanguage: "当前语音服务不支持所选源语言。请更换源语言或语音服务。",
      authenticationRejected: "请检查凭据和账号权限。",
      serviceRejected: "服务拒绝了请求。",
      serviceNotActivated: "请在腾讯云 ASR 控制台开通实时语音翻译服务。",
      quotaExhausted: "服务额度或余额不足，请检查腾讯云 ASR 的计费和用量。",
      concurrencyLimited: "并发会话已达上限，请停止其他会话后重试。",
      localRecognitionOverloaded: "本地识别积压。请停止其他本地模型或选择更小的模型后重试。",
      localRecognitionTimeout: "本地识别超时。请等待服务就绪后重试。",
      timeout: "连接超时。",
      unreachable: "无法连接服务。",
    },
  },
  ja: {
    localDevReadOnly: "開発プリセットはローカルの .env から各サービスのキーを読み込みます。キーを変更したら mimi dev を再起動してください。他の設定は専用ローカルファイルを使用し、通常どおり編集できます。",
    localDevTranslationLocked: "この開発プリセットは組み込みサービスで認識と翻訳を行います。",
    localDevTranslationHelp: "通常の設定を追加すると、別のサービスや独立したテキスト翻訳を選べます。認証情報は開発アプリ専用のローカルファイルに保存され、プリセットのキーは読み取り専用の .env に残ります。",
    localDevUnavailable: "開発用 .env を読めません。形式、所有者、0600 権限を確認し、mimi dev を再起動してください。",
    storage: "サービスの認証情報を読めません。ローカルファイルへのアクセスを確認するか、再保存してください。",
    linuxStorage: "デスクトップのパスワードストアにアクセスできません。現在のデスクトップセッションで GNOME Keyring などの Secret Service がインストールされ、有効になっていることを確認してください。ストアのロックを解除するかシステムのアクセス許可を承認して、再確認してください。",
    serviceUnavailable: "Linux Secret Service を利用できません。現在のデスクトップセッションで GNOME Keyring などの対応サービスをインストールするか有効にしてください。デスクトップの設定に応じてログアウトして再ログインし、再確認してください。",
    accessDenied: "システムの認証情報ストアがロックされているか、アクセスが拒否されました。ロックを解除するかシステムのアクセス許可を承認して、再確認してください。",
    missing: "認証情報が未設定です。まずサービス設定を保存してください。",
    invalid: "保存された認証情報を読めません。もう一度保存してください。",
    auth: "認証が拒否されました。キーとアカウントの権限を確認し、認証情報を更新してください。",
    quota: "サービスの上限に達しました。しばらくして再試行するか、利用枠を確認してください。",
    translationLimited: "サービスの制限が続いています。しばらくして再試行してください。",
    translationTemporary: "翻訳を一時的に利用できません。再接続してください。",
    translationBacklog: "翻訳が遅れています。再接続してください。",
    translationSourceUnsupported: "現在の翻訳モデルは検出された言語に対応していません。",
    subtitleTooLarge: "サービスの字幕が長すぎます。再接続してください。",
    timeout: "接続がタイムアウトしました。ネットワークやプロキシを確認してください。",
    unreachable: "安全な接続に失敗しました。ネットワーク、プロキシ、システム時刻を確認してください。",
    speechUnreachable: "音声認識サービスに接続できません。サービスの URL、ネットワーク、プロキシを確認してください。ローカルサービスの場合は起動を確認してください。",
    speechTimeout: "音声認識の接続がタイムアウトしました。サービスの準備ができていることと、URL、ネットワーク、プロキシを確認して再試行してください。",
    test: "接続を確認", testing: "確認中…",
    available: "接続可能", unavailable: "利用不可", notTested: "未確認",
    checkSkipped: "UI プレビューモードでは接続を確認できません。通常モードで確認してください。",
    elapsed: "確認時間",
    elapsedHelp: "音声認識はセッション開始、翻訳は短文リクエストの所要時間です。字幕のリアルタイム遅延ではありません。",
    checkFailed: "確認に失敗しました。もう一度お試しください。",
    macosRecovery: "キーチェーンのロック解除かアクセス許可後、再確認してください。",
    windowsRecovery: "資格情報マネージャーのアクセスを確認し、再試行してください。",
    linuxRecovery: "パスワードストアのロック解除かアクセス許可後、再確認してください。",
    reasons: {
      credentialsMissing: "認証情報を保存してください。",
      textTranslationNotConfigured: "文字翻訳サービスを選択して保存してください。",
      credentialsUnavailable: "ローカルファイルへのアクセスを確認するか、認証情報を再保存してください。",
      credentialsServiceUnavailable: "GNOME Keyring などの Secret Service を有効にし、再確認してください。",
      credentialsAccessDenied: "認証情報ストアのロックを解除するかアクセスを許可してください。",
      invalidConfiguration: "サービス設定を確認してください。",
      unsupportedLanguage: "選択した入力言語に対応していません。入力言語または音声認識サービスを変更してください。",
      authenticationRejected: "認証情報とアカウントの権限を確認してください。",
      serviceRejected: "サービスがリクエストを拒否しました。",
      serviceNotActivated: "Tencent Cloud ASR コンソールでリアルタイム音声翻訳を有効にしてください。",
      quotaExhausted: "サービスの利用枠または残高が不足しています。Tencent Cloud ASR の料金と使用量を確認してください。",
      concurrencyLimited: "同時セッション数の上限です。他のセッションを停止して再試行してください。",
      localRecognitionOverloaded: "ローカル認識の処理が追いつきません。他のモデルを停止するか、小さいモデルで再試行してください。",
      localRecognitionTimeout: "ローカル認識がタイムアウトしました。サービスの準備ができてから再試行してください。",
      timeout: "接続がタイムアウトしました。",
      unreachable: "サービスに接続できません。",
    },
  },
  ...supplemental.connectionDiagnostics_copy,
};
export function diagnosticPlatform(userAgent = typeof navigator === "undefined" ? "" : navigator.userAgent): DiagnosticPlatform {
  return /Mac/.test(userAgent) ? "macos" : /Windows/.test(userAgent) ? "windows" : "linux";
}
export function diagnosticCopy(platform: DiagnosticPlatform = diagnosticPlatform()) {
  const labels = copy[effectiveUiLanguage()];
  const recovery = labels[`${platform}Recovery`];
  return {
    ...labels,
    storage: labels.storage,
    reasons: { ...labels.reasons, credentialsAccessDenied: recovery,
      appleSpeechAssetsMissing: I18N.settings.appleSpeechAssetsMissing,
      appleSpeechPreparing: I18N.settings.appleSpeechPreparationInProgress,
      appleSpeechStatusFailed: I18N.settings.appleSpeechLoadFailed,
      appleSpeechLanguageUnsupported: I18N.settings.appleSpeechLanguageUnsupported,
      appleSpeechUnavailable: I18N.settings.appleSpeechUnavailable,
      appleSpeechRecognitionFailed: I18N.settings.appleSpeechRecognitionFailed,
      appleTranslationAssetsMissing: I18N.settings.appleTranslationAssetsMissing,
      appleTranslationLanguageUnsupported: I18N.settings.appleTranslationUnsupported,
      appleTranslationUnavailable: I18N.settings.appleTranslationUnavailable,
      appleTranslationFailed: I18N.settings.appleTranslationFailed,
      localDevCredentialsUnavailable: labels.localDevUnavailable },
  };
}
export function credentialUnavailableHelp(platform: DiagnosticPlatform = diagnosticPlatform()): string {
  return diagnosticCopy(platform).storage;
}
export function connectionDiagnosticMessage(result: ConnectionDiagnostic, platform?: DiagnosticPlatform): string {
  const labels = diagnosticCopy(platform);
  if (result.service === "available") return labels.available;
  const credentialReason = {
    missing: "credentialsMissing", unavailable: "credentialsUnavailable", localDevUnavailable: "localDevCredentialsUnavailable",
    serviceUnavailable: "credentialsServiceUnavailable", accessDenied: "credentialsAccessDenied", invalid: "invalidConfiguration",
  } as const;
  const reasonCode = result.reason ?? (result.credential === "present" ? null : credentialReason[result.credential]);
  const reason = reasonCode ? labels.reasons[reasonCode] : null;
  if (reason) return `${labels.unavailable}: ${reason}`;
  if (result.service === "notTested") return labels.checkSkipped;
  return result.service === "unavailable" ? labels.unavailable : labels.checkFailed;
}
const deepLXErrors = {
  zh: {
    timeout: "DeepLX 响应超时。请检查文字翻译地址、网络和服务器，再重新启动字幕。",
    connection: "无法连接 DeepLX。请检查文字翻译地址、网络和服务器，再重新启动字幕。",
    response: "DeepLX 返回无效或空翻译。请确认文字翻译地址支持 DeepLX /translate JSON 接口。",
    size: "DeepLX 返回数据过大。请检查服务器的 /translate 响应。",
    rejected: "DeepLX 拒绝请求。请向服务器管理员确认文字翻译地址和可选 Bearer token。",
  },
  en: SUPPLEMENTAL_EN.connectionDiagnostics_deepLXErrors,
  ja: {
    timeout: "DeepLX がタイムアウトしました。文字翻訳 URL、ネットワーク、サーバーを確認し、字幕を再開してください。",
    connection: "DeepLX に接続できません。文字翻訳 URL、ネットワーク、サーバーを確認し、字幕を再開してください。",
    response: "DeepLX の翻訳が無効または空です。文字翻訳 URL が DeepLX /translate JSON API に対応しているか確認してください。",
    size: "DeepLX の応答が大きすぎます。サーバーの /translate 応答を確認してください。",
    rejected: "DeepLX がリクエストを拒否しました。文字翻訳 URL と任意の Bearer token をサーバー管理者に確認してください。",
  },
  ...supplemental.connectionDiagnostics_deepLXErrors,
};
const openAICompatibleErrors = {
  zh: {
    timeout: "第三方翻译服务响应超时。请检查服务和网络，再重新启动字幕。",
    connection: "无法连接第三方翻译服务。请检查服务地址和网络，再重新启动字幕。",
    response: "第三方翻译服务返回无效或空文本。请确认支持非流式 Chat Completions，并返回 choices[0].message.content。",
    size: "第三方翻译服务返回数据过大。请检查服务器的响应。",
    rejected: "第三方翻译服务拒绝请求。请检查 API Key、模型名称和服务地址。",
  },
  en: SUPPLEMENTAL_EN.connectionDiagnostics_openAICompatibleErrors,
  ja: {
    timeout: "外部翻訳サービスがタイムアウトしました。サービスとネットワークを確認し、字幕を再開してください。",
    connection: "外部翻訳サービスに接続できません。URL とネットワークを確認し、字幕を再開してください。",
    response: "外部翻訳サービスのテキストが無効または空です。非ストリーミング Chat Completions と choices[0].message.content の応答に対応しているか確認してください。",
    size: "外部翻訳サービスの応答が大きすぎます。サーバーの応答を確認してください。",
    rejected: "外部翻訳サービスがリクエストを拒否しました。API Key、モデル名、サービス URL を確認してください。",
  },
  ...supplemental.connectionDiagnostics_openAICompatibleErrors,
};
// These are exact, content-free labels emitted by the built-in clients. Keep
// provider response bodies and native transport details outside this allowlist.
const builtinServiceErrors = {
  unreachable: new Set([
    "The live translation transport failed.",
    "The live translation session is not connected.",
    "The live translation connection closed.",
    "The OpenAI Realtime Translation connection failed.",
    "The OpenAI Realtime Translation session is not connected.",
    "The Gemini Live Translation connection failed.",
    "The Gemini Live Translation session is not connected.",
    "The Azure OpenAI Realtime Translation connection failed.",
    "The Azure OpenAI Realtime Translation session is not connected.",
    "The xAI Grok Voice connection failed.",
    "The xAI Grok Voice session is not connected.",
    "The Tencent Cloud realtime translation connection failed.",
    "The Tencent Cloud realtime translation session is not connected.",
    "The Baidu realtime translation connection failed.",
    "The Baidu realtime translation session is not connected.",
    "The Volcano Engine connection failed.",
    "The Volcano Engine transport failed.",
    "The Volcano Engine translation session is not connected.",
  ]),
  timeout: new Set([
    "The live translation connection could not be established in time.",
    "The live translation connection stopped responding.",
    "The live translation session setup timed out.",
    "The OpenAI Realtime Translation connection stopped responding.",
    "OpenAI Realtime Translation did not confirm the session configuration in time.",
    "The Gemini Live Translation connection stopped responding.",
    "Gemini Live Translation did not confirm the session configuration in time.",
    "The Azure OpenAI Realtime Translation connection stopped responding.",
    "Azure OpenAI Realtime Translation did not confirm the session configuration in time.",
    "The xAI Grok Voice connection stopped responding.",
    "xAI Grok Voice did not confirm the session configuration in time.",
    "xAI Grok Voice did not finish the final turn in time.",
    "The Tencent Cloud realtime translation connection stopped responding.",
    "Tencent Cloud realtime translation did not confirm the session in time.",
    "The Baidu realtime translation connection stopped responding.",
    "Baidu realtime translation did not confirm the session in time.",
    "The Volcano Engine connection stopped responding.",
    "The Volcano Engine connection could not be established in time.",
    "Volcano Engine did not confirm the session configuration in time.",
  ]),
  rejected: new Set([
    "The live translation session setup was rejected.",
    "OpenAI Realtime Translation rejected the session configuration.",
    "OpenAI Realtime Translation rejected the session.",
    "Gemini Live Translation rejected the session configuration.",
    "Gemini Live Translation rejected the session.",
    "Azure OpenAI Realtime Translation rejected the session configuration.",
    "Azure OpenAI Realtime Translation rejected the session.",
    "xAI Grok Voice rejected the session configuration.",
    "xAI Grok Voice rejected the session.",
    "Tencent Cloud realtime translation rejected the session configuration.",
    "Tencent Cloud realtime translation rejected the session.",
    "Baidu realtime translation rejected the session configuration.",
    "Baidu realtime translation rejected the session.",
    "Volcano Engine rejected the session configuration.",
    "Volcano Engine rejected the translation session.",
  ]),
  missing: new Set([
    "Add an Alibaba Cloud Model Studio API key in Settings.",
    "Add an OpenAI API key in Settings.",
    "Add a Google Gemini API key in Settings.",
    "Add an Azure OpenAI API key in Settings.",
    "Add an xAI API key in Settings.",
    "Add a Tencent Cloud AppID, SecretID, and SecretKey in Settings.",
    "Add a Baidu Cloud AppID and AppKey in Settings.",
    "Add a Volcano Engine API key in Settings.",
  ]),
  invalidConfiguration: new Set([
    "Enter a valid Azure OpenAI resource endpoint in Settings.",
    "Enter Azure OpenAI translation and transcription deployment names in Settings.",
    "The Azure OpenAI endpoint must be an official HTTPS resource endpoint.",
    "The Azure OpenAI deployment name is invalid.",
  ]),
  unsupportedLanguage: new Set([
    "OpenAI Realtime Translation requires a translated output language.",
    "Gemini Live Translation requires a translated output language.",
    "Azure OpenAI Realtime Translation requires a translated output language.",
    "xAI Grok Voice requires a translated output language.",
    "Tencent Cloud realtime translation requires a translated output language.",
    "Baidu realtime translation requires an explicit supported source language.",
    "Baidu realtime translation requires a translated output language.",
    "Volcano Engine requires an explicit Chinese, English, or Japanese source language.",
    "Volcano Engine requires a Chinese, English, or Japanese translation language.",
  ]),
  interrupted: new Set([
    "The live translation service returned invalid data.",
    "OpenAI Realtime Translation returned an invalid response.",
    "Gemini Live Translation returned an invalid response.",
    "Azure OpenAI Realtime Translation returned an invalid response.",
    "xAI Grok Voice returned an invalid response.",
    "xAI Grok Voice did not complete the current turn.",
    "Tencent Cloud realtime translation returned an invalid response.",
    "Tencent Cloud realtime translation ended the session unexpectedly.",
    "Baidu realtime translation returned an invalid response.",
    "Baidu realtime translation ended the session unexpectedly.",
    "Volcano Engine returned an invalid response.",
    "Volcano Engine ended the translation session unexpectedly.",
  ]),
};

const tencentErrorReasons = {
  tencent_configuration_rejected: "invalidConfiguration",
  tencent_service_activation_required: "serviceNotActivated",
  tencent_quota_exhausted: "quotaExhausted",
  tencent_capacity_exceeded: "concurrencyLimited",
  tencent_provider_rejected: "serviceRejected",
} as const;

function builtinServiceErrorMessage(error: string): string | null {
  const labels = diagnosticCopy();
  if (Object.hasOwn(tencentErrorReasons, error)) return labels.reasons[tencentErrorReasons[error as keyof typeof tencentErrorReasons]];
  if (builtinServiceErrors.unreachable.has(error)) return labels.unreachable;
  if (builtinServiceErrors.timeout.has(error)) return labels.timeout;
  if (builtinServiceErrors.missing.has(error)) return labels.missing;
  if (builtinServiceErrors.invalidConfiguration.has(error)) return labels.reasons.invalidConfiguration;
  if (builtinServiceErrors.unsupportedLanguage.has(error)) return I18N.settings.languageSwitchUnsupported;
  if (builtinServiceErrors.interrupted.has(error)) return labels.translationTemporary;
  const rejected = `${labels.reasons.serviceRejected} ${labels.reasons.invalidConfiguration}`;
  if (builtinServiceErrors.rejected.has(error)) return rejected;
  // Baidu's setup error contains only an i64 code. Accept the complete canonical
  // Rust representation, never a provider body, URL, or trailing newline.
  const baiduCode = /^Baidu realtime translation rejected the session configuration \(code (0|-?[1-9]\d{0,18})\)\.$/.exec(error);
  if (baiduCode?.[0] === error) {
    const code = BigInt(baiduCode[1]);
    if (code >= -9223372036854775808n && code <= 9223372036854775807n) return `${rejected} (${baiduCode[1]})`;
  }
  return null;
}

/** Match only sanitized backend labels; never interpolate arbitrary native errors. */
export function credentialErrorMessage(error: unknown, platform?: DiagnosticPlatform): string | null {
  if (typeof error !== "string") return null;
  if (error === "apple_translation_assets_missing") return I18N.settings.appleTranslationAssetsMissing;
  if (error === "apple_translation_source_required") return I18N.settings.appleTranslationChooseSource;
  if (error === "apple_translation_language_unsupported") return I18N.settings.appleTranslationUnsupported;
  if (error === "apple_translation_unavailable" || error === "apple_translation_ui_test_unavailable") return I18N.settings.appleTranslationUnavailable;
  if (error === "apple_translation_status_failed") return I18N.settings.appleTranslationStatusFailed;
  if (error === "apple_translation_prepare_failed") return I18N.settings.appleTranslationPrepareFailed;
  if (error === "apple_translation_cancelled") return I18N.settings.appleTranslationCancelled;
  if (error === "apple_translation_busy") return I18N.settings.appleTranslationBusy;
  if (["apple_translation_timeout", "apple_translation_failed", "apple_translation_invalid_input", "apple_translation_invalid_result"].includes(error)) return I18N.settings.appleTranslationFailed;
  if (error === "apple_speech_assets_missing") return I18N.settings.appleSpeechAssetsMissing;
  if (error === "apple_speech_language_unsupported") return I18N.settings.appleSpeechLanguageUnsupported;
  if (error === "apple_speech_translation_language_unsupported") return I18N.settings.appleSpeechTranslationLanguageUnsupported;
  if (error === "apple_speech_unavailable" || error === "apple_speech_ui_test_unavailable") return I18N.settings.appleSpeechUnavailable;
  if (error === "apple_speech_status_failed") return I18N.settings.appleSpeechLoadFailed;
  if (error === "apple_speech_prepare_failed") return I18N.settings.appleSpeechPrepareFailed;
  if (error === "apple_speech_service_unavailable") return I18N.settings.appleSpeechServiceUnavailable;
  if (error === "apple_speech_reservation_limit") return I18N.settings.appleSpeechReservationLimit;
  if (error === "apple_speech_download_cancelled") return I18N.settings.appleSpeechDownloadCancelled;
  if (error === "apple_speech_download_timeout") return I18N.settings.appleSpeechDownloadTimeout;
  if (error === "apple_speech_download_network") return I18N.settings.appleSpeechDownloadNetwork;
  if (error === "apple_speech_download_storage") return I18N.settings.appleSpeechDownloadStorage;
  if (error === "apple_speech_resources_unavailable") return I18N.settings.appleSpeechResourcesUnavailable;
  if (error === "apple_speech_preparing") return I18N.settings.appleSpeechPreparationInProgress;
  if (["apple_speech_setup_timeout", "apple_speech_start_failed", "apple_speech_recognition_failed", "apple_speech_audio_failed", "apple_speech_not_connected", "apple_speech_result_backlog", "apple_speech_invalid_result", "apple_speech_finalize_timeout"].includes(error)) return I18N.settings.appleSpeechRecognitionFailed;
  if (error === "custom_speech_endpoint_invalid") return I18N.settings.customSpeechEndpointInvalid;
  if (error === "custom_speech_model_invalid") return I18N.settings.customSpeechModelInvalid;
  if (error === "custom_speech_session_rejected") return I18N.settings.customSpeechRejected;
  if (error === "custom_speech_unreachable") return I18N.settings.customSpeechUnreachable;
  if (error === "custom_speech_protocol_invalid") return I18N.settings.customSpeechProtocolInvalid;
  if (error === "custom_speech_credentials_missing") return diagnosticCopy().missing;
  if (error === "custom_speech_authentication_failed") return diagnosticCopy().auth;
  if (error === "custom_speech_setup_timeout" || error === "custom_speech_health_timeout") return diagnosticCopy().timeout;
  if (error === "custom_speech_not_connected") return I18N.settings.customSpeechUnreachable;
  if (error === "custom_speech_audio_invalid") return I18N.settings.customSpeechProtocolInvalid;
  if (error === "text_translation_credentials_missing") return diagnosticCopy().missing;
  if (error === "translation_rate_limited") return diagnosticCopy().translationLimited;
  if (error === "translation_temporarily_unavailable") return diagnosticCopy().translationTemporary;
  if (error === "translation_source_unsupported") return diagnosticCopy().translationSourceUnsupported;
  if (error === "subtitle_text_too_large" || error === "The subtitle service returned too much text.") return diagnosticCopy().subtitleTooLarge;
  if (error === "Translation fell behind live audio. mimi is reconnecting.") return diagnosticCopy().translationBacklog;
  if (error.startsWith("You exceeded your current quota, please check your plan and billing details.")) return diagnosticCopy().quota;
  const dlx = deepLXErrors[effectiveUiLanguage()];
  if (error.startsWith("DeepLX timed out.")) return dlx.timeout;
  if (error.startsWith("Could not connect to DeepLX.")) return dlx.connection;
  if (error.startsWith("DeepLX returned an invalid or empty translation.")) return dlx.response;
  if (error.startsWith("DeepLX returned too much data.")) return dlx.size;
  const rejected = /^DeepLX rejected the request \(code (\d{1,3})\)\./.exec(error);
  if (rejected) return `${dlx.rejected} (${rejected[1]})`;

  const compatible = openAICompatibleErrors[effectiveUiLanguage()];
  if (error === "The OpenAI-compatible endpoint is invalid." || error.startsWith("Use an HTTPS OpenAI-compatible endpoint") || error.startsWith("Check the custom translation service address.")) return I18N.settings.deepLXEndpointInvalid;
  if (error === "The OpenAI-compatible model name is invalid." || error.startsWith("Enter a model name without control characters")) return I18N.settings.openAICompatibleModelInvalid;
  if (error.startsWith("Enter a valid API key for the custom translation service.")) return I18N.settings.openAICompatibleApiKeyInvalid;
  if (error.startsWith("The selected language is not supported by this custom translation route.")) return diagnosticCopy().translationSourceUnsupported;
  if (error.startsWith("The custom translation service timed out.")) return compatible.timeout;
  if (error.startsWith("Could not connect to the custom translation service.")) return compatible.connection;
  if (error.startsWith("The custom translation service returned invalid or empty text.")) return compatible.response;
  if (error.startsWith("The custom translation service returned too much data.")) return compatible.size;
  const compatibleRejected = /^The custom translation service rejected the request \(HTTP (\d{1,3})\)\./.exec(error);
  if (compatibleRejected) return `${compatible.rejected} (HTTP ${compatibleRejected[1]})`;

  if (error === "local_dev_credentials_read_only") return diagnosticCopy(platform).localDevReadOnly;
  if (error === "local_dev_credentials_unavailable") return diagnosticCopy(platform).localDevUnavailable;
  if (error === "credential_service_unavailable") return diagnosticCopy(platform).serviceUnavailable;
  if (error === "credential_store_access_denied") return diagnosticCopy(platform).accessDenied;
  if (error === "credential_store_unavailable" || error === "The system credential store is unavailable.") return diagnosticCopy(platform).storage;
  // Audio3 uses these fixed labels for both built-in and custom speech routes.
  // Match exactly: native transport details or provider bodies are never safe copy.
  if (["The speech recognition transport failed.", "The speech recognition session is not connected.", "The speech recognition connection closed."].includes(error)) return diagnosticCopy().speechUnreachable;
  if (["The speech recognition connection could not be established in time.", "The speech recognition connection stopped responding."].includes(error)) return diagnosticCopy().speechTimeout;
  const builtinMessage = builtinServiceErrorMessage(error);
  if (builtinMessage) return builtinMessage;
  if (error === "credential_authentication_failed") return diagnosticCopy().auth;
  if (/^Add the connection credentials for .+ in Settings\.$/.test(error)) return diagnosticCopy().missing;
  if (/^(The saved credentials could not be read\.|The saved credentials do not match the selected service\.|One or more credential fields are invalid\.)$/.test(error)) return diagnosticCopy().invalid;
  return null;
}
export function profileErrorMessage(error: unknown): string {
  const label = error instanceof Error ? error.message : error;
  if (label === "profile_language_preset_unsupported") return I18N.settings.profileLanguagesUnsupported;
  if (label === "profile_switch_recording_requires_stop") return I18N.settings.profileSwitchRecordingRequiresStop;
  if (label === "profile_switch_busy" || label === "profile_switch_superseded") return I18N.settings.profileSwitchBusy;
  if (typeof error === "string" && error.startsWith("Use an HTTPS DeepLX endpoint")) return I18N.settings.deepLXEndpointInvalid;
  return sessionActionErrorMessage(label, I18N.settings.profileActionFailed);
}

/** Exact safe labels only: navigation never starts capture or downloads resources. */
export function sessionErrorSettingsTarget(error: unknown): SettingsNavigationTarget | null {
  const label = error instanceof Error ? error.message : error;
  if (typeof label === "string" && [
    "apple_speech_assets_missing", "apple_speech_preparing", "apple_speech_prepare_failed",
    "apple_speech_service_unavailable", "apple_speech_reservation_limit", "apple_speech_download_cancelled",
    "apple_speech_download_timeout", "apple_speech_download_network", "apple_speech_download_storage",
    "apple_speech_resources_unavailable",
  ].includes(label)) {
    return "appleSpeechResources";
  }
  if (typeof label !== "string") return null;
  if ([
    "apple_translation_assets_missing",
    "apple_translation_source_required",
    "apple_translation_language_unsupported",
    "apple_translation_unavailable",
    "apple_translation_prepare_failed",
    "apple_translation_status_failed",
    "credential_authentication_failed",
    "custom_speech_authentication_failed",
    "custom_speech_credentials_missing",
    "custom_speech_endpoint_invalid",
    "custom_speech_model_invalid",
    "text_translation_credentials_missing",
    "tencent_configuration_rejected",
    "tencent_service_activation_required",
    "tencent_quota_exhausted",
  ].includes(label) || builtinServiceErrors.missing.has(label) ||
    builtinServiceErrors.invalidConfiguration.has(label) || builtinServiceErrors.unsupportedLanguage.has(label)) return "service";
  return audio3ErrorRequiresConfiguration(label) ? "service" : null;
}

/** Keep language failures actionable without exposing arbitrary IPC/provider text. */
export function languageActionErrorMessage(error: unknown, fallback: string): string {
  const label = error instanceof Error ? error.message : error;
  if (label === "source_switch_busy" || label === "target_switch_busy") return I18N.settings.languageSwitchBusy;
  if (label === "source_switch_superseded" || label === "language_switch_superseded") return I18N.settings.languageSwitchSuperseded;
  if (label === "source_switch_unsupported" || label === "target_switch_unsupported") return I18N.settings.languageSwitchUnsupported;
  if (label === "source_switch_save_failed" || label === "target_switch_save_failed") return I18N.settings.languageSaveFailed;
  if (label === "source_switch_profile" || label === "target_switch_profile") return I18N.settings.languageSwitchProfileUnavailable;
  if (label === "Listening settings cannot be changed while a session is active.") return I18N.settings.languageChangeRequiresStop;
  return sessionActionErrorMessage(label, fallback);
}


/** Reuse the same allowlisted provider causes for pause/resume and start actions. */
export function sessionActionErrorMessage(error: unknown, fallback: string): string {
  const label = error instanceof Error ? error.message : error;
  return credentialErrorMessage(label) ?? (typeof label === "string" ? audio3ErrorMessage(label) : null) ?? fallback;
}
