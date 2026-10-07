/**
 * IPC contract types for the mimi Tauri frontend.
 *
 * These mirror the Tauri command payloads and `src-tauri/src/core/models.rs`
 * (language enums and status semantics).
 */

import { I18N, effectiveUiLanguage, isChineseSystem, localizedRecord } from "./i18n";

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

type SessionStatus =
  | { kind: "idle" }
  | { kind: "connecting" }
  | { kind: "listening" }
  | { kind: "stopping" }
  | { kind: "error"; message: string };

interface SubtitleLineSnapshot {
  text: string;
  isFinal: boolean;
  /** Provider utterance this line belongs to. Both lines of one utterance
   * carry the source id, so previews only stack lines of the same sentence. */
  utteranceId?: string | null;
}

interface SubtitleHistoryItem {
  audioSource?: AudioSource;
  source: string;
  translation: string;
  /** Epoch milliseconds. */
  createdAt: number;
}

export interface SourceSubtitleSnapshot {
  displayPair?: SubtitleSnapshot["previewPair"];
  /** Reducer confirmation, independent of preview completion. Missing means unconfirmed. */
  displayPairFinal?: boolean;
  audioSource: AudioSource;
  source: SubtitleLineSnapshot;
  translation: SubtitleLineSnapshot;
  history: SubtitleHistoryItem[];
  previewPair?: SubtitleSnapshot["previewPair"];
  detectedLanguage: string | null;
  isTranslationPending: boolean;
  isTranslationPreviewPending?: boolean;
  isTranslationTimedOut: boolean;
  translationRecovery?: SessionStateEvent["translationRecovery"];
}

export interface SubtitleSnapshot {
  /** One bounded complete current pair, independent from saved history. */
  displayPair?: SubtitleSnapshot["previewPair"];
  /** Reducer confirmation, independent of preview completion. Missing means unconfirmed. */
  displayPairFinal?: boolean;
  /** Independent source streams; top-level history remains chronological. */
  tracks?: SourceSubtitleSnapshot[];
  /** One replaceable completed preview; never confirmed history. */
  previewPair?: {
    source: string;
    translation: string;
    /** Opaque owner of this completed pair, independent of newer raw ASR. */
    utteranceId?: string | null;
  } | null;
  source: SubtitleLineSnapshot;
  translation: SubtitleLineSnapshot;
  history: SubtitleHistoryItem[];
}

