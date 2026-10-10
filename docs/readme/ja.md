<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>パソコンやマイクの音声をリアルタイムで翻訳字幕に。</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="最新リリース"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="総ダウンロード数"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="main ブランチの CI 状態"></a>
    <a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT ライセンス"></a>
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

<p align="center">Mimi は、パソコンやマイクの音声をリアルタイムで翻訳する字幕アプリです。映画、ライブ配信、オンライン授業などの音声を、画面上に浮かぶ字幕で読めます。</p>

![オリジナルのイラストに重ねて表示した Mimi の二言語字幕ウィンドウ](../assets/readme-preview.png)

## 機能

- システム音声、マイク、または両方を選択できます。初期設定はシステム音声で、マイクは自分で選んだ場合にのみ使用します。
- 特定のアプリの音声を翻訳できます（macOS / Windows 11）。
- 原文、翻訳、または両方を表示できます。
- 字幕の位置、サイズ、色を調整できます。マウスのクリックを字幕ウィンドウの背面に通すこともできます。
- 必要に応じて字幕や音声を端末に保存し、TXT / WAV で書き出せます。保存と録音は初期設定ではオフです。
- Apple シリコンと Intel の Mac（macOS 13 以降）、Windows / Linux x86_64 に対応しています。

## はじめに

初めて使う場合は、**Alibaba Cloud または Google Gemini をおすすめします**。Alibaba Cloud は実際の利用環境でより多くの検証を行っています。私がこれまで使った範囲では、Gemini の字幕出力が最も安定していました。

