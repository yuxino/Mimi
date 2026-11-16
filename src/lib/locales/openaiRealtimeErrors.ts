import { effectiveUiLanguage, type ResolvedUiLanguage } from "../i18n";

const copy = {
  "en": {
    openai_realtime_session_expired: "OpenAI Realtime session expired.",
    openai_realtime_permission_denied: "OpenAI denied access. Check project permissions and region.",
    openai_realtime_quota_exhausted: "OpenAI quota or credit balance is exhausted. Check billing and usage.",
    openai_realtime_rate_limited: "OpenAI has limited requests. Wait and check the usage limits.",
    openai_realtime_model_unavailable: "The requested OpenAI model or endpoint is unavailable.",
    openai_realtime_invalid_request: "OpenAI rejected the session parameters.",
    openai_realtime_service_error: "OpenAI reported a service error. Try again shortly.",
  },
  "zh": {
    openai_realtime_session_expired: "OpenAI 实时会话已到期。",
    openai_realtime_permission_denied: "OpenAI 拒绝了访问。请检查项目权限和地区。",
    openai_realtime_quota_exhausted: "OpenAI 额度或余额已用完。请检查计费和用量。",
    openai_realtime_rate_limited: "OpenAI 限制了请求。请稍后重试并查看用量限制。",
    openai_realtime_model_unavailable: "所请求的 OpenAI 模型或接口不可用。",
    openai_realtime_invalid_request: "OpenAI 拒绝了会话参数。",
    openai_realtime_service_error: "OpenAI 服务发生错误，请稍后重试。",
  },
  "zh-TW": {
    openai_realtime_session_expired: "OpenAI 即時工作階段已到期。",
    openai_realtime_permission_denied: "OpenAI 拒絕存取。請檢查專案權限和地區。",
    openai_realtime_quota_exhausted: "OpenAI 額度或餘額已用完。請檢查帳務和用量。",
    openai_realtime_rate_limited: "OpenAI 限制了請求。請稍後重試並查看用量限制。",
    openai_realtime_model_unavailable: "所請求的 OpenAI 模型或端點無法使用。",
    openai_realtime_invalid_request: "OpenAI 拒絕了工作階段參數。",
    openai_realtime_service_error: "OpenAI 服務發生錯誤，請稍後重試。",
  },
  "ja": {
    openai_realtime_session_expired: "OpenAI のリアルタイムセッションの有効期限が切れました。",
    openai_realtime_permission_denied: "OpenAI がアクセスを拒否しました。プロジェクトの権限と地域を確認してください。",
    openai_realtime_quota_exhausted: "OpenAI の利用枠または残高を使い切りました。請求情報と使用量を確認してください。",
    openai_realtime_rate_limited: "OpenAI がリクエストを制限しました。しばらく待ってから利用制限を確認してください。",
    openai_realtime_model_unavailable: "指定した OpenAI モデルまたはエンドポイントは利用できません。",
    openai_realtime_invalid_request: "OpenAI がセッションの設定を拒否しました。",
    openai_realtime_service_error: "OpenAI のサービスでエラーが発生しました。しばらくしてから再試行してください。",
  },
  "de": {
    openai_realtime_session_expired: "Die OpenAI-Echtzeitsitzung ist abgelaufen.",
    openai_realtime_permission_denied: "OpenAI hat den Zugriff verweigert. Prüfe Projektberechtigungen und Region.",
    openai_realtime_quota_exhausted: "Das OpenAI-Kontingent oder Guthaben ist aufgebraucht. Prüfe Abrechnung und Nutzung.",
    openai_realtime_rate_limited: "OpenAI hat Anfragen begrenzt. Warte kurz und prüfe die Nutzungslimits.",
    openai_realtime_model_unavailable: "Das angeforderte OpenAI-Modell oder der Endpunkt ist nicht verfügbar.",
    openai_realtime_invalid_request: "OpenAI hat die Sitzungsparameter abgelehnt.",
    openai_realtime_service_error: "Bei OpenAI ist ein Dienstfehler aufgetreten. Versuche es in Kürze erneut.",
  },
  "fr": {
    openai_realtime_session_expired: "La session OpenAI en temps réel a expiré.",
    openai_realtime_permission_denied: "OpenAI a refusé l’accès. Vérifiez les autorisations du projet et la région.",
    openai_realtime_quota_exhausted: "Le quota ou le crédit OpenAI est épuisé. Vérifiez la facturation et l’utilisation.",
    openai_realtime_rate_limited: "OpenAI a limité les requêtes. Patientez et vérifiez les limites d’utilisation.",
    openai_realtime_model_unavailable: "Le modèle ou le point de terminaison OpenAI demandé est indisponible.",
    openai_realtime_invalid_request: "OpenAI a refusé les paramètres de la session.",
    openai_realtime_service_error: "OpenAI a signalé une erreur de service. Réessayez dans un instant.",
  },
  "ko": {
    openai_realtime_session_expired: "OpenAI 실시간 세션이 만료되었습니다.",
    openai_realtime_permission_denied: "OpenAI에서 액세스를 거부했습니다. 프로젝트 권한과 지역을 확인하세요.",
    openai_realtime_quota_exhausted: "OpenAI 할당량이나 잔액이 소진되었습니다. 결제와 사용량을 확인하세요.",
    openai_realtime_rate_limited: "OpenAI에서 요청을 제한했습니다. 잠시 기다린 뒤 사용량 한도를 확인하세요.",
    openai_realtime_model_unavailable: "요청한 OpenAI 모델 또는 엔드포인트를 사용할 수 없습니다.",
    openai_realtime_invalid_request: "OpenAI에서 세션 매개변수를 거부했습니다.",
    openai_realtime_service_error: "OpenAI 서비스에 오류가 발생했습니다. 잠시 후 다시 시도하세요.",
  },
  "th": {
    openai_realtime_session_expired: "เซสชันเรียลไทม์ของ OpenAI หมดอายุแล้ว",
    openai_realtime_permission_denied: "OpenAI ปฏิเสธการเข้าถึง โปรดตรวจสอบสิทธิ์ของโปรเจกต์และภูมิภาค",
    openai_realtime_quota_exhausted: "โควตาหรือเครดิต OpenAI หมดแล้ว โปรดตรวจสอบการเรียกเก็บเงินและการใช้งาน",
    openai_realtime_rate_limited: "OpenAI จำกัดคำขอ โปรดรอสักครู่และตรวจสอบขีดจำกัดการใช้งาน",
    openai_realtime_model_unavailable: "ไม่สามารถใช้โมเดลหรือปลายทางของ OpenAI ที่ขอได้",
    openai_realtime_invalid_request: "OpenAI ปฏิเสธพารามิเตอร์ของเซสชัน",
    openai_realtime_service_error: "เกิดข้อผิดพลาดในบริการ OpenAI โปรดลองอีกครั้งในอีกสักครู่",
  },
  "ru": {
    openai_realtime_session_expired: "Сеанс OpenAI в реальном времени истёк.",
    openai_realtime_permission_denied: "OpenAI отказал в доступе. Проверьте права проекта и регион.",
    openai_realtime_quota_exhausted: "Квота или баланс OpenAI исчерпаны. Проверьте оплату и использование.",
    openai_realtime_rate_limited: "OpenAI ограничил запросы. Подождите и проверьте лимиты использования.",
    openai_realtime_model_unavailable: "Запрошенная модель или точка подключения OpenAI недоступна.",
    openai_realtime_invalid_request: "OpenAI отклонил параметры сеанса.",
    openai_realtime_service_error: "Ошибка сервиса OpenAI. Повторите попытку чуть позже.",
  },
} satisfies Record<ResolvedUiLanguage, Record<string, string>>;

type OpenAIRealtimeError = keyof typeof copy.en;

export function openAIRealtimeErrorMessage(error: string, language = effectiveUiLanguage()): string | null {
  const label = openAIRealtimeErrorCode(error);
  return label ? copy[language][label] : null;
}

export function openAIRealtimeErrorCode(error: string): OpenAIRealtimeError | undefined {
  if (error === "openai_realtime_setup_expired") return "openai_realtime_session_expired";
  if (error === "openai_realtime_service_unavailable") return "openai_realtime_service_error";
  return Object.hasOwn(copy.en, error) ? error as OpenAIRealtimeError : undefined;
}
