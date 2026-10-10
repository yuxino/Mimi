<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>macOS 13 이상(Apple silicon 및 Intel)과 Windows / Linux x86_64에서 시스템 오디오 또는 마이크를 실시간 자막으로 보고 번역하세요.</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="최신 릴리스"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="전체 다운로드"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="main 브랜치 CI 상태"></a>
    <a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT 라이선스"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="../../android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ja.md">日本語</a> · <a href="th.md">ภาษาไทย</a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

<p align="center">Mimi는 컴퓨터나 마이크의 음성을 실시간 번역 자막으로 보여 줍니다. 영화, 방송, 강의를 화면 위에 떠 있는 자막과 함께 즐기세요.</p>

![자체 제작 일러스트 장면 위에 표시된 Mimi의 원문·번역 자막 창](../assets/readme-preview.png)

## 기능

- 시스템 오디오, 마이크 또는 두 입력 모두를 선택할 수 있습니다. 시스템 오디오가 기본이며 마이크는 직접 선택해야 캡처합니다.
- macOS와 Windows 11에서는 선택한 앱의 오디오를 번역할 수 있습니다.
- 원문, 번역 또는 둘 다 표시할 수 있습니다.
- 자막의 위치, 크기와 색상을 조정하거나 클릭이 아래 창으로 전달되도록 설정할 수 있습니다.
- 필요할 때 자막이나 오디오를 로컬에 저장하고 TXT / WAV로 내보낼 수 있습니다. 저장과 녹음은 기본적으로 꺼져 있습니다.

## 시작하기

처음 설정한다면 **Alibaba Cloud나 Google Gemini로 시작하는 것을 권장합니다**. 실제 사용 테스트는 Alibaba Cloud로 더 많이 진행했습니다. 지금까지 제 경험으로는 Gemini의 자막 출력이 가장 일관적이었습니다.

