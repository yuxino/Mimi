# Using Mimi

[Back to README](../README.md) · [简体中文](usage.zh-CN.md)

## Install and update

1. Download the macOS Apple silicon or Intel DMG, a Windows x64 EXE, MSI, or portable ZIP, or a Linux x86_64 .deb / AppImage from the [latest release](https://github.com/yuxino/mimi/releases/latest), or build from source.
2. Open Settings → Speech & Translation, add a configuration, enter the requested provider credentials, and save. Choose the recognition and translation languages.
3. Play something and turn on Live Subtitles under Subtitles. You can also start from the mimi menu bar/system tray icon. macOS asks for Screen & System Audio Recording access to capture system audio.

Bring your own provider API credentials; usage charges may apply. Credentials are stored in the OS credential store.

For macOS, Windows installer copies, and Linux AppImage, open Settings → General → Software Update. Mimi downloads the
update with progress, then lets you install it. Windows reopens Mimi after
installation; macOS and Linux AppImage offer a separate Restart and Finish Update action.
Versions older than v1.3.8 need one manual installation to enable in-app updates.

### Platform support

- macOS 13+ (Apple silicon and Intel): Choose the `_aarch64.dmg` for Apple silicon or `_x64.dmg` for Intel. Intel packages are available from v1.4.4; build and signing checks passed, but Intel hardware capture and permission behavior remain unverified. DMG installers are not Apple-notarized. If first launch is blocked, choose Open Anyway in System Settings → Privacy & Security. See the permission notes below when upgrading from an older build.
- Windows x64: Unsigned preview EXE / MSI installers and, since v1.4.3, a portable ZIP are available; SmartScreen may warn. Extract `mimi_<version>_x64-portable.zip` and launch `mimi.exe` without installation. WebView2 must already be installed (it is normally present on Windows 11). The ZIP does not move settings, service credentials, or exported files into its folder; those remain in their existing user-selected or OS-managed locations. Update this copy by quitting Mimi and replacing it with a new ZIP from Releases. The portable build does not run the in-app installer updater.

- Linux x86_64 preview (Ubuntu 22.04+ baseline): Use the `.deb` package or AppImage. Requires PulseAudio or PipeWire with `pipewire-pulse`, a working default output device, and an unlocked Secret Service keyring (for example GNOME Keyring). System audio uses only the output monitor. Restart the session after changing output devices. X11 is recommended; Wayland compositors may restrict positioning, always-on-top, and click-through. For Wayland keyboard shortcuts, use the commands shown in Settings to create system shortcuts. Use Settings if your desktop does not show a tray icon. Minimize it to keep subtitles running; closing it exits Mimi on Linux. Linux ARM64 packages are not provided. See [Linux setup and verification](development/linux.md).

### macOS permissions after an update

The release pipeline now requires one fixed self-signed certificate across macOS builds. Earlier releases through v1.4.1 used ad-hoc signatures that changed with each build; installing the first release with the fixed identity may require one new recording grant. This source change does not alter an already installed app. Fixed signing prevents build-specific identity changes; it does not promise that macOS will never request consent again. Deleting a local signing certificate does not change the installed app's identity or fix this. Keep the release app at `/Applications/mimi.app`; use the separate `mimi-dev.app` for development.

Recording is enabled, but Mimi still reports permission denied: first quit and reopen Mimi and follow the normal permission prompt. If capture is still denied:

1. Quit Mimi. In System Settings → Privacy & Security → Screen & System Audio Recording (the label varies by macOS version), select and remove only the old mimi entry.
2. Use + to add `/Applications/mimi.app`, enable its permission, and complete any system authentication yourself. Leave `mimi-dev` and other apps alone.
3. Reopen that same app, start a session, and play audio containing speech to check that subtitles actually appear. An enabled switch alone does not confirm recovery.

This removes an old recording authorization, not a certificate or API key. If one attempt does not help, stop repeating the reset and [report the error](https://github.com/yuxino/mimi/issues), including your macOS and Mimi versions and installation source. Do not include API keys or subtitle content.

Keychain asks for your password or access to a saved API key: confirm that the request comes from the Mimi copy you intended to open. If the dialog offers Always Allow, it can retain access for that build, but cannot guarantee access after a self-signed binary changes. Do not delete saved credentials, certificates, or the login keychain, allow all apps to access a key, or run a system-wide permission reset. A `codesign` request for a development signing private key is a separate build-time prompt.

## Audio and saved sessions

Desktop microphone input is temporarily unavailable. Startup changes a saved microphone or both-input selection to system audio and switches audio recording off; explicitly enable recording again if needed.

Under Settings → Save & export, choose whether to save subtitles or record system audio. Both switches are off by default. When enabled, content is written incrementally to private local session files. Turning an option off clears its current-session content; saved sessions require explicit deletion.

Transcript retention is limited to 10,000 confirmed pairs / 2 MiB of text and audio to 64 MiB. Reaching a limit stops retention and shows a notice. Transcript timestamps mark confirmation time; WAV omits pauses and reconnect gaps, so the two are not synchronized.

## Subtitle display

In Settings, choose one of the five subtitle color swatches (white by default), or use the custom swatch after them to open the desktop color picker. The preview and floating subtitles update immediately, including immersive mode. In bilingual mode, original and translated text use the source’s selected color.

Choose Translation only, Original + translation, or Original only
in Settings, the overlay control panel, or the tray. Switch instantly with
⌘⇧B on macOS or Ctrl+Shift+B on Windows/Linux X11. On Wayland, bind `mimi --cycle-subtitle-display` in desktop settings. This changes what you see
without restarting translation. Bilingual mode pairs confirmed sentences and
previews the recognized original while a translation is pending.

## More docs

[Contributing](../CONTRIBUTING.md) · [Security & privacy](../SECURITY.md) · [Platform differences and verification](development/platform-parity.md)
