import type { ResolvedUiLanguage } from "./i18n";
import type { AudioRecordingStatus } from "./types";

type VisibleRecordingStatus = Exclude<AudioRecordingStatus, "off">;
interface RecordingStatusText {
  short: string;
  label: string;
}

export const recordingStatusCopy: Record<ResolvedUiLanguage, Record<VisibleRecordingStatus, RecordingStatusText>> = {
  zh: {
    starting: { short: "录音准备", label: "正在准备录音" },
    recording: { short: "录音", label: "正在录音" },
    paused: { short: "录音暂停", label: "录音已暂停" },
    limited: { short: "录音已满", label: "录音已达到保存上限" },
    error: { short: "录音异常", label: "录音出现错误" },
  },
  "zh-TW": {
    starting: { short: "錄音準備", label: "正在準備錄音" },
    recording: { short: "錄音", label: "正在錄音" },
    paused: { short: "錄音暫停", label: "錄音已暫停" },
    limited: { short: "錄音已滿", label: "錄音已達儲存上限" },
    error: { short: "錄音異常", label: "錄音發生錯誤" },
  },
  en: {
    starting: { short: "Preparing", label: "Preparing audio recording" },
    recording: { short: "Recording", label: "Recording audio" },
    paused: { short: "Rec. paused", label: "Audio recording paused" },
    limited: { short: "Record limit", label: "Audio recording reached its storage limit" },
    error: { short: "Record error", label: "Audio recording error" },
  },
  ja: {
    starting: { short: "録音準備", label: "録音を準備中" },
    recording: { short: "録音", label: "録音中" },
    paused: { short: "録音一時停止", label: "録音を一時停止中" },
    limited: { short: "録音上限", label: "録音が保存上限に達しました" },
    error: { short: "録音エラー", label: "録音でエラーが発生しました" },
  },
  ko: {
    starting: { short: "녹음 준비", label: "녹음을 준비하는 중" },
    recording: { short: "녹음", label: "녹음 중" },
    paused: { short: "녹음 일시 정지", label: "녹음 일시 정지됨" },
    limited: { short: "녹음 한도", label: "녹음 저장 한도에 도달했습니다" },
    error: { short: "녹음 오류", label: "녹음 오류가 발생했습니다" },
  },
  de: {
    starting: { short: "Vorbereitung", label: "Audioaufnahme wird vorbereitet" },
    recording: { short: "Aufnahme", label: "Audioaufnahme läuft" },
    paused: { short: "Aufn. pausiert", label: "Audioaufnahme pausiert" },
    limited: { short: "Aufnahmelimit", label: "Speichergrenze der Audioaufnahme erreicht" },
    error: { short: "Aufnahmefehler", label: "Fehler bei der Audioaufnahme" },
  },
  fr: {
    starting: { short: "Préparation", label: "Préparation de l’enregistrement audio" },
    recording: { short: "Enreg.", label: "Enregistrement audio en cours" },
    paused: { short: "Enreg. en pause", label: "Enregistrement audio en pause" },
    limited: { short: "Limite audio", label: "Limite de stockage de l’enregistrement audio atteinte" },
    error: { short: "Erreur audio", label: "Erreur d’enregistrement audio" },
  },
  th: {
    starting: { short: "เตรียมอัด", label: "กำลังเตรียมบันทึกเสียง" },
    recording: { short: "อัดเสียง", label: "กำลังบันทึกเสียง" },
    paused: { short: "พักอัดเสียง", label: "พักการบันทึกเสียงแล้ว" },
    limited: { short: "เสียงเต็ม", label: "การบันทึกเสียงถึงขีดจำกัดพื้นที่แล้ว" },
    error: { short: "อัดเสียงผิดพลาด", label: "เกิดข้อผิดพลาดในการบันทึกเสียง" },
  },
  ru: {
    starting: { short: "Подготовка", label: "Подготовка к записи звука" },
    recording: { short: "Запись", label: "Идёт запись звука" },
    paused: { short: "Запись на паузе", label: "Запись звука приостановлена" },
    limited: { short: "Лимит записи", label: "Достигнут лимит хранения звукозаписи" },
    error: { short: "Ошибка записи", label: "Ошибка записи звука" },
  },
};

/** Include this in a parent button's explicit accessible label as well. */
export function recordingStatusLabel(status: AudioRecordingStatus | null | undefined, language: ResolvedUiLanguage): string | null {
  return !status || status === "off" ? null : recordingStatusCopy[language][status].label;
}
