<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>為正在播放的影片加上即時翻譯字幕。</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="最新版本"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="總下載量"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="main 分支 CI 狀態"></a>
    <a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT 授權條款"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="../../android/README.zh-CN.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p><a href="https://github.com/yuxino/mimi/releases/latest"><strong>下載 Mimi</strong></a> · <a href="https://mimi.yuxino.cn/zh-tw/">官網</a> · <a href="../usage.zh-TW.md">使用指南</a></p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ja.md">日本語</a> · <a href="th.md">ภาษาไทย</a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

![Mimi 雙語字幕示意：原創插畫與範例對白](../assets/readme-preview.png)

字幕顯示示意，使用範例對白。[觀看字幕與沉浸模式示範（舊版介面）](../demos/README.md)。

## 還能做什麼

- **雙語對照**：看外語影片、直播和網課時，可顯示雙語、僅譯文或僅原文。
- **沉浸觀看**：移動字幕並調整字級、顏色。沉浸模式只留下文字，點擊可穿透到下方應用程式。
- **選擇音源**：桌面版在 macOS / Windows 11 可指定應用程式，Linux 可指定播放串流。
- **儲存內容**：桌面版開啟字幕儲存或錄音後，可匯出 TXT / Markdown 字幕或 WAV 音訊。兩項功能預設關閉。

## 開始使用

Mimi 免費開源，不收會員費。雲端服務需使用自己的 API 金鑰，可能收費。音訊或辨識文字會傳送給你選擇的服務。

符合條件的 Apple 晶片 Mac（macOS 26+）可在下載語言資源後，使用[不需 API 金鑰的 Apple 本機服務](../provider-setup.zh-CN.md#apple-speech)。

1. **下載並安裝**：在[下載頁](https://github.com/yuxino/mimi/releases/latest)選擇安裝檔：macOS 13+（Apple 晶片 / Intel）用 DMG、Windows x64 用 EXE、Linux x86_64 用 AppImage / DEB。Android 10+ 手機見[安裝與使用（簡體中文）](../../android/README.zh-CN.md#start)。
2. **設定服務**：在「設定 → 模型與服務」新增 OpenAI、Gemini、xAI 或其他支援的服務，填入自己的 API 金鑰等資料後儲存。取得方式與填寫內容見[設定指南（簡體中文）](../provider-setup.zh-CN.md#choose)。
3. **選擇聲音和語言**：在「聲音與語言」選擇辨識語言和翻譯語言。預設擷取電腦聲音；要翻譯自己的話時請選麥克風，也可同時選擇兩路音源。
4. **開啟字幕**：在「字幕」頁開啟「即時字幕」，依提示允許所需權限，再播放有人聲的影片、直播等內容。

## 使用中遇到問題？

[使用指南](../usage.zh-TW.md)包含安裝、權限、字幕顯示和儲存記錄。沒有字幕或連線失敗，先看[辨識錯誤](../usage.zh-TW.md#errors)；仍未解決可[回報問題](https://github.com/yuxino/mimi/issues)。

<details>
<summary>貢獻者與社群</summary>

## 貢獻者

想幫 Mimi 改進？從[貢獻指南（中英）](../../.github/CONTRIBUTING.md)開始。

感謝每一位寫程式碼、提問題、試用和分享的朋友 (๑•̀ㅂ•́)و✧

特別感謝 [@yebuwudong](https://github.com/yebuwudong) 貢獻 [Android 版](https://github.com/yuxino/mimi/pull/37)，以及 [@LLLin000](https://github.com/LLLin000) 貢獻[字幕動畫](https://github.com/yuxino/mimi/pull/67)和 [Windows 音訊來源改進](https://github.com/yuxino/mimi/pull/89)。

也感謝 [@Chtholly000](https://github.com/Chtholly000) 改進 [Gemini 連續字幕和連線輪換](https://github.com/yuxino/Mimi/pull/176)。

感謝 [@EmmetZ](https://github.com/EmmetZ) 修復 [Wayland 浮窗與控制元件不同步的問題](https://github.com/yuxino/Mimi/pull/242)，並新增 [Linux 個別播放串流擷取](https://github.com/yuxino/Mimi/pull/275)和[播放串流應用程式圖示](https://github.com/yuxino/Mimi/pull/304)。

感謝 [@wangmian0](https://github.com/wangmian0) 為 [DeepSeek 即時字幕翻譯關閉思考模式](https://github.com/yuxino/Mimi/pull/270)。

感謝 [@hubumulity](https://github.com/hubumulity) 貢獻[字幕顯示則數設定](https://github.com/yuxino/Mimi/pull/303)。

感謝 [@ryannyui](https://github.com/ryannyui) 貢獻 [Linux AppImage 圖形相容性修復與工具鏈升級](https://github.com/yuxino/Mimi/pull/312)。

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

[檢視所有貢獻者](https://github.com/yuxino/mimi/graphs/contributors)

## 社群致謝

也感謝 [V2EX](https://www.v2ex.com/)、[LINUX DO](https://linux.do/)、[小眾軟體](https://meta.appinn.net/)、[NodeLoc](https://www.nodeloc.com/)、[Solo](https://solo.xin/)、[新趣集](https://xinquji.com/posts/859305)和[電鴨](https://eleduck.com/)社群朋友的試用、意見回饋與分享。

</details>

[MIT](../../LICENSE) © 2026 yuxino
