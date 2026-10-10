import { effectiveUiLanguage } from "./i18n";

const copy = {
  th: {
    denied: "ยังไม่ได้รับสิทธิ์บันทึกเสียงระบบ ให้สิทธิ์ Mimi ในการตั้งค่าระบบ → ความเป็นส่วนตัวและความปลอดภัย → การบันทึกหน้าจอและเสียงระบบ จากนั้นเปิดแอปใหม่ตามคำแนะนำของระบบแล้วลองอีกครั้ง",
    open: "เปิดสิทธิ์เสียงระบบ",
    failed: "เปิดการตั้งค่าระบบไม่ได้ โปรดเปิดความเป็นส่วนตัวและความปลอดภัย → การบันทึกหน้าจอและเสียงระบบด้วยตนเอง",
    guide: "macOS 14.2 ขึ้นไปใช้สิทธิ์บันทึกเฉพาะเสียงระบบได้ หากมีสิทธิ์บันทึกหน้าจออยู่แล้ว ไม่ต้องอนุญาตใหม่ macOS รุ่นก่อนหน้านั้นต้องใช้สิทธิ์บันทึกหน้าจอและเสียงระบบ Mimi รับเฉพาะเสียงและไม่บันทึกภาพหน้าจอ",
  },
  "en": {
    "denied": "System audio recording permission is missing. In System Settings → Privacy & Security → Screen & System Audio Recording, allow Mimi to record system audio, then follow the system’s restart instructions and try again.",
    "open": "Open system audio permissions",
    "failed": "Could not open System Settings. Open Privacy & Security → Screen & System Audio Recording manually.",
    "guide": "On macOS 14.2 or later, use System Audio Recording Only. Existing screen-recording grants continue to work without another authorization. Earlier macOS versions require Screen & System Audio Recording. Mimi captures audio without saving screen images."
  },
  "zh": {
    "denied": "未获得系统录音权限。请前往「系统设置 → 隐私与安全性 → 录屏与系统录音」，允许 Mimi 采集系统声音，再按系统提示重新打开后重试。",
    "open": "打开系统录音权限",
    "failed": "无法打开系统设置，请手动前往「隐私与安全性 → 录屏与系统录音」。",
    "guide": "macOS 14.2 及以上可使用「仅系统录音」，已有录屏授权无需重新授权。更早的 macOS 需要「录屏与系统录音」。Mimi 只采集声音，不保存屏幕画面。"
  },
  "ja": {
    "denied": "システム音声の録音が許可されていません。「システム設定 → プライバシーとセキュリティ → 画面収録とシステムオーディオ録音」で Mimi を許可し、システムの案内に従って再起動してから再試行してください。",
    "open": "システム音声の権限を開く",
    "failed": "システム設定を開けません。「プライバシーとセキュリティ → 画面収録とシステムオーディオ録音」を手動で開いてください。",
    "guide": "macOS 14.2 以降では「システムオーディオ録音のみ」を使用できます。既存の画面収録の許可があれば再承認は不要です。それ以前の macOS では「画面収録とシステムオーディオ録音」が必要です。Mimi は音声だけを取得し、画面の画像を保存しません。"
  },
  "zh-TW": {
    "denied": "尚未取得系統錄音權限。請前往「系統設定 → 隱私權與安全性 → 螢幕與系統音訊錄製」，允許 Mimi 擷取系統聲音，再依系統提示重新開啟後重試。",
    "open": "開啟系統錄音權限",
    "failed": "無法開啟系統設定，請手動前往「隱私權與安全性 → 螢幕與系統音訊錄製」。",
    "guide": "macOS 14.2 以上可使用「僅系統音訊錄製」，已有螢幕錄製授權無須重新授權。更早的 macOS 需要「螢幕與系統音訊錄製」。Mimi 只擷取聲音，不儲存螢幕畫面。"
  },
  "de": {
    "denied": "Die Berechtigung für Systemaudioaufnahmen fehlt. Erlaube Mimi unter Systemeinstellungen → Datenschutz & Sicherheit → Bildschirm- und Systemaudioaufnahme den Zugriff auf Systemaudio. Folge den Neustarthinweisen des Systems und versuche es erneut.",
    "open": "Systemaudio-Berechtigungen öffnen",
    "failed": "Systemeinstellungen konnten nicht geöffnet werden. Öffne Datenschutz & Sicherheit → Bildschirm- und Systemaudioaufnahme manuell.",
    "guide": "Ab macOS 14.2 kannst du „Nur Systemaudioaufnahme“ verwenden. Eine bestehende Bildschirmaufnahme-Berechtigung funktioniert ohne erneute Freigabe. Ältere macOS-Versionen benötigen „Bildschirm- und Systemaudioaufnahme“. Mimi erfasst Audio, ohne Bildschirmbilder zu speichern."
  },
  "ko": {
    "denied": "시스템 오디오 녹음 권한이 없습니다. 시스템 설정 → 개인정보 보호 및 보안 → 화면 및 시스템 오디오 녹음에서 Mimi를 허용하고, 시스템의 재시작 안내에 따라 다시 연 후 재시도하세요.",
    "open": "시스템 오디오 권한 열기",
    "failed": "시스템 설정을 열 수 없습니다. 개인정보 보호 및 보안 → 화면 및 시스템 오디오 녹음을 직접 열어 주세요.",
    "guide": "macOS 14.2 이상에서는 시스템 오디오 녹음만 허용할 수 있습니다. 기존 화면 녹음 권한이 있으면 다시 허용할 필요가 없습니다. 이전 macOS에서는 화면 및 시스템 오디오 녹음 권한이 필요합니다. Mimi는 오디오만 캡처하고 화면 이미지를 저장하지 않습니다."
  },
  "fr": {
    "denied": "L’autorisation d’enregistrer l’audio du système manque. Autorisez Mimi dans Réglages Système → Confidentialité et sécurité → Enregistrement de l’écran et de l’audio du système, puis suivez les instructions de redémarrage et réessayez.",
    "open": "Ouvrir les autorisations audio du système",
    "failed": "Impossible d’ouvrir Réglages Système. Ouvrez manuellement Confidentialité et sécurité → Enregistrement de l’écran et de l’audio du système.",
    "guide": "Sous macOS 14.2 ou ultérieur, utilisez « Enregistrement de l’audio du système uniquement ». Une autorisation d’enregistrement de l’écran existante fonctionne sans nouvelle demande. Les versions antérieures de macOS exigent l’enregistrement de l’écran et de l’audio du système. Mimi capture l’audio sans enregistrer d’images de l’écran."
  }
} as const;

export function systemAudioPermissionCopy(language = effectiveUiLanguage()) { return copy[language]; }

/** Exact native label only; raw OS/provider errors remain hidden. */
export function isSystemAudioPermissionDenied(message: string) {
  return message === "System audio capture permission was denied.";
}
