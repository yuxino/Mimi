<div align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="96" alt="mimi">
  <h1>mimi</h1>
  <p>Live subtitles and translation for system audio on macOS 13+ (Apple silicon and Intel) and Windows / Linux x86_64.</p>
  <p>
    <a href="https://mimi.yuxino.cn">Website</a>
    · <a href="https://github.com/yuxino/mimi/releases/latest"><strong>Download latest</strong></a>
    · <a href="README_ZH.md">简体中文</a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/github/v/release/yuxino/mimi?style=flat&amp;logo=github&amp;logoColor=white" alt="Latest release"></a>
    <a href="https://github.com/yuxino/mimi/actions/workflows/ci.yml?query=branch%3Amain"><img src="https://img.shields.io/github/actions/workflow/status/yuxino/mimi/ci.yml?style=flat&amp;logo=githubactions&amp;logoColor=white&amp;branch=main&amp;event=push&amp;label=CI" alt="CI status on main"></a>
    <a href="LICENSE"><img src="https://img.shields.io/github/license/yuxino/mimi?style=flat&amp;logo=opensourceinitiative&amp;logoColor=white" alt="MIT license"></a>
  </p>
  <p>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/macOS-13%2B-555?style=flat&amp;logo=apple&amp;logoColor=white" alt="macOS 13+"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Windows-x64-0078D4?style=flat&amp;logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCI%2BPHBhdGggZmlsbD0id2hpdGUiIGQ9Ik0wIDBoMTF2MTFIMHptMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgwem0xMyAwaDExdjExSDEzeiIvPjwvc3ZnPg%3D%3D&amp;logoColor=white" alt="Windows x64"></a>
    <a href="https://github.com/yuxino/mimi/releases/latest"><img src="https://img.shields.io/badge/Linux-x86__64-FCC624?style=flat&amp;logo=linux&amp;logoColor=white" alt="Linux"></a>
    <a href="android/README.md"><img src="https://img.shields.io/badge/Android-app-3DDC84?style=flat&amp;logo=android&amp;logoColor=white" alt="Android"></a>
  </p>
</div>

Mimi shows live subtitles in a floating window for films, live streams, lessons, and games playing on your computer. Your chosen cloud service transcribes the selected audio or translates it into the languages supported by your selected service. The name `mimi` means “ear” in Japanese.

<!-- project-demo-v1 -->
## Demo

https://github.com/user-attachments/assets/5acd46bb-e6b5-4bb5-b280-d70d4e0cdbb4

<p align="center">A 4K / 60 fps tour of service setup, subtitle controls, and Immersive Mode in the macOS app, with English narration, captions, and original film audio.</p>
<p align="center"><a href="https://mimi.yuxino.cn/en/?lang=en#demo">Watch in English</a> · <a href="https://mimi.yuxino.cn/?lang=zh#demo">观看中文版</a> · <a href="docs/demos/full-tour-4k.md">Video details and credits</a></p>
<!-- /project-demo-v1 -->

## Features

- **Live subtitles and translation** — system audio is on by default. Desktop microphone input and its controls are temporarily unavailable. A saved microphone or both-input selection is changed to system audio on startup, with audio recording switched off; explicitly enable recording again if needed.
- **Service configurations** — save and switch between services without repeatedly entering credentials.
- **Subtitle overlay** — move, resize, collapse, pause, enable click-through, or use Immersive Mode. New installations start at 659×328 with font size 16; saved window size and font preferences are preserved.
- **Updates** — check and install updates in Settings on macOS, Windows installers, and Linux AppImage. Windows ZIP and Linux .deb builds link to Releases for manual updates.
- **Save & export** — under Settings → Save & export, choose whether to save confirmed subtitles or record system audio as WAV. Browse saved sessions, then export TXT / WAV or delete a session. Both switches are off by default.
- **Settings appearance** — light, dark, or follow the system.
- **Privacy** — no mimi account or screen video capture; audio goes only to the active provider. History retention and audio recording are off by default. When enabled, confirmed subtitles or audio are saved incrementally to private, bounded local session files. Turning an option off clears its current-session content; saved sessions remain until you delete them explicitly.

Transcript retention is limited to 10,000 confirmed pairs / 2 MiB of text and audio to 64 MiB; reaching a limit stops retention and shows a notice. Transcript timestamps mark final confirmation time, not media playback time. Pauses and reconnect gaps are omitted from WAV audio, so it is not synchronized to transcript timestamps.

## Get started

