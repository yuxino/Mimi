import { effectiveUiLanguage, I18N } from "../../lib/i18n";
import type { ServiceProfile, XAISpeechMode } from "../../lib/types";
import { SettingsHelp } from "./SettingsHelp";
import { SettingsSelect } from "./SettingsPrimitives";
import { useSettingsToast } from "./useSettingsToast";

const copy = {
  zh: { mode: "语音模式", asr: "流式语音识别（ASR）", help: "xAI 识别语音后，可单独选择文字翻译服务。Grok Voice 在检测到说话停顿后分段翻译。停止字幕后可切换。", failed: "语音模式保存失败，请重试。" },
  "zh-TW": { mode: "語音模式", asr: "串流語音辨識（ASR）", help: "xAI 辨識語音後，可單獨選擇文字翻譯服務。Grok Voice 偵測到說話停頓後分段翻譯。停止字幕後可切換。", failed: "語音模式儲存失敗，請重試。" },
  en: { mode: "Speech mode", asr: "Streaming speech recognition (ASR)", help: "Recognize speech with xAI, then choose text translation separately. Grok Voice translates in segments after pauses. Stop subtitles to switch.", failed: "Could not save the speech mode. Try again." },
  ja: { mode: "音声モード", asr: "ストリーミング音声認識（ASR）", help: "xAI で音声を認識し、テキスト翻訳サービスを別に選べます。Grok Voice は発話の切れ目ごとに翻訳します。字幕を停止して切り替えてください。", failed: "音声モードを保存できませんでした。再試行してください。" },
  de: { mode: "Sprachmodus", asr: "Streaming-Spracherkennung (ASR)", help: "Sprache mit xAI erkennen und den Textübersetzungsdienst separat wählen. Grok Voice übersetzt nach Sprechpausen abschnittsweise. Zum Wechseln Untertitel stoppen.", failed: "Sprachmodus konnte nicht gespeichert werden. Bitte erneut versuchen." },
  ko: { mode: "음성 모드", asr: "스트리밍 음성 인식 (ASR)", help: "xAI로 음성을 인식하고 텍스트 번역 서비스를 별도로 선택할 수 있습니다. Grok Voice는 말이 멈춘 뒤 구간별로 번역합니다. 자막을 중지한 후 변경하세요.", failed: "음성 모드를 저장하지 못했습니다. 다시 시도하세요." },
  fr: { mode: "Mode vocal", asr: "Reconnaissance vocale en flux (ASR)", help: "Reconnaissez la parole avec xAI, puis choisissez séparément la traduction de texte. Grok Voice traduit par segments après les pauses. Arrêtez les sous-titres pour changer.", failed: "Impossible d’enregistrer le mode vocal. Réessayez." },
  th: { mode: "โหมดเสียง", asr: "รู้จำเสียงแบบสตรีม (ASR)", help: "ใช้ xAI รู้จำเสียงและเลือกบริการแปลข้อความแยกกันได้ Grok Voice แปลเป็นช่วงหลังหยุดพูด หยุดคำบรรยายก่อนเปลี่ยนโหมด", failed: "บันทึกโหมดเสียงไม่สำเร็จ ลองอีกครั้ง" },
  ru: { mode: "Режим речи", asr: "Потоковое распознавание речи (ASR)", help: "xAI распознаёт речь, а сервис перевода текста выбирается отдельно. Grok Voice переводит сегменты после пауз в речи. Остановите субтитры, чтобы сменить режим.", failed: "Не удалось сохранить режим речи. Повторите попытку." },
};

export function XAISpeechSettings({ profile, disabled, onSave }: {
  profile: ServiceProfile;
  disabled: boolean;
  onSave: (options: { xaiSpeechMode: XAISpeechMode }) => Promise<unknown>;
}) {
  const strings = copy[effectiveUiLanguage()];
  const { runWithToast } = useSettingsToast();
  const mode = profile.xaiSpeechMode ?? "voice";
  return <section className="service-stage xai-speech-settings">
    <div className="settings-field service-stage__selector">
      <span className="service-stage__name-help">{strings.mode}<SettingsHelp text={strings.help} label={I18N.settings.helpLabel} /></span>
      <SettingsSelect label={strings.mode} value={mode} disabled={disabled}
        options={[{ value: "asrMt", label: strings.asr }, { value: "voice", label: "Grok Voice" }]}
        onChange={value => { if (value !== mode) void runWithToast(() => onSave({ xaiSpeechMode: value as XAISpeechMode }), strings.failed); }} />
    </div>
  </section>;
}
