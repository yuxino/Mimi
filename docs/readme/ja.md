<div align="center">
  <img src="../../src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>再生中の動画に、リアルタイムの翻訳字幕を。</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="最新リリース"></a>&nbsp;<a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="総ダウンロード数"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="main ブランチの CI 状態"></a>&nbsp;<a href="../../LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT ライセンス"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>&nbsp;<a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>&nbsp;<a href="../../android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p><a href="https://github.com/yuxino/mimi/releases/latest"><strong>Mimi をダウンロード</strong></a> · <a href="https://mimi.yuxino.cn/ja/">公式サイト</a> · <a href="../usage.ja.md">使い方</a></p>
  <p>
    <a href="../../README_EN.md">English</a> · <a href="../../README.md">简体中文</a> · <a href="zh-TW.md">繁體中文</a> · <a href="ja.md">日本語</a> · <a href="th.md">ภาษาไทย</a> · <a href="ko.md">한국어</a> · <a href="fr.md">Français</a> · <a href="de.md">Deutsch</a>
  </p>
</div>

**字幕アプリの追加サブスクは不要。** Mimi は有料のリアルタイム字幕アプリに代わる、無料のオープンソースアプリです。自分の API キーで音声認識・翻訳サービスをつなぎ、動画や配信、オンライン授業に翻訳字幕を付けられます。

![サンプルの会話を使った Mimi の二言語字幕イメージ](../assets/readme-preview.png)

サンプルの会話による画面イメージです。[字幕とイマーシブモードのデモ（旧 UI）](../demos/README.md)。

## できること

- **二言語で見る**：外国語の動画や配信、オンライン授業で、二言語、訳文のみ、原文のみを切り替えられます。
- **字幕だけを残す**：字幕の位置、文字サイズ、色を調整できます。イマーシブモードでは字幕だけを残し、背後のアプリをクリックできます。
- **音源を選ぶ**：デスクトップ版では、macOS / Windows 11 はアプリ、Linux は再生ストリームを指定できます。
- **あとで見返す**：デスクトップ版では字幕の保存や録音を有効にすると、TXT / Markdown の字幕や WAV 音声を書き出せます。どちらも初期設定ではオフです。

## はじめに

クラウドサービスには自分の API キーなどが必要で、利用料がかかる場合があります。音声や認識した文字を選択したサービスに送信します。

対応する Apple シリコン Mac（macOS 26 以降）では、言語リソースをダウンロードすると [Apple のローカルサービスを API キーなしで使えます](../provider-setup.md#apple-speech)。

1. **ダウンロードしてインストールする**：[ダウンロードページ](https://github.com/yuxino/mimi/releases/latest)で、macOS 13 以降（Apple シリコン / Intel）は DMG、Windows x64 は EXE、Linux x86_64 は AppImage / DEB を選びます。Android 10 以降のスマートフォンは[専用の手順（英語）](../../android/README.md#start)をご覧ください。
2. **サービスを設定する**：「設定 → モデルとサービス」で OpenAI、Gemini、xAI などの対応サービスを追加し、自分の API キーなどを入力して保存します。取得方法と入力内容は[設定ガイド（英語）](../provider-setup.md#choose)にあります。
3. **音源と言語を選ぶ**：音声と言語の設定で、認識言語と翻訳先を選びます。初期設定はパソコンの音声です。自分の声を翻訳する場合はマイク、両方使う場合は二つの音源を選びます。
4. **字幕を開始する**：「字幕」ページでライブ字幕をオンにし、必要な権限を許可してから、人の声が入った動画や配信を再生します。

## 困ったときは

[使い方ガイド](../usage.ja.md)でインストール、権限、字幕表示、保存を説明しています。字幕が出ない・接続できない場合は[音声認識のエラー](../usage.ja.md#errors)を確認し、解決しなければ[問題を報告](https://github.com/yuxino/mimi/issues)してください。

<details>
<summary>貢献者とコミュニティ</summary>

## 貢献者

開発に参加する方は[貢献ガイド（英語・中国語）](../../.github/CONTRIBUTING.md)をご覧ください。

コードの作成、不具合の報告、試用、紹介をしてくださる皆さん、ありがとうございます (๑•̀ㅂ•́)و✧

[@yebuwudong](https://github.com/yebuwudong) さんの [Android 版](https://github.com/yuxino/mimi/pull/37)、[@LLLin000](https://github.com/LLLin000) さんの[字幕アニメーション](https://github.com/yuxino/mimi/pull/67)と [Windows の音声取得の改善](https://github.com/yuxino/mimi/pull/89)に、特に感謝します。

[@Chtholly000](https://github.com/Chtholly000) さんによる [Gemini の連続字幕と計画的な接続切り替えの改善](https://github.com/yuxino/Mimi/pull/176)にも感謝します。

[@EmmetZ](https://github.com/EmmetZ) さんによる [Wayland の字幕操作パネルの修正](https://github.com/yuxino/Mimi/pull/242)、[Linux での個別再生ストリームの音声取得](https://github.com/yuxino/Mimi/pull/275)、[再生ストリームのアプリアイコン表示](https://github.com/yuxino/Mimi/pull/304)に感謝します。

[@wangmian0](https://github.com/wangmian0) さんは、[DeepSeek によるリアルタイム字幕翻訳の思考モードを無効化](https://github.com/yuxino/Mimi/pull/270)してくださいました。

[@hubumulity](https://github.com/hubumulity) さんによる[字幕表示件数の設定](https://github.com/yuxino/Mimi/pull/303)に感謝します。

[@ryannyui](https://github.com/ryannyui) さんによる [Linux AppImage のグラフィックス互換性修正とツールチェーン更新](https://github.com/yuxino/Mimi/pull/312)に感謝します。

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

[すべての貢献者を見る](https://github.com/yuxino/mimi/graphs/contributors)

## コミュニティへの感謝

[V2EX](https://www.v2ex.com/)、[LINUX DO](https://linux.do/)、[Appinn](https://meta.appinn.net/)、[NodeLoc](https://www.nodeloc.com/)、[Solo](https://solo.xin/)、[Xinquji](https://xinquji.com/posts/859305)、[Eleduck](https://eleduck.com/) のコミュニティの皆さんにも、試用、フィードバック、紹介をしていただき、ありがとうございます。

</details>

[MIT](../../LICENSE) © 2026 yuxino