Google Gemini を使う場合は、安定したネットワーク接続を用意してください。利用可能な割り当て、料金、API キーは [Google AI Studio](https://aistudio.google.com/) で確認できます。

各サービスの有効化、認証情報の取得、Mimi が現在使用しているモデルについては、[**サービス設定ガイド（英語）**](../provider-setup.md)をご覧ください。

1. 「設定 → 音声と翻訳」を開き、設定を追加して、サービスの認証情報を入力・保存します。
2. 認識言語と翻訳先の言語を選びます。
3. 音声を再生し、「字幕」ページで「リアルタイム字幕」をオンにします。macOS 14.2 以降では、すべてのアプリまたは特定のアプリの音声を取得する際に、システム音声のみの録音権限を使用できます。既存の画面収録権限がある場合は引き続き使用します。それより前の macOS では、画面とシステム音声の収録権限が必要です。

クラウド音声認識には自分で用意したサービスの認証情報が必要で、音声はそのサービスに送信されます。利用料金が発生する場合があります。対応する Mac では、Apple Speech で端末内の音声認識を行えます。外部の文字翻訳サービスを使う場合は、認識したテキストがそのサービスに送信されます。Apple Translation は Mac 内でテキストを処理します。

[使い方とよくある質問（英語）](../usage.md) · [Android](../../android/README.md) · [不具合を報告](https://github.com/yuxino/mimi/issues) · [開発への参加](../../.github/CONTRIBUTING.md)

<a id="apple-local-recognition"></a>

### Apple によるローカル音声認識

**Apple Speech** は、Apple シリコン、macOS 26 以降、利用可能なシステム音声認識エンジンがそろった Mac で表示されます。音声認識用の API キーは不要です。字幕を停止して「認識言語」で言語を選んでください。未ダウンロードの場合は「ダウンロードして使用」を押すと、Mimi が Apple のリソースをダウンロードし、準備ができた時点で認識言語に設定します。ダウンロード済みの場合は「認識言語に設定」を押してから字幕を開始してください。システム設定を開く必要はありません。言語の自動判定には対応していません。詳しくは [Apple Speech の設定手順（英語）](../provider-setup.md#apple-speech)をご覧ください。

**Apple Translation** は、Apple シリコンと macOS 26 以降に対応した Mac で使える、独立した端末内の文字翻訳サービスです。「文字翻訳」で選んで設定を保存し、認識言語と翻訳先を指定してください。Mimi に言語の組み合わせの準備状態が表示されます。「言語パックをダウンロード／有効化」を押すと、Apple の確認画面が開きます。翻訳用と音声認識用の言語パックは別々です。既存のパックは再利用し、不足しているものは確認後に Apple がダウンロードします。接続確認や字幕の開始ではダウンロードされません。文字翻訳用の API キーやプロキシは不要です。

Apple Speech で原文だけを表示する場合は、「翻訳なし（原文のみ）」を選びます。[Index-Translate](#try-index-translate) などの外部の文字翻訳サービスと組み合わせることもできます。その場合、認識したテキストは外部サービスに送信されます。

<a id="try-index-translate"></a>

### Index-Translate を試す

Bilibili の [Index-Translate](https://github.com/bilibili/Index-Translate#inference) は、無料の公開翻訳 API を提供しています（2026 年 10 月 5 日時点）。Alibaba Cloud または Apple Speech で認識したテキストの翻訳に使用できます。

「**設定 → 音声と翻訳**」で「**Alibaba Cloud**」または「**Apple Speech**」の設定を開き、「**文字翻訳**」で「**OpenAI 互換 API**」を選びます。[公式のサンプル](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py#L40-L41)に従って、次の値を入力してください。

| 項目 | 入力する値 |
| --- | --- |
| サービス URL | `https://index-translate.bilibili.com/v1` |
| モデル名 | `Index-Translate-35B-A3B` |
| API Key | 空欄のままにします。公開 API は現在、認証を必要としません。 |

保存してから、「文字翻訳」の接続確認を行ってください。この URL にキーを保存したことがある場合は、「翻訳キーを削除」を選んでから保存してください。

Index-Translate は文字翻訳のみを行います。Alibaba Cloud と組み合わせる場合は、音声認識の認証情報も必要で、認識には利用料金が発生する場合があります。Apple Speech の音声認識にはキーは不要です。無料 API の今後の提供状況は、提供元によって変わる可能性があります。

## よくある質問

**録音の権限を許可しているのに、macOS が何度も許可を求める場合は？** Mimi を終了し、「システム設定 → プライバシーとセキュリティ → 画面収録とシステムオーディオ録音」で、Mimi の項目だけを削除して追加し直してください。正式版は `/Applications/mimi.app`、開発版は `/Applications/mimi-dev.app` を使用します。権限をオンにしてから、同じアプリを開き直してください。詳しくは[権限の復旧手順（英語）](../usage.md#macos-permissions-after-an-update)をご覧ください。

## 貢献者

コードの作成、不具合の報告、試用、紹介をしてくださる皆さん、ありがとうございます (๑•̀ㅂ•́)و✧

[@yebuwudong](https://github.com/yebuwudong) さんの [Android 版](https://github.com/yuxino/mimi/pull/37)、[@LLLin000](https://github.com/LLLin000) さんの[字幕アニメーション](https://github.com/yuxino/mimi/pull/67)と [Windows の音声取得の改善](https://github.com/yuxino/mimi/pull/89)に、特に感謝します。

[@Chtholly000](https://github.com/Chtholly000) さんによる [Gemini の連続字幕と計画的な接続切り替えの改善](https://github.com/yuxino/Mimi/pull/176)にも感謝します。

[@EmmetZ](https://github.com/EmmetZ) さんは、[Wayland で字幕ウィンドウの移動に操作パネルが追従しない問題](https://github.com/yuxino/Mimi/pull/242)を修正してくださいました。

<p>
  <a href="https://github.com/yuxino"><img src="../assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="../assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="../assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="../assets/contributors/Chtholly000.svg" width="64" height="64" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="../assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="../assets/contributors/EmmetZ.svg" width="64" height="64" alt="@EmmetZ"></a>
</p>

[すべての貢献者を見る](https://github.com/yuxino/mimi/graphs/contributors)

## コミュニティへの感謝

[V2EX](https://www.v2ex.com/)、[LINUX DO](https://linux.do/)、[Appinn](https://meta.appinn.net/)、[NodeLoc](https://www.nodeloc.com/)、[Solo](https://solo.xin/)、[Xinquji](https://xinquji.com/posts/859305)、[Eleduck](https://eleduck.com/) のコミュニティの皆さんにも、試用、フィードバック、紹介をしていただき、ありがとうございます。

[MIT](../../LICENSE) © 2026 yuxino
