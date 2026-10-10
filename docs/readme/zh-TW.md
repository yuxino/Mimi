<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>系統聲音或麥克風即時字幕與翻譯，支援 Apple 晶片和 Intel Mac（macOS 13+），以及 Windows / Linux x86_64。</p>
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
    <a href="../../android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

<p align="center">Mimi 把電腦或麥克風中的人聲翻譯成即時字幕。看電影、直播或線上課程時，字幕會浮動顯示在螢幕上。</p>

![Mimi 雙語字幕視窗，搭配原創插畫與範例對白](../assets/readme-preview.png)

## 功能

- 選擇系統聲音、麥克風，或同時開啟兩路。預設使用系統聲音，麥克風需要主動選擇。
- 選擇要翻譯的應用程式聲音（macOS / Windows 11）。
- 顯示原文、譯文，或雙語字幕。
- 調整字幕的位置、大小和顏色，也可讓滑鼠點擊穿透字幕視窗。
- 按需儲存字幕或音訊到本機，匯出 TXT / WAV；預設不儲存、不錄音。

## 開始使用

初次使用，**推薦先用阿里雲或 Google Gemini**。我們對阿里雲做了更多實際測試；目前個人用下來，Gemini 的字幕輸出最穩定。

使用 Google Gemini 時，請保持網路連線穩定，並在 [Google AI Studio](https://aistudio.google.com/) 查看可用額度與費用，並取得 API 金鑰。

各家服務的啟用入口、認證資訊取得步驟，以及 Mimi 目前使用的模型，見[**服務開通指南**](../provider-setup.zh-CN.md)。

1. 開啟「設定 → 語音與翻譯」，新增設定，按提示填入服務供應商認證資訊並儲存。
2. 選擇辨識和翻譯語言。
3. 播放內容，在「字幕」頁開啟「即時字幕」。macOS 14.2 以上，全部應用程式或指定單一應用程式均可使用「僅系統音訊錄製」；已有螢幕錄製授權會繼續使用。更早的 macOS 需要「螢幕與系統音訊錄製」。

雲端辨識需要自備服務供應商認證資訊，並將音訊傳送至該服務，呼叫可能產生費用。符合條件的 Mac 可用 Apple Speech 在本機辨識；遠端文字翻譯會將辨識文字傳送至你選擇的服務；Apple Translation 在 Mac 本機處理文字。

[使用與常見問題](../usage.zh-CN.md) · [Android](../../android/README.md) · [回報問題](https://github.com/yuxino/mimi/issues) · [貢獻指南](../../.github/CONTRIBUTING.md)

<a id="apple-本地识别"></a>

### Apple 本機辨識

**Apple Speech** 只在系統支援時出現：Apple 晶片、macOS 26 或更新版本，且系統辨識引擎可用。辨識不需要 API 金鑰。先停止字幕，在「辨識語言」中選好語言；未下載時按一下「下載並使用」，Mimi 會下載 Apple 資源，並在就緒後將它設為辨識語言。已下載時按一下「設為辨識語言」，再啟動字幕。無需前往系統設定。不提供自動辨識語言。詳見 [Apple Speech 設定步驟](../provider-setup.zh-CN.md#apple-speech)。

**Apple Translation** 是獨立的本機文字翻譯服務，目前支援 Apple 晶片、macOS 26 或更新版本。在「文字翻譯」中選擇它並儲存設定，再選擇明確的辨識語言和翻譯語言。Mimi 會顯示語言組合是否就緒；按一下「下載或啟用語言套件」，即可開啟 Apple 的確認介面。翻譯語言套件和語音辨識語言套件分開；會沿用已下載的套件，缺少的由 Apple 在你確認後下載。檢查連線、啟動字幕不會觸發下載。不需要文字翻譯 API 金鑰或文字代理伺服器。

Apple Speech 只看原文時，選擇「不翻譯（僅原文）」。也可搭配 [Index-Translate](#试试-index-translate) 等遠端文字翻譯服務；遠端服務會收到辨識文字。

<a id="试试-index-translate"></a>

### 試試 Index-Translate

B 站的 [Index-Translate](https://github.com/bilibili/Index-Translate#inference) 目前提供免費的公開翻譯 API（截至 2026 年 10 月 5 日）。可以作為文字翻譯服務，搭配阿里雲或 Apple 本機辨識使用。

在「**設定 → 語音與翻譯**」中開啟「**Alibaba Cloud**」或「**Apple Speech**」設定，將「**文字翻譯**」的服務切換為「**OpenAI 相容介面**」，按[官方範例](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py#L40-L41)填寫：

| 欄位 | 填寫內容 |
| --- | --- |
| 服務位址 | `https://index-translate.bilibili.com/v1` |
| 模型名稱 | `Index-Translate-35B-A3B` |
| API 金鑰 | 留空，公開介面目前不需要認證。 |

儲存後，按一下「文字翻譯」旁的連線檢查。如果這個位址之前儲存過金鑰，選擇「移除翻譯金鑰」後再儲存。

Index-Translate 只負責文字翻譯。搭配阿里雲時，仍需設定語音辨識認證資訊，辨識服務可能產生費用；搭配 Apple Speech 時，本機辨識不需要金鑰。免費介面的後續可用性以上游為準。

## 常見問題

**macOS 已開啟音訊錄製權限，仍反覆要求授權？** 先結束 Mimi，在「系統設定 → 隱私權與安全性 → 螢幕與系統音訊錄製」中，僅刪除並重新新增對應應用程式：正式版為 `/Applications/mimi.app`，開發版為 `/Applications/mimi-dev.app`，開啟權限後重新開啟同一個應用程式。詳見[權限恢復步驟](../usage.zh-CN.md#macos-更新后重复授权)。

## 貢獻者

感謝每一位寫程式碼、提問題、試用和分享的朋友 (๑•̀ㅂ•́)و✧

特別感謝 [@yebuwudong](https://github.com/yebuwudong) 貢獻 [Android 版](https://github.com/yuxino/mimi/pull/37)，以及 [@LLLin000](https://github.com/LLLin000) 貢獻[字幕動畫](https://github.com/yuxino/mimi/pull/67)和 [Windows 音訊來源改進](https://github.com/yuxino/mimi/pull/89)。

也感謝 [@Chtholly000](https://github.com/Chtholly000) 改進 [Gemini 連續字幕和連線輪換](https://github.com/yuxino/Mimi/pull/176)。

感謝 [@EmmetZ](https://github.com/EmmetZ) 修復 [Wayland 下字幕浮窗與控制元件未同步移動的問題](https://github.com/yuxino/Mimi/pull/242)。

<p>
  <a href="https://github.com/yuxino"><img src="../assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="../assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="../assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="../assets/contributors/Chtholly000.svg" width="64" height="64" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="../assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="../assets/contributors/EmmetZ.svg" width="64" height="64" alt="@EmmetZ"></a>
</p>

[檢視所有貢獻者](https://github.com/yuxino/mimi/graphs/contributors)

## 社群致謝

也感謝 [V2EX](https://www.v2ex.com/)、[LINUX DO](https://linux.do/)、[小眾軟體](https://meta.appinn.net/)、[NodeLoc](https://www.nodeloc.com/)、[Solo](https://solo.xin/)、[新趣集](https://xinquji.com/posts/859305)和[電鴨](https://eleduck.com/)社群朋友的試用、意見回饋與分享。

[MIT](../../LICENSE) © 2026 yuxino
