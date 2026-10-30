<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>재생 중인 영상에 실시간 번역 자막을 더합니다.</p>
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
  <p><a href="https://github.com/yuxino/mimi/releases/latest"><strong>Mimi 다운로드</strong></a> · <a href="https://mimi.yuxino.cn/ko/">웹사이트</a> · <a href="../usage.ko.md">사용 안내</a></p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ja.md">日本語</a> · <a href="th.md">ภาษาไทย</a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

![예시 대화를 사용한 Mimi 이중 언어 자막 이미지](../assets/readme-preview.png)

예시 대화로 만든 화면 이미지입니다. [자막과 몰입 모드 데모 보기(이전 UI)](../demos/README.md).

## 주요 기능

- **두 언어로 보기**: 외국어 영상, 방송, 온라인 수업에서 두 언어, 번역문만, 원문만 표시할 수 있습니다.
- **자막에 집중하기**: 자막 위치, 글자 크기, 색상을 바꿀 수 있습니다. 몰입 모드에서는 자막만 남기고 뒤에 있는 앱을 클릭할 수 있습니다.
- **소리 선택하기**: 데스크톱의 macOS / Windows 11에서는 앱을, Linux에서는 재생 스트림을 선택할 수 있습니다.
- **나중에 다시 보기**: 데스크톱에서 자막 저장이나 녹음을 켜면 TXT / Markdown 자막 또는 WAV 오디오로 내보낼 수 있습니다. 두 기능 모두 기본적으로 꺼져 있습니다.

## 시작하기

Mimi는 무료 오픈 소스 앱이며, 별도 구독료가 없습니다. 클라우드 서비스에는 본인의 API 키 등이 필요하며 요금이 발생할 수 있습니다. 오디오 또는 인식된 텍스트가 선택한 서비스로 전송됩니다.

지원되는 Apple silicon Mac(macOS 26 이상)에서는 언어 리소스를 내려받은 뒤 [Apple 로컬 서비스를 API 키 없이](../provider-setup.md#apple-speech) 사용할 수 있습니다.

1. **다운로드 및 설치**: [다운로드 페이지](https://github.com/yuxino/mimi/releases/latest)에서 macOS 13 이상(Apple silicon / Intel)은 DMG, Windows x64는 EXE, Linux x86_64는 AppImage / DEB를 받으세요. Android 10 이상 휴대폰은 [Android 안내(영어)](../../android/README.md#start)를 참고하세요.
2. **서비스 설정**: 설정 → 모델 및 서비스에서 OpenAI, Gemini, xAI 등 지원되는 서비스를 추가하세요. 본인의 API 키 등을 입력하고 저장합니다. 발급 방법과 입력할 항목은 [설정 안내(영어)](../provider-setup.md#choose)에 있습니다.
3. **소리와 언어 선택**: 오디오 및 언어 설정에서 인식 언어와 번역 언어를 정하세요. 컴퓨터 소리가 기본으로 선택됩니다. 자신의 말을 번역하려면 마이크를, 두 음원을 쓰려면 두 입력을 모두 선택하세요.
4. **자막 시작**: 자막 페이지에서 실시간 자막을 켜고 필요한 권한을 허용한 뒤, 말소리가 있는 동영상이나 방송을 재생합니다.

## 도움이 필요한가요?

[사용 안내](../usage.ko.md)에서 설치, 권한, 자막 표시, 저장을 설명합니다. 자막이 나오지 않거나 연결에 실패하면 [인식 오류](../usage.ko.md#errors)를 먼저 확인하고, 해결되지 않으면 [문제를 알려 주세요](https://github.com/yuxino/mimi/issues).

<details>
<summary>기여자와 커뮤니티</summary>

## 기여자

참여하려면 [기여 안내(영어·중국어)](../../.github/CONTRIBUTING.md)를 확인하세요.

코드를 작성하고, 문제를 알려 주고, Mimi를 사용하거나 소개해 주시는 모든 분께 감사드립니다 (๑•̀ㅂ•́)و✧

[Android 앱](https://github.com/yuxino/mimi/pull/37)을 만들어 주신 [@yebuwudong](https://github.com/yebuwudong), [자막 애니메이션](https://github.com/yuxino/mimi/pull/67)과 [Windows 오디오 개선](https://github.com/yuxino/mimi/pull/89)에 기여해 주신 [@LLLin000](https://github.com/LLLin000)께 특별히 감사드립니다.

[Gemini 연속 자막과 계획된 연결 교체](https://github.com/yuxino/Mimi/pull/176)를 개선해 주신 [@Chtholly000](https://github.com/Chtholly000)께도 감사드립니다.

[Wayland 자막 컨트롤 수정](https://github.com/yuxino/Mimi/pull/242), [Linux 개별 재생 스트림 캡처](https://github.com/yuxino/Mimi/pull/275), [재생 스트림의 앱 아이콘 표시](https://github.com/yuxino/Mimi/pull/304)를 기여해 주신 [@EmmetZ](https://github.com/EmmetZ)께 감사드립니다.

[DeepSeek 실시간 자막 번역의 사고 모드를 비활성화](https://github.com/yuxino/Mimi/pull/270)해 주신 [@wangmian0](https://github.com/wangmian0)께 감사드립니다.

[자막 표시 개수 설정](https://github.com/yuxino/Mimi/pull/303)을 기여해 주신 [@hubumulity](https://github.com/hubumulity)께 감사드립니다.

[Linux AppImage 그래픽 호환성 수정과 도구 체인 업그레이드](https://github.com/yuxino/Mimi/pull/312)에 기여해 주신 [@ryannyui](https://github.com/ryannyui)께 감사드립니다.

<p>
  <a href="https://github.com/yuxino"><img src="../assets/contributors/yuxino.svg" width="40" height="40" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="../assets/contributors/LLLin000.svg" width="40" height="40" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="../assets/contributors/yebuwudong.svg" width="40" height="40" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="../assets/contributors/Chtholly000.svg" width="40" height="40" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="../assets/contributors/inhome.svg" width="40" height="40" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="../assets/contributors/EmmetZ.svg" width="40" height="40" alt="@EmmetZ"></a>
  <a href="https://github.com/wangmian0"><img src="../assets/contributors/wangmian0.svg" width="40" height="40" alt="@wangmian0"></a>
  <a href="https://github.com/hubumulity"><img src="../assets/contributors/hubumulity.svg" width="40" height="40" alt="@hubumulity"></a>
  <a href="https://github.com/ryannyui"><img src="../assets/contributors/ryannyui.svg" width="40" height="40" alt="@ryannyui"></a>
</p>

[모든 기여자](https://github.com/yuxino/mimi/graphs/contributors)

## 커뮤니티

[V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305), [Eleduck](https://eleduck.com/)에서 Mimi를 사용하고 의견을 나누며 소개해 주신 분들께 감사드립니다.

</details>

[MIT](../../LICENSE) © 2026 yuxino