export interface SessionStateEvent {
  debugSnapshotId?: number | null;
  /** Latest content-free timing samples. Absent values are not measurements. */
  apiLatencyMs?: number | null;
  translationLatencyMs?: number | null;
  translationLatencyKind?: "request" | "follow" | null;
  /** MT backoff leaves the selected audio input and recognition running. */
  translationRecovery?: {
    reason: "rateLimited" | "temporarilyUnavailable";
    retryAfterMs: number;
    /** Legacy snapshots imply true; false means no preview retry is queued. */
    retryScheduled?: boolean;
  } | null;
  status: SessionStatus;
  isActive: boolean;
  isPaused: boolean;
  isOverlayCollapsed: boolean;
  subtitles: SubtitleSnapshot;
  /** "zh" | "ja" | "en" | "ko" | ... (normalized language code). */
  detectedLanguage: string | null;
  isTranslationPending: boolean;
  /** Actual replaceable-preview HTTP work, independent of final-pair waiting. */
  isTranslationPreviewPending?: boolean;
  /** The latest source final outlived its translation deadline. */
  isTranslationTimedOut: boolean;
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

export interface SettingsSnapshot {
  /** Availability metadata only; credential bytes are never in snapshots. */
  credentialStorage?: "keychain" | "localDevFile" | "localFile";
  /** Service profiles never contain credential material, only availability. */
  profiles: ServiceProfile[];
  activeProfileId: string;
  sourceLanguage: SourceLanguage;
  targetLanguage: TargetLanguage;
  /** Native options for this exact active profile and selected target. Older snapshots omit this. */
  languageCapabilities?: LanguageCapabilitiesSnapshot;
  translationMode: TranslationMode;
  /** 14..20 */
  fontSize: number;
  /** Empty or omitted uses the language-aware system font stack. */
  subtitleFontFamily?: string;
  /** 0..100, background only. */
  subtitleBackgroundOpacity: number;
  subtitleColor: SubtitleColor;
  microphoneSubtitleColor?: SubtitleColor;
  subtitleAlignment: SubtitleAlignment;
  subtitleDisplayMode: SubtitleDisplayMode;
  /** Missing legacy settings preserve immediate previews. */
  showIntermediateSubtitles?: boolean;
  showSubtitleDividers: boolean;
  /** Missing legacy settings keep confirmation times hidden. */
  showSubtitleTimestamps?: boolean;
  /** `null` follows the system reduce-motion setting. */
  pulseAnimation: boolean | null;
  pulseStyle: PulseStyle;
  subtitleAnimation: boolean | null;
  subtitleBlendsWithBackground: boolean;
  isOverlayLocked: boolean;
  /** UI language override; `null` or `system` follows the system language. */
  uiLanguage: UiLanguage | null;
  retainSessionHistory: boolean;
  recordSessionAudio: boolean;
  audioInput: AudioInput;
  /** Native release availability; missing snapshots keep the microphone hidden. */
  microphoneInputAvailable?: boolean;
  windowsAudioSource: string;
  systemAudioTarget: SystemAudioTarget;
  /** macOS only; false retains menu-bar utility behavior. */
  showInDock: boolean;
  networkProxy: NetworkProxyConfig;
}

export type NetworkProxyMode = "system" | "direct" | "custom";
/** Credential-free route for a provider HTTP or WebSocket stage. */
export interface NetworkProxyConfig {
  mode: NetworkProxyMode;
  url: string | null;
}

export type UiLanguage = "system" | "zh" | "en" | "ja";
export type AudioSource = "system" | "microphone";
export type AudioInput = AudioSource | "both";
export type SystemAudioTarget = { kind: "system" } | { kind: "application"; id: string; name: string };
export type PulseStyle = "syllable" | "ribbon";
export type SubtitleDisplayMode = "translation" | "bilingual" | "original";
export type SubtitlePresetColor = "white" | "teal" | "yellow" | "green" | "pink";
export type SubtitleColor = SubtitlePresetColor | `#${string}`;
export type SubtitleAlignment = "left" | "center" | "right";

export interface SettingsDraft {
  sourceLanguage?: SourceLanguage;
  targetLanguage?: TargetLanguage;
  translationMode?: TranslationMode;
  fontSize?: number;
  subtitleFontFamily?: string;
  subtitleBackgroundOpacity?: number;
  subtitleColor?: SubtitleColor;
  microphoneSubtitleColor?: SubtitleColor;
  subtitleAlignment?: SubtitleAlignment;
  subtitleDisplayMode?: SubtitleDisplayMode;
  showIntermediateSubtitles?: boolean;
  showSubtitleDividers?: boolean;
  showSubtitleTimestamps?: boolean;
  pulseAnimation?: boolean;
  pulseStyle?: PulseStyle;
  subtitleAnimation?: boolean;
  subtitleBlendsWithBackground?: boolean;
  isOverlayLocked?: boolean;
  uiLanguage?: UiLanguage;
  retainSessionHistory?: boolean;
  recordSessionAudio?: boolean;
  audioInput?: AudioInput;
  windowsAudioSource?: string;
  systemAudioTarget?: SystemAudioTarget;
  showInDock?: boolean;
  networkProxy?: NetworkProxyConfig;
}

export type ServiceProvider =
  | "appleSpeech"
  | "alibabaCloud"
  | "openAIRealtime"
  | "googleGeminiLive"
  | "azureOpenAIRealtime"
  | "volcanoEngine"
  | "tencentCloud"
  | "baiduTranslate"
  | "xAIRealtime"
  | "customDashScopeASR"
  | "customOpenAIASR"
  | "deepLX";

export type TextTranslation = "followService" | "apple" | "deepL" | "deepLX" | "openAICompatible" | "chatMock";

/** Write-only payload sent to the native secure credential store. */
export type ProviderCredentialsInput =
  | { kind: "customSpeech"; endpoint: string; model: string; apiKey: string }
  | { kind: "alibabaTranslation"; apiKey: string; textTranslation: TextTranslation; endpoint: string; token: string; model: string; clearToken?: boolean }
  | { kind: "deepLX"; asrApiKey: string; endpoint: string; token: string }
  | { kind: "apiKey"; apiKey: string }
  | {
      kind: "azureOpenAI";
      endpoint: string;
      deployment: string;
      transcriptionDeployment: string;
      apiKey: string;
    }
  | {
      kind: "tencentCloud";
      appId: string;
      secretId: string;
      secretKey: string;
    }
  | { kind: "baiduTranslate"; appId: string; appKey: string };

/**
 * Sanitized keychain state returned by native snapshots. A replacement key
 * crosses IPC only in the dedicated write-only save command and is never
 * returned to the frontend.
 */
export type CredentialState = "present" | "missing" | "unavailable";

export type ProfileNetworkProxyDraft = Partial<Pick<ServiceProfile, "speechNetworkProxy" | "textNetworkProxy">>;
export type TextTranslationNameDraft = { route: Exclude<TextTranslation, "followService">; name: string };
export type ProfileOptionsDraft = ProfileNetworkProxyDraft & {
  speechRecognitionName?: string;
  textTranslationName?: TextTranslationNameDraft;
  customSpeechSourceLanguages?: SourceLanguage[] | null;
};

export interface ServiceProfile {
  id: string;
  name: string;
  provider: ServiceProvider;
  credentialState: CredentialState;
  /** Storage kind only; credential values remain in the backend. */
  credentialStorage?: "keychain" | "localDevFile" | "localFile";
  /** Custom speech profiles expose each independent store's availability, never its values. */
  speechCredentialState?: CredentialState;
  textCredentialState?: CredentialState;
  /** Optional for historical/native fixture snapshots; inferred from provider when absent. */
  textTranslation?: TextTranslation;
  /** User-declared custom ASR languages, not discovered capabilities. Null/absent is unknown; [] keeps only service default. */
  customSpeechSourceLanguages?: SourceLanguage[] | null;
  /** Display metadata only; names stay independent for each saved translation route. */
  textTranslationNames?: Partial<Record<Exclude<TextTranslation, "followService">, string>>;
  /** Optional display name for custom speech recognition, independent of the configuration name. */
  speechRecognitionName?: string;
  /** Missing legacy fields inherit SettingsSnapshot.networkProxy. */
  speechNetworkProxy?: NetworkProxyConfig | null;
  textNetworkProxy?: NetworkProxyConfig | null;
}

export interface ProviderCapabilities {
  sourceLanguages: readonly SourceLanguage[];
  targetLanguages: readonly TargetLanguage[];
  translationModes: readonly TranslationMode[];
}

export type AppleSpeechResourceStatus = "unsupported" | "supported" | "downloading" | "installed" | "unknown";

export interface AppleSpeechSupport {
  available: boolean;
  languages: { sourceLanguage: Exclude<SourceLanguage, "auto">; locale: string; installed: boolean; downloading?: boolean; status?: AppleSpeechResourceStatus }[];
}

export interface AppleTranslationSupport {
  available: boolean;
  sourceLanguages: Exclude<SourceLanguage, "auto">[];
  targetLanguages: Exclude<TargetLanguage, "original">[];
}

export type AppleTranslationStatus = "installed" | "supported" | "unsupported" | "unavailable";

export interface LanguageCapabilitiesSnapshot {
  profileId: string;
  provider: ServiceProvider;
  textTranslation: TextTranslation;
  targetLanguage: TargetLanguage;
  sourceLanguages: readonly SourceLanguage[];
  targetLanguages: readonly TargetLanguage[];
  /** Complete Apple resource status, including languages excluded by this route. */
  appleSpeechSupportRevision?: number;
  /** Declaration used to intersect a native translation catalog with custom ASR. */
  customSpeechSourceLanguages?: readonly SourceLanguage[] | null;
}

// ---------------------------------------------------------------------------
// Languages
// ---------------------------------------------------------------------------

/** Verified Audio 3.0 language_hints codes; Automatic omits the hint. */
export const AUDIO3_RECOGNITION_LANGUAGE_CODES = Object.freeze([
  "zh", "en", "ja", "ko", "vi", "th", "id", "ms", "tl", "hi", "ar", "fr", "de", "es", "pt",
  "ru", "it", "nl", "sv", "da", "fi", "no", "el", "pl", "cs", "hu", "ro", "bg", "hr", "sk",
] as const);

/** The current Qwen-MT Lite table, not another model's larger language range. */
export const QWEN_MT_LITE_TRANSLATION_LANGUAGE_CODES = Object.freeze([
  "en", "zh", "zh_tw", "ru", "ja", "ko", "es", "fr", "pt", "de", "it", "th", "vi", "id", "ms",
  "ar", "hi", "he", "ur", "bn", "pl", "nl", "tr", "km", "cs", "sv", "hu", "da", "fi", "tl", "fa",
] as const);

/** Serializable source codes. Provider capabilities determine which choices are supported. */
export const SOURCE_LANGUAGE_CODES = Object.freeze([
  "zh", "en", "ja", "ko", "vi", "th", "id", "ms", "tl", "hi", "ar", "fr", "de", "es", "pt", "ru", "it", "nl",
  "sv", "da", "fi", "no", "el", "pl", "cs", "hu", "ro", "bg", "hr", "sk", "wuu", "ast", "ace", "af", "ak", "am", "an",
  "ar-AE", "ar-EG", "ar-SA", "as", "ay", "az", "ba", "be", "bho", "bn", "br", "bs", "ca", "ceb", "ckb", "cy",
  "en-GB", "en-US", "eo", "es-419", "es-ES", "es-MX", "et", "eu", "fa", "ga", "gl", "gn", "gom", "gu", "ha",
  "he", "ht", "hy", "ig", "is", "jv", "ka", "kk", "km", "kmr", "kn", "ky", "la", "lb", "lmo", "ln", "lo",
  "lt", "lv", "mai", "mg", "mi", "mk", "ml", "mn", "mr", "mt", "my", "ne", "oc", "om", "pa", "pag", "pam",
  "prs", "ps", "pt-BR", "pt-PT", "qu", "rw", "sa", "scn", "sd", "si", "sl", "sq", "sr", "st", "su", "sw",
  "ta", "te", "tg", "tk", "tn", "tr", "ts", "tt", "uk", "ur", "uz", "wo", "xh", "yi", "yue", "zh_en",
  "zh_tw", "zu",
] as const);

/** Serializable target codes. Provider capabilities determine which choices are supported. */
export const TARGET_LANGUAGE_CODES = Object.freeze([
  "zh", "en", "ja", "zh_tw", "ko", "ru", "es", "fr", "pt", "de", "it", "th", "vi", "id", "ms", "ar", "hi",
  "he", "ur", "bn", "pl", "nl", "tr", "km", "cs", "sv", "hu", "da", "fi", "tl", "fa", "wuu", "ast", "ace", "af", "ak",
  "am", "an", "ar-AE", "ar-EG", "ar-SA", "as", "ay", "az", "ba", "be", "bg", "bho", "br", "bs", "ca", "ceb",
  "ckb", "cy", "el", "en-GB", "en-US", "eo", "es-419", "es-ES", "es-MX", "et", "eu", "ga", "gl", "gn", "gom",
  "gu", "ha", "hr", "ht", "hy", "ig", "is", "jv", "ka", "kk", "kmr", "kn", "ky", "la", "lb", "lmo", "ln",
  "lo", "lt", "lv", "mai", "mg", "mi", "mk", "ml", "mn", "mr", "mt", "my", "ne", "no", "oc", "om", "pa",
  "pag", "pam", "prs", "ps", "pt-BR", "pt-PT", "qu", "ro", "rw", "sa", "scn", "sd", "si", "sk", "sl", "sq",
  "sr", "st", "su", "sw", "ta", "te", "tg", "tk", "tn", "ts", "tt", "uk", "uz", "wo", "xh", "yi", "yue",
  "zh_en", "zu",
] as const);

export type SourceLanguage = "auto" | typeof SOURCE_LANGUAGE_CODES[number];
export type TargetLanguage = "original" | typeof TARGET_LANGUAGE_CODES[number];
export type TranslationMode = "lowLatency" | "highQuality" | "turbo";

/** Conservative ASR scope for translation routes without expanded language mapping. */
export const LEGACY_SOURCE_LANGUAGE_CASES: readonly SourceLanguage[] = [
  "auto",
  "ja",
  "en",
  "ko",
  "zh",
];

export const TRANSLATION_MODE_CASES: readonly TranslationMode[] = [
  "turbo",
];

type LanguageDisplayCode = Exclude<SourceLanguage, "auto"> | Exclude<TargetLanguage, "original">;
type LanguageDisplayLocale = "zh" | "en" | "ja";

const LANGUAGE_DISPLAY_NAMES: Record<LanguageDisplayLocale, Record<LanguageDisplayCode, string>> = {
  zh: {
    zh: "中文", zh_tw: "繁体中文", en: "英语", ja: "日语", ko: "韩语", vi: "越南语",
    th: "泰语", id: "印尼语", ms: "马来语", tl: "菲律宾语", hi: "印地语", ar: "阿拉伯语",
    fr: "法语", de: "德语", es: "西班牙语", pt: "葡萄牙语", ru: "俄语", it: "意大利语",
    nl: "荷兰语", sv: "瑞典语", da: "丹麦语", fi: "芬兰语", no: "挪威语", el: "希腊语",
    pl: "波兰语", cs: "捷克语", hu: "匈牙利语", ro: "罗马尼亚语", bg: "保加利亚语",
    hr: "克罗地亚语", sk: "斯洛伐克语", he: "希伯来语", ur: "乌尔都语", bn: "孟加拉语",
    tr: "土耳其语", km: "高棉语", fa: "波斯语",
    "wuu": "上海话", "ast": "阿斯图里亚斯语", "ace": "亚齐语", "af": "南非荷兰语", "ak": "阿肯语", "am": "阿姆哈拉语", "an": "阿拉贡语", "ar-AE": "阿拉伯语（阿拉伯联合酋长国）",
    "ar-EG": "阿拉伯语（埃及）", "ar-SA": "阿拉伯语（沙特阿拉伯）", "as": "阿萨姆语", "ay": "艾马拉语", "az": "阿塞拜疆语", "ba": "巴什基尔语",
    "be": "白俄罗斯语", "bho": "博杰普尔语", "br": "布列塔尼语", "bs": "波斯尼亚语", "ca": "加泰罗尼亚语", "ceb": "宿务语",
    "ckb": "中库尔德语", "cy": "威尔士语", "en-GB": "英国英语", "en-US": "美国英语", "eo": "世界语", "es-419": "拉丁美洲西班牙语",
    "es-ES": "欧洲西班牙语", "es-MX": "墨西哥西班牙语", "et": "爱沙尼亚语", "eu": "巴斯克语", "ga": "爱尔兰语", "gl": "加利西亚语",
    "gn": "瓜拉尼语", "gom": "孔卡尼语", "gu": "古吉拉特语", "ha": "豪萨语", "ht": "海地克里奥尔语", "hy": "亚美尼亚语", "ig": "伊博语",
    "is": "冰岛语", "jv": "爪哇语", "ka": "格鲁吉亚语", "kk": "哈萨克语", "kmr": "北库尔德语", "kn": "卡纳达语", "ky": "柯尔克孜语",
    "la": "拉丁语", "lb": "卢森堡语", "lmo": "伦巴第语", "ln": "林加拉语", "lo": "老挝语", "lt": "立陶宛语", "lv": "拉脱维亚语",
    "mai": "迈蒂利语", "mg": "马拉加斯语", "mi": "毛利语", "mk": "马其顿语", "ml": "马拉雅拉姆语", "mn": "蒙古语", "mr": "马拉地语",
    "mt": "马耳他语", "my": "缅甸语", "ne": "尼泊尔语", "oc": "奥克语", "om": "奥罗莫语", "pa": "旁遮普语", "pag": "邦阿西南语",
    "pam": "邦板牙语", "prs": "达里语", "ps": "普什图语", "pt-BR": "巴西葡萄牙语", "pt-PT": "欧洲葡萄牙语", "qu": "克丘亚语",
    "rw": "卢旺达语", "sa": "梵语", "scn": "西西里语", "sd": "信德语", "si": "僧伽罗语", "sl": "斯洛文尼亚语", "sq": "阿尔巴尼亚语",
    "sr": "塞尔维亚语", "st": "南索托语", "su": "巽他语", "sw": "斯瓦希里语", "ta": "泰米尔语", "te": "泰卢固语", "tg": "塔吉克语",
    "tk": "土库曼语", "tn": "茨瓦纳语", "ts": "聪加语", "tt": "鞑靼语", "uk": "乌克兰语", "uz": "乌兹别克语", "wo": "沃洛夫语",
    "xh": "科萨语", "yi": "意第绪语", "yue": "粤语", "zh_en": "中英混合", "zu": "祖鲁语",
  },
  en: {
    zh: "Chinese", zh_tw: "Traditional Chinese", en: "English", ja: "Japanese", ko: "Korean",
    vi: "Vietnamese", th: "Thai", id: "Indonesian", ms: "Malay", tl: "Filipino", hi: "Hindi",
    ar: "Arabic", fr: "French", de: "German", es: "Spanish", pt: "Portuguese", ru: "Russian",
    it: "Italian", nl: "Dutch", sv: "Swedish", da: "Danish", fi: "Finnish", no: "Norwegian",
    el: "Greek", pl: "Polish", cs: "Czech", hu: "Hungarian", ro: "Romanian", bg: "Bulgarian",
    hr: "Croatian", sk: "Slovak", he: "Hebrew", ur: "Urdu", bn: "Bengali", tr: "Turkish",
    km: "Khmer", fa: "Persian",
    "wuu": "Shanghainese", "ast": "Asturian", "ace": "Acehnese", "af": "Afrikaans", "ak": "Akan", "am": "Amharic", "an": "Aragonese",
    "ar-AE": "Arabic (United Arab Emirates)", "ar-EG": "Arabic (Egypt)", "ar-SA": "Arabic (Saudi Arabia)",
    "as": "Assamese", "ay": "Aymara", "az": "Azerbaijani", "ba": "Bashkir", "be": "Belarusian",
    "bho": "Bhojpuri", "br": "Breton", "bs": "Bosnian", "ca": "Catalan", "ceb": "Cebuano",
    "ckb": "Central Kurdish", "cy": "Welsh", "en-GB": "British English", "en-US": "American English",
    "eo": "Esperanto", "es-419": "Latin American Spanish", "es-ES": "European Spanish",
    "es-MX": "Mexican Spanish", "et": "Estonian", "eu": "Basque", "ga": "Irish", "gl": "Galician",
    "gn": "Guarani", "gom": "Konkani", "gu": "Gujarati", "ha": "Hausa", "ht": "Haitian Creole",
    "hy": "Armenian", "ig": "Igbo", "is": "Icelandic", "jv": "Javanese", "ka": "Georgian", "kk": "Kazakh",
    "kmr": "Northern Kurdish", "kn": "Kannada", "ky": "Kyrgyz", "la": "Latin", "lb": "Luxembourgish",
    "lmo": "Lombard", "ln": "Lingala", "lo": "Lao", "lt": "Lithuanian", "lv": "Latvian", "mai": "Maithili",
    "mg": "Malagasy", "mi": "Māori", "mk": "Macedonian", "ml": "Malayalam", "mn": "Mongolian",
    "mr": "Marathi", "mt": "Maltese", "my": "Burmese", "ne": "Nepali", "oc": "Occitan", "om": "Oromo",
    "pa": "Punjabi", "pag": "Pangasinan", "pam": "Pampanga", "prs": "Dari", "ps": "Pashto",
    "pt-BR": "Brazilian Portuguese", "pt-PT": "European Portuguese", "qu": "Quechua", "rw": "Kinyarwanda",
    "sa": "Sanskrit", "scn": "Sicilian", "sd": "Sindhi", "si": "Sinhala", "sl": "Slovenian",
    "sq": "Albanian", "sr": "Serbian", "st": "Southern Sotho", "su": "Sundanese", "sw": "Swahili",
    "ta": "Tamil", "te": "Telugu", "tg": "Tajik", "tk": "Turkmen", "tn": "Tswana", "ts": "Tsonga",
    "tt": "Tatar", "uk": "Ukrainian", "uz": "Uzbek", "wo": "Wolof", "xh": "Xhosa", "yi": "Yiddish",
    "yue": "Cantonese", "zh_en": "Chinese and English (mixed)", "zu": "Zulu",
  },
  ja: {
    zh: "中国語", zh_tw: "繁体中国語", en: "英語", ja: "日本語", ko: "韓国語",
    vi: "ベトナム語", th: "タイ語", id: "インドネシア語", ms: "マレー語", tl: "フィリピン語",
    hi: "ヒンディー語", ar: "アラビア語", fr: "フランス語", de: "ドイツ語", es: "スペイン語",
    pt: "ポルトガル語", ru: "ロシア語", it: "イタリア語", nl: "オランダ語", sv: "スウェーデン語",
    da: "デンマーク語", fi: "フィンランド語", no: "ノルウェー語", el: "ギリシャ語", pl: "ポーランド語",
    cs: "チェコ語", hu: "ハンガリー語", ro: "ルーマニア語", bg: "ブルガリア語", hr: "クロアチア語",
    sk: "スロバキア語", he: "ヘブライ語", ur: "ウルドゥー語", bn: "ベンガル語", tr: "トルコ語",
    km: "クメール語", fa: "ペルシア語",
    "wuu": "上海語", "ast": "アストゥリアス語", "ace": "アチェ語", "af": "アフリカーンス語", "ak": "アカン語", "am": "アムハラ語", "an": "アラゴン語", "ar-AE": "アラビア語 (アラブ首長国連邦)",
    "ar-EG": "アラビア語 (エジプト)", "ar-SA": "アラビア語 (サウジアラビア)", "as": "アッサム語", "ay": "アイマラ語", "az": "アゼルバイジャン語",
    "ba": "バシキール語", "be": "ベラルーシ語", "bho": "ボージュプリー語", "br": "ブルトン語", "bs": "ボスニア語", "ca": "カタロニア語",
    "ceb": "セブアノ語", "ckb": "中央クルド語", "cy": "ウェールズ語", "en-GB": "イギリス英語", "en-US": "アメリカ英語", "eo": "エスペラント語",
    "es-419": "スペイン語 (ラテンアメリカ)", "es-ES": "スペイン語 (イベリア半島)", "es-MX": "スペイン語 (メキシコ)", "et": "エストニア語",
    "eu": "バスク語", "ga": "アイルランド語", "gl": "ガリシア語", "gn": "グアラニー語", "gom": "コンカニ語", "gu": "グジャラート語",
    "ha": "ハウサ語", "ht": "ハイチ・クレオール語", "hy": "アルメニア語", "ig": "イボ語", "is": "アイスランド語", "jv": "ジャワ語",
    "ka": "ジョージア語", "kk": "カザフ語", "kmr": "北クルド語", "kn": "カンナダ語", "ky": "キルギス語", "la": "ラテン語",
    "lb": "ルクセンブルク語", "lmo": "ロンバルド語", "ln": "リンガラ語", "lo": "ラオ語", "lt": "リトアニア語", "lv": "ラトビア語",
    "mai": "マイティリー語", "mg": "マダガスカル語", "mi": "マオリ語", "mk": "マケドニア語", "ml": "マラヤーラム語", "mn": "モンゴル語",
    "mr": "マラーティー語", "mt": "マルタ語", "my": "ミャンマー語", "ne": "ネパール語", "oc": "オック語", "om": "オロモ語", "pa": "パンジャブ語",
    "pag": "パンガシナン語", "pam": "パンパンガ語", "prs": "ダリー語", "ps": "パシュトゥー語", "pt-BR": "ポルトガル語 (ブラジル)",
    "pt-PT": "ポルトガル語 (イベリア半島)", "qu": "ケチュア語", "rw": "キニアルワンダ語", "sa": "サンスクリット語", "scn": "シチリア語",
    "sd": "シンド語", "si": "シンハラ語", "sl": "スロベニア語", "sq": "アルバニア語", "sr": "セルビア語", "st": "南部ソト語", "su": "スンダ語",
    "sw": "スワヒリ語", "ta": "タミル語", "te": "テルグ語", "tg": "タジク語", "tk": "トルクメン語", "tn": "ツワナ語", "ts": "ツォンガ語",
    "tt": "タタール語", "uk": "ウクライナ語", "uz": "ウズベク語", "wo": "ウォロフ語", "xh": "コサ語", "yi": "イディッシュ語", "yue": "広東語",
    "zh_en": "中国語・英語（混在）", "zu": "ズールー語",
  },
};

function languageDisplayLocale(): LanguageDisplayLocale {
  return effectiveUiLanguage() === "ja" ? "ja" : isChineseSystem() ? "zh" : "en";
}

function languageNamesForCodes<Code extends LanguageDisplayCode>(
  codes: readonly Code[],
  names: Record<LanguageDisplayCode, string>,
): Record<Code, string> {
  return Object.fromEntries(codes.map((code) => [code, names[code]])) as Record<Code, string>;
}

export function sourceLanguageDisplayName(language: SourceLanguage, locale = languageDisplayLocale()): string {
  return language === "auto"
    ? { zh: "自动识别", en: "Auto Detect", ja: "自動認識" }[locale]
    : LANGUAGE_DISPLAY_NAMES[locale][language];
}

export function targetLanguageDisplayName(language: TargetLanguage, locale = languageDisplayLocale()): string {
  if (language === "original") {
    return { zh: "原文（不翻译）", en: "Original (no translation)", ja: "原文（翻訳しない）" }[locale];
  }
  if (language === "zh") return { zh: "简体中文", en: "Simplified Chinese", ja: "簡体中国語" }[locale];
  if (language === "tl") return { zh: "塔加洛语", en: "Tagalog", ja: "タガログ語" }[locale];
  return LANGUAGE_DISPLAY_NAMES[locale][language];
}

/** Localized source-language labels for the active UI language. */
export const SOURCE_LANGUAGE_DISPLAY_NAMES: Record<SourceLanguage, string> = localizedRecord(() => {
  const locale = languageDisplayLocale();
  return {
    auto: sourceLanguageDisplayName("auto", locale),
    ...languageNamesForCodes(SOURCE_LANGUAGE_CODES, LANGUAGE_DISPLAY_NAMES[locale]),
  };
});

/** Wire codes and scripts stay unchanged when display labels switch languages. */
export const TARGET_LANGUAGE_DISPLAY_NAMES: Record<TargetLanguage, string> = localizedRecord(() => {
  const locale = languageDisplayLocale();
  return {
    original: targetLanguageDisplayName("original", locale),
    ...languageNamesForCodes(TARGET_LANGUAGE_CODES, LANGUAGE_DISPLAY_NAMES[locale]),
    zh: targetLanguageDisplayName("zh", locale),
    tl: targetLanguageDisplayName("tl", locale),
  };
});

/** Display labels for normalized recognition-service language codes. */
const DETECTED_LANGUAGE_DISPLAY_NAMES: Record<string, string> = {
  zh: "中文",
  chinese: "中文",
  mandarin: "中文",
  yue: "粤语",
  cantonese: "粤语",
  en: "English",
  english: "English",
  ja: "日本語",
  japanese: "日本語",
  ko: "한국어",
  korean: "한국어",
  de: "Deutsch",
  fr: "Français",
  es: "Español",
  pt: "Português",
  it: "Italiano",
  ru: "Русский",
  ar: "العربية",
  hi: "हिन्दी",
  id: "Bahasa Indonesia",
  th: "ไทย",
  tr: "Türkçe",
  vi: "Tiếng Việt",
  uk: "Українська",
  cs: "Čeština",
  da: "Dansk",
  tl: "Filipino",
  fil: "Filipino",
  fi: "Suomi",
  is: "Íslenska",
  ms: "Bahasa Melayu",
  no: "Norsk",
  nb: "Norsk",
  pl: "Polski",
  sv: "Svenska",
  hu: "Magyar",
  el: "Ελληνικά",
  ro: "Română",
  bg: "Български",
  hr: "Hrvatski",
  sk: "Slovenčina",
  zh_tw: "繁體中文",
  he: "עברית",
  ur: "اردو",
  bn: "বাংলা",
  km: "ខ្មែរ",
  fa: "فارسی",
};

function detectedLanguageDisplayName(code: string): string {
  return DETECTED_LANGUAGE_DISPLAY_NAMES[code] ?? code.toUpperCase();
}

/** Builds the source-language status shown while automatic detection runs. */
export function sourceLanguageStatusDisplayName(
  sourceLanguage: SourceLanguage,
  detectedLanguage: string | null,
  targetLanguage: TargetLanguage,
): string {
  if (sourceLanguage !== "auto") {
    return SOURCE_LANGUAGE_DISPLAY_NAMES[sourceLanguage];
  }
  if (detectedLanguage === null) {
    return I18N.overlay.autoDetecting;
  }
  if (targetLanguage === "zh" && detectedLanguage === "zh") {
    return I18N.overlay.autoDetecting;
  }
  return `${I18N.overlay.autoDetectedPrefix}${detectedLanguageDisplayName(detectedLanguage)}${I18N.overlay.autoDetectedSuffix}`;
}

/** Whether the selected target requires machine translation. */
export function targetLanguageTranslatesAudio(target: TargetLanguage): boolean {
  return target !== "original";
}

// ---------------------------------------------------------------------------
// Overlay activity phase
// ---------------------------------------------------------------------------

export type OverlayActivityPhaseKind =
  | "idle"
  | "error"
  | "connecting"
  | "listening"
  | "recognizing"
  | "translating"
  | "paused";

interface OverlayActivityPhaseInfo {
  accessibilityLabel: string;
  /** Base RGB as `#RRGGBB`. */
  color: string;
  animationSpeed: number;
}

/**
 * Labels, colors and working states for the overlay activity indicator.
 */
export const OVERLAY_ACTIVITY_PHASES: Record<
  OverlayActivityPhaseKind,
  OverlayActivityPhaseInfo
> = {
  idle: {
    get accessibilityLabel() { return I18N.overlay.phaseIdle; },
    color: "#FFFFFF",
    animationSpeed: 0,
  },
  error: {
    get accessibilityLabel() { return I18N.overlay.phaseError; },
    color: "#FF8A80",
    animationSpeed: 0,
  },
  connecting: {
    get accessibilityLabel() { return I18N.overlay.phaseConnecting; },
    color: "#FFFFFF",
    animationSpeed: 2.6,
  },
  listening: {
    get accessibilityLabel() { return I18N.overlay.phaseListening; },
    color: "#7AA8FF",
    animationSpeed: 2.6,
  },
  recognizing: {
    get accessibilityLabel() { return I18N.overlay.phaseRecognizing; },
    color: "#7AA8FF",
    animationSpeed: 2.6,
  },
  translating: {
    get accessibilityLabel() { return I18N.overlay.phaseTranslating; },
    color: "#B894FF",
    animationSpeed: 2.6,
  },
  paused: {
    get accessibilityLabel() { return I18N.overlay.phasePaused; },
    color: "#FFB852",
    animationSpeed: 0,
  },
};

// ---------------------------------------------------------------------------
// Small color helpers
// ---------------------------------------------------------------------------

/** Converts a `#RRGGBB` color into an `rgba()` string. */
export function hexToRgba(hex: string, alpha: number): string {
  const r = Number.parseInt(hex.slice(1, 3), 16);
  const g = Number.parseInt(hex.slice(3, 5), 16);
  const b = Number.parseInt(hex.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Content-free metadata for the opt-in in-memory session archive. */
export interface SessionArchiveState {
  audioSources?: AudioSource[];
  transcriptCount: number;
  transcriptLimited: boolean;
  audioBytes: number;
  audioLimited: boolean;
  sampleRate: number;
  historySaveError: boolean;
}

export interface TranscriptPageEntry {
  audioSource?: AudioSource;
  index: number;
  source: string;
  translation: string;
  createdAtMs: number;
}

export interface TranscriptPage {
  total: number;
  page: number;
  entries: TranscriptPageEntry[];
}

export interface SessionHistoryItem {
  audioSources?: AudioSource[];
  id: string;
  startedAtMs: number;
  endedAtMs: number;
  count: number;
  limited: boolean;
  hasAudio: boolean;
}

export type SessionExportKind = "transcript" | "audio";