Google Gemini는 안정적인 네트워크 연결을 사용하세요. [Google AI Studio](https://aistudio.google.com/)에서 사용 가능량, 결제 정보와 API 키를 확인하세요.

서비스 활성화 링크, 인증 정보 안내와 Mimi가 현재 사용하는 모델은 **[서비스 설정 안내](../provider-setup.md)**를 참고하세요.

1. 설정 → 음성 및 번역에서 구성을 추가하고 필요한 서비스 인증 정보를 입력한 다음 저장하세요.
2. 인식 언어와 번역 언어를 선택하세요.
3. 오디오를 재생하고 자막에서 실시간 자막을 켜세요. macOS 14.2 이상에서는 모든 앱이나 선택한 앱 모두 시스템 오디오 녹음만 허용하면 됩니다. 기존 화면 녹음 권한도 계속 사용할 수 있습니다. 이전 macOS에서는 화면 및 시스템 오디오 녹음 권한이 필요합니다.

클라우드 음성 서비스에는 본인의 인증 정보가 필요하며 오디오가 해당 서비스로 전송됩니다. 사용 요금이 발생할 수 있습니다. Apple Speech는 지원되는 Mac에서 오디오를 로컬로 인식합니다. 원격 텍스트 번역은 인식한 텍스트를 선택한 서비스로 전송하며, Apple Translation은 텍스트를 Mac 안에서 처리합니다.

[설정 및 도움말](../usage.md) · [Android](../../android/README.md) · [버그 신고](https://github.com/yuxino/mimi/issues) · [기여 안내](../../.github/CONTRIBUTING.md)

<a id="apple-local-recognition"></a>

### Apple 로컬 인식

**Apple Speech**는 Apple silicon, macOS 26 이상, 사용 가능한 시스템 전사 기능 등 시스템 조건이 충족되면 표시됩니다. 음성 API 키는 필요하지 않습니다. 자막을 중지하고 **인식 언어**에서 언어를 선택하세요. 언어 팩이 없다면 **다운로드 후 사용**을 누르세요. Mimi가 Apple 리소스를 다운로드하고 준비되면 해당 언어를 선택합니다. 이미 다운로드한 언어라면 **인식 언어로 설정**을 누른 다음 자막을 시작하세요. 시스템 설정을 따로 열 필요는 없습니다. 언어 자동 감지는 제공하지 않습니다. [Apple Speech 설정](../provider-setup.md#apple-speech)을 참고하세요.

**Apple Translation**은 macOS 26 이상이 설치된 지원 대상 Apple silicon Mac에서 사용하는 별도의 기기 내 텍스트 번역 서비스입니다. **텍스트 번역**에서 선택하고 구성을 저장한 다음 입력 언어와 번역 언어를 직접 지정하세요. Mimi가 언어 조합의 준비 상태를 표시합니다. **언어 다운로드 또는 허용**을 누르면 Apple 설정 확인 창이 열립니다. 번역 모델은 음성 모델과 별개입니다. 기존 모델은 재사용하며, 확인하면 없는 모델만 Apple에서 다운로드합니다. 연결 확인이나 자막 세션에서는 다운로드를 요청하지 않습니다. 텍스트 API 키나 텍스트 프록시는 필요하지 않습니다.

Apple Speech로 원문만 표시하려면 **번역 안 함 (원문만)**을 선택하세요. [Index-Translate](#try-index-translate)와 같은 원격 텍스트 번역 서비스도 사용할 수 있으며, 인식한 텍스트가 해당 서비스로 전송됩니다.

<a id="try-index-translate"></a>

### Index-Translate 사용해 보기

Bilibili의 [Index-Translate](https://github.com/bilibili/Index-Translate#inference)는 현재 무료 공개 번역 API를 제공합니다(2026년 10월 5일 기준). Alibaba Cloud 또는 Apple Speech로 인식한 텍스트를 번역하는 데 사용할 수 있습니다.

**설정 → 음성 및 번역**에서 **Alibaba Cloud** 또는 **Apple Speech** 구성을 열고 **텍스트 번역** 아래의 **OpenAI 호환 API**를 선택하세요. [공식 예제](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py#L40-L41)에 나온 값을 사용하세요.

| 항목 | 값 |
| --- | --- |
| 서비스 주소 | `https://index-translate.bilibili.com/v1` |
| 모델 이름 | `Index-Translate-35B-A3B` |
| API 키 | 비워 두세요. 현재 공개 API는 인증을 요구하지 않습니다. |

저장한 다음 **텍스트 번역** 옆에서 연결을 확인하세요. 이 주소에 이미 저장된 키가 있다면 **번역 키 제거**를 선택하고 다시 저장하세요.

Index-Translate는 텍스트 번역만 처리합니다. Alibaba Cloud를 사용할 때는 음성 인식 인증 정보를 유지해야 하며 인식 요금이 발생할 수 있습니다. Apple Speech 인식에는 키가 필요하지 않습니다. 무료 API의 제공 여부는 해당 서비스에 따라 달라집니다.

## 자주 묻는 질문

**macOS에서 녹음 권한을 허용했는데도 계속 권한을 요청하나요?** Mimi를 종료한 다음 시스템 설정 → 개인정보 보호 및 보안 → 화면 및 시스템 오디오 기록에서 Mimi 항목만 제거하고 다시 추가하세요. 정식 앱은 `/Applications/mimi.app`, 개발 앱은 `/Applications/mimi-dev.app`을 사용하세요. 권한을 켠 다음 같은 앱을 다시 여세요. [권한 복구 안내](../usage.md#macos-permissions-after-an-update)를 참고하세요.

## 기여자

코드를 작성하고, 문제를 알려 주고, Mimi를 사용하거나 소개해 주시는 모든 분께 감사드립니다 (๑•̀ㅂ•́)و✧

[Android 앱](https://github.com/yuxino/mimi/pull/37)을 만들어 주신 [@yebuwudong](https://github.com/yebuwudong), [자막 애니메이션](https://github.com/yuxino/mimi/pull/67)과 [Windows 오디오 개선](https://github.com/yuxino/mimi/pull/89)에 기여해 주신 [@LLLin000](https://github.com/LLLin000)께 특별히 감사드립니다.

[Gemini 연속 자막과 계획된 연결 교체](https://github.com/yuxino/Mimi/pull/176)를 개선해 주신 [@Chtholly000](https://github.com/Chtholly000)께도 감사드립니다.

[Wayland에서 자막 창을 이동할 때 컨트롤이 따라오지 않는 문제](https://github.com/yuxino/Mimi/pull/242)를 수정해 주신 [@EmmetZ](https://github.com/EmmetZ)께 감사드립니다.

<p>
  <a href="https://github.com/yuxino"><img src="../assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="../assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="../assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="../assets/contributors/Chtholly000.svg" width="64" height="64" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="../assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="../assets/contributors/EmmetZ.svg" width="64" height="64" alt="@EmmetZ"></a>
</p>

[모든 기여자](https://github.com/yuxino/mimi/graphs/contributors)

## 커뮤니티

[V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305), [Eleduck](https://eleduck.com/)에서 Mimi를 사용하고 의견을 나누며 소개해 주신 분들께 감사드립니다.

[MIT](../../LICENSE) © 2026 yuxino
