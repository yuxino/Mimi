<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>Mimi <sup>みみ</sup></h1>
  <p>Live subtitles and translation for system audio or your microphone on macOS 13+ (Apple silicon and Intel) and Windows / Linux x86_64.</p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="Latest release"></a>
    <a href="https://github.com/yuxino/mimi/releases"><img src="https://img.shields.io/github/downloads/yuxino/mimi/total?style=flat&amp;labelColor=a85f82&amp;color=e889b5" alt="Total downloads"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="CI status on main"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT license"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
  <p>
    <a href="README.md">English</a> · <a href="README_ZH.md">简体中文</a> · <a href="README_ZH-TW.md">繁體中文</a> · <a href="README_KO.md">한국어</a> · <a href="README_FR.md">Français</a> · <a href="README_DE.md">Deutsch</a>
  </p>
</div>

<p align="center">Mimi turns speech from your computer or microphone into live translated subtitles. Watch films, follow streams, or take lessons with subtitles floating over your screen.</p>

![Mimi's bilingual subtitle window over an original illustrated scene](docs/assets/readme-preview.png)

## Features

- Choose system audio, microphone, or both. System audio is the default; microphone capture requires explicit selection.
- Translate audio from a selected app on macOS or Windows 11.
- Show original text, translations, or both.
- Adjust subtitle position, size, and color, or let mouse clicks pass through the window.
- Save subtitles or audio locally and export TXT / WAV when needed. Saving and recording are off by default.

The desktop interface supports **简体中文 · 繁體中文 · English · 日本語 · Deutsch · 한국어 · Français**. Change it in **Settings → General**. Interface, recognition and translation languages are separate choices.

On desktop, each service configuration can remember a recognition/translation language pair. Save, update or remove it explicitly in the configuration details. Switching to or reapplying the configuration restores its saved pair; temporary language changes do not overwrite it. Tray and floating menus show the pair below the configuration name. Click the service information in the subtitle header to open the current configuration details.

## Get started

For your first setup, **we recommend starting with Alibaba Cloud or Google Gemini**. We’ve done more real-world testing with Alibaba Cloud; in my experience so far, Gemini has delivered the most consistent subtitle output.

For Google Gemini, use a stable network connection. Check your available quota, billing, and API keys in [Google AI Studio](https://aistudio.google.com/).

For activation links, credential instructions, and the models Mimi currently uses, see the **[provider setup guide](docs/provider-setup.md)**.

1. Open Settings → Speech & Translation, add a configuration, enter the requested provider credentials, and save.
2. Choose the recognition and translation languages.
3. Play something and turn on Live Subtitles under Subtitles. On macOS, allow Screen & System Audio Recording when asked.

Cloud speech services require your own credentials and receive your audio; usage charges may apply. Apple Speech recognizes audio locally on supported Macs. Remote text translation sends recognized text to your chosen service; Apple Translation keeps text on the Mac.

[Setup & help](docs/usage.md) · [Android](android/README.md) · [Report a bug](https://github.com/yuxino/mimi/issues) · [Contributing](CONTRIBUTING.md)

<a id="apple-local-recognition"></a>

### Apple local recognition

**Apple Speech** appears when the system supports it: Apple silicon, macOS 26 or later, and an available system transcriber. It needs no speech API key. Stop subtitles and choose the language under **Recognition Language**. If it is missing, click **Download and use**; Mimi downloads its Apple resources and selects that language when ready. For an already downloaded language, click **Set recognition language**, then start subtitles. No System Settings detour is needed. Automatic language detection is not offered. See [Apple Speech setup](docs/provider-setup.md#apple-speech).

**Apple Translation** is a separate on-device text service on supported Apple silicon Macs running macOS 26 or later. Select it under **Text Translation**, save the configuration, and choose an explicit source and target language. Mimi shows whether the pair is ready; use **Download or enable languages** to open Apple’s setup confirmation. Translation models are separate from speech models. Existing models are reused; Apple downloads missing models when you confirm. Checks and subtitle sessions never request a download. No text API key or text proxy is needed.

To show only the original text with Apple Speech, select **No translation (original only)**. You can also use a remote text translator such as [Index-Translate](#try-index-translate); that service receives recognized text.

### Try Index-Translate

Bilibili's [Index-Translate](https://github.com/bilibili/Index-Translate#inference) currently offers a free public translation API (as of October 5, 2026). It can provide text translation for Alibaba Cloud or Apple Speech recognition.

In **Settings → Speech & Translation**, open an **Alibaba Cloud** or **Apple Speech** configuration and select **OpenAI-compatible API** under **Text translation**. Use the values from the [official example](https://github.com/bilibili/Index-Translate/blob/main/inference/llm/call_api.py#L40-L41):

| Field | Value |
| --- | --- |
| Service address | `https://index-translate.bilibili.com/v1` |
| Model name | `Index-Translate-35B-A3B` |
| API Key | Leave empty; the public API currently requires no authentication. |

Save, then run the connection check beside **Text translation**. If this address already has a saved key, remove it with **Remove translation key** and save again.

Index-Translate handles text translation only. When using Alibaba Cloud, keep its speech-recognition credentials configured; recognition may still incur charges. Apple Speech recognition needs no key. Free API availability is subject to the upstream service.

## FAQ

**macOS keeps asking for recording permission even though it is enabled?** Quit Mimi, then remove and re-add only its entry in System Settings → Privacy & Security → Screen & System Audio Recording. Use `/Applications/mimi.app` for the release app or `/Applications/mimi-dev.app` for development, enable it, and reopen the same app. See [permission recovery](docs/usage.md#macos-permissions-after-an-update).

## Contributors

Thanks to everyone who writes code, reports issues, tries Mimi, or shares it (๑•̀ㅂ•́)و✧

Special thanks to [@yebuwudong](https://github.com/yebuwudong) for the [Android app](https://github.com/yuxino/mimi/pull/37), and [@LLLin000](https://github.com/LLLin000) for [subtitle animation](https://github.com/yuxino/mimi/pull/67) and [Windows audio improvements](https://github.com/yuxino/mimi/pull/89).

Thanks also to [@Chtholly000](https://github.com/Chtholly000) for improving [Gemini's continuous captions and planned connection rotation](https://github.com/yuxino/Mimi/pull/176).

<p>
  <a href="https://github.com/yuxino"><img src="docs/assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/LLLin000"><img src="docs/assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="docs/assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
  <a href="https://github.com/Chtholly000"><img src="docs/assets/contributors/Chtholly000.svg" width="64" height="64" alt="@Chtholly000"></a>
  <a href="https://github.com/inhome"><img src="docs/assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
</p>

[All contributors](https://github.com/yuxino/mimi/graphs/contributors)

## Community

Thanks to the people in [V2EX](https://www.v2ex.com/), [LINUX DO](https://linux.do/), [Appinn](https://meta.appinn.net/), [NodeLoc](https://www.nodeloc.com/), [Solo](https://solo.xin/), [Xinquji](https://xinquji.com/posts/859305), and [Eleduck](https://eleduck.com/) for trying Mimi, sharing feedback, and spreading the word.

[MIT](LICENSE) © 2026 yuxino