1. Download the macOS Apple silicon or Intel DMG, a Windows x64 EXE, MSI, or portable ZIP, or a Linux x86_64 .deb / AppImage from the [latest release](https://github.com/yuxino/mimi/releases/latest), or build from source.
2. Open **Translation Service**, choose a provider, and save its credentials.
3. Play something and select **Start** from the mimi menu bar/system tray icon. macOS asks for **Screen & System Audio Recording** access to capture system audio.

Bring your own provider API credentials; usage charges may apply. Credentials are stored in the OS credential store.

For macOS, Windows installer copies, and Linux AppImage, open **Settings → General → Software Update**. Mimi downloads the
update with progress, then lets you install it. Windows reopens Mimi after
installation; macOS and Linux AppImage offer a separate **Restart and Finish Update** action.
Versions older than v1.3.8 need one manual installation to enable in-app updates.

### Platform support

- **macOS 13+ (Apple silicon and Intel)**: Choose the `_aarch64.dmg` for Apple silicon or `_x64.dmg` for Intel. Intel packages are available from v1.4.4; build and signing checks passed, but Intel hardware capture and permission behavior remain unverified. DMG installers are not Apple-notarized. If first launch is blocked, choose **Open Anyway** in **System Settings → Privacy & Security**. See the permission notes below when upgrading from an older build.
- **Windows x64**: Unsigned preview EXE / MSI installers and, since v1.4.3, a portable ZIP are available; SmartScreen may warn. Extract `mimi_<version>_x64-portable.zip` and launch `mimi.exe` without installation. WebView2 must already be installed (it is normally present on Windows 11). The ZIP does not move settings, service credentials, or exported files into its folder; those remain in their existing user-selected or OS-managed locations. Update this copy by quitting Mimi and replacing it with a new ZIP from Releases. The portable build does not run the in-app installer updater.

- **Linux x86_64 preview (Ubuntu 22.04+ baseline)**: Use the `.deb` package or AppImage. Requires PulseAudio or PipeWire with `pipewire-pulse`, a working default output device, and an unlocked Secret Service keyring (for example GNOME Keyring). System audio uses only the output monitor. Restart the session after changing output devices. X11 is recommended; Wayland compositors may restrict positioning, always-on-top, and click-through. For Wayland keyboard shortcuts, use the commands shown in Settings to create system shortcuts. Use Settings if your desktop does not show a tray icon. Minimize it to keep subtitles running; closing it exits Mimi on Linux. Linux ARM64 packages are not provided. See [Linux setup and verification](docs/development/linux.md).

### macOS permissions after an update

The release pipeline now requires one fixed self-signed certificate across macOS builds. Earlier releases through v1.4.1 used ad-hoc signatures that changed with each build; installing the first release with the fixed identity may require one new recording grant. This source change does not alter an already installed app. Fixed signing prevents build-specific identity changes; it does not promise that macOS will never request consent again. Deleting a local signing certificate does not change the installed app's identity or fix this. Keep the release app at `/Applications/mimi.app`; use the separate `mimi-dev.app` for development.

**Recording is enabled, but Mimi still reports permission denied:** first quit and reopen Mimi and follow the normal permission prompt. If capture is still denied:

1. Quit Mimi. In **System Settings → Privacy & Security → Screen & System Audio Recording** (the label varies by macOS version), select and remove only the old **mimi** entry.
2. Use **+** to add `/Applications/mimi.app`, enable its permission, and complete any system authentication yourself. Leave `mimi-dev` and other apps alone.
3. Reopen that same app, start a session, and play audio containing speech to check that subtitles actually appear. An enabled switch alone does not confirm recovery.

This removes an old recording authorization, not a certificate or API key. If one attempt does not help, stop repeating the reset and [report the error](https://github.com/yuxino/mimi/issues), including your macOS and Mimi versions and installation source. Do not include API keys or subtitle content.

**Keychain asks for your password or access to a saved API key:** confirm that the request comes from the Mimi copy you intended to open. If the dialog offers **Always Allow**, it can retain access for that build, but cannot guarantee access after a self-signed binary changes. Do not delete saved credentials, certificates, or the login keychain, allow all apps to access a key, or run a system-wide permission reset. A `codesign` request for a development signing private key is a separate build-time prompt.

My wallet is still a little empty, and I’m saving up for Apple Developer membership (๑•̀ㅂ•́)و✧ Thank you for understanding! Stable self-signing does not need a paid membership; Apple notarization is a separate step. The manual recovery above is for a stale recording grant, not something you should have to repeat after every update.

## Development

See the [contributing guide](CONTRIBUTING.md) for building and contributing, and the [security policy](SECURITY.md) for reporting vulnerabilities.

## Subtitle display

Choose **Translation only**, **Original + translation**, or **Original only**
in Settings, the overlay control panel, or the tray. Switch instantly with
**⌘⇧B** on macOS or **Ctrl+Shift+B** on Windows/Linux X11. On Wayland, bind `mimi --cycle-subtitle-display` in desktop settings. This changes what you see
without restarting translation. Bilingual mode pairs confirmed sentences and
previews the recognized original while a translation is pending.

## Contributors

Thanks to everyone contributing to Mimi. See [CONTRIBUTING.md](CONTRIBUTING.md) to get involved.

Special thanks to [@yebuwudong](https://github.com/yebuwudong) for contributing the native [Android port in PR #37](https://github.com/yuxino/mimi/pull/37). [Download Android](https://github.com/yuxino/mimi/releases/latest); see its [setup and verification notes](android/README.md).

<p>
  <a href="https://github.com/yuxino"><img src="docs/assets/contributors/yuxino.svg" width="64" height="64" alt="@yuxino"></a>
  <a href="https://github.com/inhome"><img src="docs/assets/contributors/inhome.svg" width="64" height="64" alt="@inhome"></a>
  <a href="https://github.com/LLLin000"><img src="docs/assets/contributors/LLLin000.svg" width="64" height="64" alt="@LLLin000"></a>
  <a href="https://github.com/yebuwudong"><img src="docs/assets/contributors/yebuwudong.svg" width="64" height="64" alt="@yebuwudong"></a>
</p>

[View GitHub's contribution history](https://github.com/yuxino/mimi/graphs/contributors). The wall reflects publicly attributed contributions on the default branch; pending work is listed separately.

Currently under review: [@LLLin000's subtitle animation #67](https://github.com/yuxino/mimi/pull/67) and [Windows audio update #89](https://github.com/yuxino/mimi/pull/89), included in [integration acceptance #88](https://github.com/yuxino/mimi/pull/88).

## Community links

[LINUX DO](https://linux.do/)

[MIT](LICENSE) © 2026 yuxino
