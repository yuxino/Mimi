<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi</h1>
  <p>Live translated subtitles from computer or microphone audio.</p>
  <p><a href="https://github.com/yuxino/mimi/releases/latest"><strong>Download Mimi</strong></a> · <a href="https://mimi.yuxino.cn/en/">Website</a> · <a href="docs/usage.md">User guide</a></p>
  <p>macOS 13+ (Apple silicon / Intel) · Windows x64 · Linux x86_64 · <a href="android/README.md#start">Android 10+</a></p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="Latest release"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT license"></a>
  </p>
  <p>
    <a href="README_EN.md"><strong>English</strong></a> · <a href="docs/readme/zh-CN.md">简体中文</a> · <a href="docs/readme/zh-TW.md">繁體中文</a> · <a href="docs/readme/ja.md">日本語</a> · <a href="docs/readme/th.md">ภาษาไทย</a> · <a href="docs/readme/ko.md">한국어</a> · <a href="docs/readme/fr.md">Français</a> · <a href="docs/readme/de.md">Deutsch</a>
  </p>
</div>

## Get started

Mimi is free. Cloud services need your own API key and may charge for usage. Captured audio or recognized text is sent to the service you choose.

Supported Apple silicon Macs on macOS 26+ can also use [Apple local services](docs/provider-setup.md#apple-speech) without API keys after downloading the required language resources.

1. **Download and install.** Choose the package for your system on the [download page](https://github.com/yuxino/mimi/releases/latest): DMG for Mac, EXE for Windows, or AppImage / DEB for Linux. For phones, follow the [Android setup](android/README.md#start).
2. **Set up a service.** Open Settings → Models & Services, add OpenAI, Gemini, xAI or another supported service, enter its credentials, and save. The [setup guide](docs/provider-setup.md#choose) explains where to get them and what to enter.
3. **Choose audio and languages.** Open Audio & Languages and choose the spoken and translation languages. Computer audio is selected by default. Choose Microphone to translate your own speech, or select both sources.
4. **Start subtitles.** Turn on Live Subtitles on the Subtitles page, grant the requested permissions, and play a video, stream or other content with speech.

![Mimi bilingual subtitle illustration with fictional dialogue](docs/assets/readme-preview.png)

Interface illustration with sample dialogue. [Watch subtitles and Immersive Mode (older UI)](docs/demos/README.md).

## What you can do

- Show both languages, translations only, or originals only.
- Move the subtitles and adjust text size and color. Immersive Mode keeps just the text and lets clicks pass through to the app underneath.
- On desktop, capture one app on macOS / Windows 11, or an individual playback stream on Linux.
- On desktop, turn on subtitle saving or audio recording to export TXT / Markdown subtitles or WAV audio. Both are off by default.

## Need help?

The [user guide](docs/usage.md) covers installation, permissions, display and saving. For missing subtitles, start with [recognition errors](docs/usage.md#errors). You can also [report an issue](https://github.com/yuxino/mimi/issues).

<details>
<summary>Contributors and community</summary>

## Contributors

Want to help improve Mimi? Start with the [contribution guide](.github/CONTRIBUTING.md).

Thanks to everyone who writes code, reports issues, tries Mimi, or shares it (๑•̀ㅂ•́)و✧

Special thanks to [@yebuwudong](https://github.com/yebuwudong) for the [Android app](https://github.com/yuxino/mimi/pull/37), and [@LLLin000](https://github.com/LLLin000) for [subtitle animation](https://github.com/yuxino/mimi/pull/67) and [Windows audio improvements](https://github.com/yuxino/mimi/pull/89).

Thanks also to [@Chtholly000](https://github.com/Chtholly000) for improving [Gemini's continuous captions and planned connection rotation](https://github.com/yuxino/Mimi/pull/176).

Thanks to [@EmmetZ](https://github.com/EmmetZ) for fixing [Wayland overlay controls](https://github.com/yuxino/Mimi/pull/242) and adding [individual playback stream capture on Linux](https://github.com/yuxino/Mimi/pull/275) and [application icons for playback streams](https://github.com/yuxino/Mimi/pull/304).

Thanks to [@wangmian0](https://github.com/wangmian0) for [disabling thinking mode in DeepSeek live subtitle translation](https://github.com/yuxino/Mimi/pull/270).

Thanks to [@hubumulity](https://github.com/hubumulity) for the [visible subtitle count setting](https://github.com/yuxino/Mimi/pull/303).

Thanks to [@ryannyui](https://github.com/ryannyui) for [Linux AppImage graphics compatibility fixes and the toolchain upgrade](https://github.com/yuxino/Mimi/pull/312).

<p>
  <a href="https://github.com/yuxino"><img src="docs/assets/contributors/yuxino.svg" width="40" height="40" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="docs/assets/contributors/LLLin000.svg" width="40" height="40" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="docs/assets/contributors/yebuwudong.svg" width="40" height="40" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="docs/assets/contributors/Chtholly000.svg" width="40" height="40" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="docs/assets/contributors/inhome.svg" width="40" height="40" alt="@inhome"></a>
  <a href="https://github.com/EmmetZ"><img src="docs/assets/contributors/EmmetZ.svg" width="40" height="40" alt="@EmmetZ"></a>
  <a href="https://github.com/wangmian0"><img src="docs/assets/contributors/wangmian0.svg" width="40" height="40" alt="@wangmian0"></a>
  <a href="https://github.com/hubumulity"><img src="docs/assets/contributors/hubumulity.svg" width="40" height="40" alt="@hubumulity"></a>
  <a href="https://github.com/ryannyui"><img src="docs/assets/contributors/ryannyui.svg" width="40" height="40" alt="@ryannyui"></a>
</p>

[All contributors](https://github.com/yuxino/mimi/graphs/contributors)

## Community

Thanks to the people in [V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305), and [Eleduck](https://eleduck.com/) for trying Mimi, sharing feedback, and spreading the word.

</details>

[MIT](LICENSE) © 2026 yuxino
