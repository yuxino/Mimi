import { effectiveUiLanguage, I18N } from "./i18n";
import type { ConnectionDiagnostic } from "./ipc";

export type DiagnosticPlatform = "macos" | "windows" | "linux";
const copy = {
  en: {
    localDevReadOnly: "Development presets read their provider keys from the local .env. Edit that file and restart mimi dev to change a key. Other configurations use the system credential store and remain editable.",
    localDevTranslationLocked: "This development preset uses its built-in recognition and translation service.",
    localDevTranslationHelp: "Add a regular configuration to choose another service or independent text translation. Its credentials use the development app's system credential store; the preset keys stay in the read-only .env.",
    localDevUnavailable: "Cannot read the local dev .env. Check its format, ownership and 0600 permissions, then restart mimi dev.",
    storage: "Cannot read service credentials. Handle the system unlock prompt, then check again.",
    linuxStorage: "Cannot access your desktop password store. Confirm a Secret Service provider, such as GNOME Keyring, is installed and enabled in this desktop session. Unlock it or allow the system prompt, then check again.",
    serviceUnavailable: "Linux Secret Service is unavailable. Install or enable a Secret Service provider, such as GNOME Keyring, in this desktop session. Sign out and back in if required by your desktop setup, then check again.",
    accessDenied: "The system credential store is locked or access was denied. Unlock it or allow the system prompt, then check again.",
    missing: "No credentials configured. Save your service settings first.",
    invalid: "Cannot read the saved credentials. Save them again.",
    auth: "The service rejected authentication. Check your key and account access, then update the credentials.",
    quota: "The service limit was reached. Try again later or check your quota.",
    translationLimited: "The service is still limiting requests. Try again later.",
    translationTemporary: "Translation is temporarily unavailable. Reconnect to try again.",
    translationBacklog: "Translation fell behind. Reconnect to continue.",
    translationSourceUnsupported: "This translation model does not support the detected language.",
    subtitleTooLarge: "The service returned too much text. Reconnect to continue.",
    timeout: "Connection timed out. Check your network or proxy, then try again.",
    unreachable: "Secure connection failed. Check your network, proxy and system clock.",
    test: "Check connection", testing: "Checking…",
    available: "Connection available", unavailable: "Unavailable", notTested: "Not checked",
    checkSkipped: "UI preview mode does not check connections. Use normal mode to check.",
    elapsed: "Check duration",
    elapsedHelp: "Recognition measures session setup. Translation measures one short text request. This is not live subtitle latency.",
    checkFailed: "Check failed. Try again.",
    macosRecovery: "Unlock Keychain or allow access, then check again.",
    windowsRecovery: "Check access in Credential Manager, then try again.",
    linuxRecovery: "Unlock the password store or allow access, then check again.",
    reasons: {
      credentialsMissing: "Save your credentials first.",
      textTranslationNotConfigured: "Choose and save a text translation service first.",
      credentialsUnavailable: "Unlock your system credential store.",
      credentialsServiceUnavailable: "Enable Secret Service (such as GNOME Keyring), then check again.",
      credentialsAccessDenied: "Unlock the credential store or allow access.",
      invalidConfiguration: "Check the service settings.",
      authenticationRejected: "Check your credentials and account access.",
      serviceRejected: "The service rejected the request.",
      timeout: "Connection timed out.",
      unreachable: "Could not connect to the service.",
    },
  },
  zh: {
    localDevReadOnly: "开发预设从本机 .env 读取对应服务的密钥。修改密钥后需重新打开 mimi dev；其他配置使用系统钥匙串，可正常编辑。",
    localDevTranslationLocked: "此开发预设使用内置服务进行识别与翻译。",
    localDevTranslationHelp: "添加普通配置即可选择其他服务或独立文字翻译。普通配置的凭据保存在开发版自己的系统钥匙串中，预设密钥仍由只读 .env 提供。",
    localDevUnavailable: "无法读取本机开发 .env。请检查格式、文件归属及 0600 权限，然后重新打开 mimi dev。",
    storage: "无法读取服务凭据。先处理系统解锁提示，再重新检查。",
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
      credentialsUnavailable: "请先解锁系统凭据存储。",
      credentialsServiceUnavailable: "启用 Secret Service（如 GNOME Keyring）后重试。",
      credentialsAccessDenied: "解锁凭据存储或允许访问。",
      invalidConfiguration: "请检查服务配置。",
      authenticationRejected: "请检查凭据和账号权限。",
      serviceRejected: "服务拒绝了请求。",
      timeout: "连接超时。",
      unreachable: "无法连接服务。",
    },
  },
  ja: {
    localDevReadOnly: "開発プリセットはローカルの .env から各サービスのキーを読み込みます。キーを変更したら mimi dev を再起動してください。他の設定はシステムの認証情報ストアを使用し、通常どおり編集できます。",
    localDevTranslationLocked: "この開発プリセットは組み込みサービスで認識と翻訳を行います。",
    localDevTranslationHelp: "通常の設定を追加すると、別のサービスや独立したテキスト翻訳を選べます。認証情報は開発アプリ専用のシステムストアに保存され、プリセットのキーは読み取り専用の .env に残ります。",
    localDevUnavailable: "開発用 .env を読めません。形式、所有者、0600 権限を確認し、mimi dev を再起動してください。",
    storage: "サービスの認証情報を読めません。システムの解除案内を確認し、もう一度お試しください。",
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
      credentialsUnavailable: "認証情報の保存先を解除してください。",
      credentialsServiceUnavailable: "GNOME Keyring などの Secret Service を有効にし、再確認してください。",
      credentialsAccessDenied: "認証情報ストアのロックを解除するかアクセスを許可してください。",
      invalidConfiguration: "サービス設定を確認してください。",
      authenticationRejected: "認証情報とアカウントの権限を確認してください。",
      serviceRejected: "サービスがリクエストを拒否しました。",
      timeout: "接続がタイムアウトしました。",
      unreachable: "サービスに接続できません。",
    },
  },
};
export function diagnosticPlatform(userAgent = typeof navigator === "undefined" ? "" : navigator.userAgent): DiagnosticPlatform {
  return /Mac/.test(userAgent) ? "macos" : /Windows/.test(userAgent) ? "windows" : "linux";
}
export function diagnosticCopy(platform: DiagnosticPlatform = diagnosticPlatform()) {
  const labels = copy[effectiveUiLanguage()];
  const recovery = labels[`${platform}Recovery`];
  return {
    ...labels,
    storage: platform === "linux" ? labels.linuxStorage : labels.storage,
    reasons: { ...labels.reasons, credentialsUnavailable: recovery, credentialsAccessDenied: recovery,
      localDevCredentialsUnavailable: labels.localDevUnavailable },
  };
}
export function credentialUnavailableHelp(platform: DiagnosticPlatform = diagnosticPlatform()): string {
  return platform === "linux" ? diagnosticCopy(platform).storage : I18N.settings.credentialUnavailableHelp;
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
  en: {
    timeout: "DeepLX timed out. Check the text translation endpoint, network and server, then restart subtitles.",
    connection: "Could not connect to DeepLX. Check the text translation endpoint, network and server, then restart subtitles.",
    response: "DeepLX returned an invalid or empty translation. Check that the text endpoint supports the DeepLX /translate JSON API.",
    size: "DeepLX returned too much data. Check the server's /translate response.",
    rejected: "DeepLX rejected the request. Confirm the text translation endpoint and optional Bearer token with your server administrator.",
  },
  ja: {
    timeout: "DeepLX がタイムアウトしました。文字翻訳 URL、ネットワーク、サーバーを確認し、字幕を再開してください。",
    connection: "DeepLX に接続できません。文字翻訳 URL、ネットワーク、サーバーを確認し、字幕を再開してください。",
    response: "DeepLX の翻訳が無効または空です。文字翻訳 URL が DeepLX /translate JSON API に対応しているか確認してください。",
    size: "DeepLX の応答が大きすぎます。サーバーの /translate 応答を確認してください。",
    rejected: "DeepLX がリクエストを拒否しました。文字翻訳 URL と任意の Bearer token をサーバー管理者に確認してください。",
  },
};
const openAICompatibleErrors = {
  zh: {
    timeout: "第三方翻译服务响应超时。请检查服务和网络，再重新启动字幕。",
    connection: "无法连接第三方翻译服务。请检查服务地址和网络，再重新启动字幕。",
    response: "第三方翻译服务返回无效或空文本。请确认支持非流式 Chat Completions，并返回 choices[0].message.content。",
    size: "第三方翻译服务返回数据过大。请检查服务器的响应。",
    rejected: "第三方翻译服务拒绝请求。请检查 API Key、模型名称和服务地址。",
  },
  en: {
    timeout: "The third-party translation service timed out. Check the service and network, then restart subtitles.",
    connection: "Could not connect to the third-party translation service. Check its address and network, then restart subtitles.",
    response: "The third-party service returned invalid or empty text. Check that it supports non-streaming Chat Completions and returns choices[0].message.content.",
    size: "The third-party translation service returned too much data. Check the server response.",
    rejected: "The third-party translation service rejected the request. Check its API key, model name and service address.",
  },
  ja: {
    timeout: "外部翻訳サービスがタイムアウトしました。サービスとネットワークを確認し、字幕を再開してください。",
    connection: "外部翻訳サービスに接続できません。URL とネットワークを確認し、字幕を再開してください。",
    response: "外部翻訳サービスのテキストが無効または空です。非ストリーミング Chat Completions と choices[0].message.content の応答に対応しているか確認してください。",
    size: "外部翻訳サービスの応答が大きすぎます。サーバーの応答を確認してください。",
    rejected: "外部翻訳サービスがリクエストを拒否しました。API Key、モデル名、サービス URL を確認してください。",
  },
};
/** Match only sanitized backend labels; never interpolate arbitrary native errors. */
export function credentialErrorMessage(error: unknown, platform?: DiagnosticPlatform): string | null {
  if (typeof error !== "string") return null;
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
  if (["The live translation transport failed.", "The OpenAI Realtime Translation connection failed."].includes(error)) return diagnosticCopy().unreachable;
  if (["The live translation connection could not be established in time.", "The live translation connection stopped responding.", "The OpenAI Realtime Translation connection stopped responding."].includes(error)) return diagnosticCopy().timeout;
  if (error === "credential_authentication_failed") return diagnosticCopy().auth;
  if (/^Add the connection credentials for .+ in Settings\.$/.test(error)) return diagnosticCopy().missing;
  if (/^(The saved credentials could not be read\.|The saved credentials do not match the selected service\.|One or more credential fields are invalid\.)$/.test(error)) return diagnosticCopy().invalid;
  return null;
}
export function profileErrorMessage(error: unknown): string {
  if (typeof error === "string" && error.startsWith("Use an HTTPS DeepLX endpoint")) return I18N.settings.deepLXEndpointInvalid;
  return credentialErrorMessage(error) ?? I18N.settings.profileActionFailed;
}
