import type { DEVELOPMENT as EnglishCopy } from "./en-development";

export const DEVELOPMENT = {
    title: "개발 디버거", start: "추적 시작", evidence: "오디오와 자막 기록", stop: "추적 중지", export: "사례 내보내기",
    help: "개발 전용입니다. 추적에는 콘텐츠를 포함하지 않습니다. 기록을 켜면 선택한 입력의 전송 오디오와 자막 스냅샷을 비공개 로컬 파일에 저장합니다. 재시작하면 꺼집니다.",
    live: "현재 인식 및 번역", timeline: "이벤트 타임라인", all: "모든 단계", provider: "서비스 이벤트", reduced: "자막 상태 반영", frontend: "프런트엔드", pipeline: "오디오 및 요청", snapshot: "스냅샷", loss: "누락된 증거", audio: "서비스로 전송한 오디오", listen: "오디오 불러오기",
    audioHelp: "로컬 소켓 전송이 완료된 실제 오디오 메시지에서 PCM을 디코딩합니다. 서버의 수신이나 처리를 증명하지는 않습니다. 실패, 취소와 증거 누락을 확인하세요. 다시 캡처되지 않도록 재생 전에 자막을 중지하세요.",
    system: "시스템 오디오", microphone: "마이크", stopFirst: "재생하려면 자막 중지", replay: "자막 스냅샷 재생", previous: "이전", next: "다음", raw: "원문", translated: "번역", empty: "아직 기록 없음", idle: "중지됨", active: "추적 중", saved: "사례를 내보냈습니다", failed: "작업에 실패했습니다. 다시 시도하세요.", details: "이벤트 필드", workspace: "작업 공간", route: "캡처된 구성",
    recordHelp: "오디오와 자막 증거 기록은 기본적으로 꺼져 있으며 비공개 내용을 포함합니다. 자동으로 공유하지 않습니다. 기록할 때마다 별도 파일을 만들며 이전 사례는 유지합니다.",
    replayHelp: "오프라인 재생은 동일한 자막 표시 로직과 레이아웃을 사용합니다. 단계별 재생은 원래 안정화 타이밍을 재현하지 않습니다. 원래 확정 시점과 잘림은 추적에서 확인하세요.",
    final: "확정된 자막", clear: "지우기", pause: "일시 정지됨", pending: "번역 대기 중", events: "이벤트", noSubtitles: "아직 인식 또는 번역 없음",
  } satisfies typeof EnglishCopy;
