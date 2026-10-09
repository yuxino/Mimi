# Using Mimi

[Back to README](../README.md) · [简体中文](usage.zh-CN.md)

Provider activation links, credential fields, and current models are in the [provider setup guide](provider-setup.md).

Choose the desktop interface language in Settings → General: Simplified Chinese, Traditional Chinese, English, Japanese, German, Korean or French. The system option follows your system language; changing the interface language keeps current edits and subtitle state. Recognition and translation languages are separate choices.

## Install and update

1. Download the macOS Apple silicon or Intel DMG, a Windows x64 EXE, MSI, or portable ZIP, or a Linux x86_64 .deb / AppImage from the [latest release](https://github.com/yuxino/mimi/releases/latest), or build from source.
2. Open Settings → Speech & Translation, add a configuration, enter the requested provider credentials, and save. Choose the recognition and translation languages.
3. Play something and turn on Live Subtitles under Subtitles. You can also start from the mimi menu bar/system tray icon. On macOS 14.2+, capturing All Applications supports System Audio Recording Only. Existing screen grants are reused without requesting a separate audio-only grant. Capturing a selected app or using macOS 13–14.1 still requires Screen & System Audio Recording.

Bring your own provider API credentials; usage charges may apply. Desktop credentials are stored in a private plaintext file protected by local file permissions.

For macOS, Windows installer copies, and Linux AppImage, open Settings → General → Software Update. Mimi downloads the
update with progress, then lets you install it. Windows reopens Mimi after
installation; macOS and Linux AppImage offer a separate Restart and Finish Update action.
Versions older than v1.3.8 need one manual installation to enable in-app updates.

See [speech-service setup and language parameters](speech-language-setup.md) for custom WebSocket examples, automatic detection versus hints, and each integration’s current language range.

### Platform support

- macOS 13+ (Apple silicon and Intel): Choose the `_aarch64.dmg` for Apple silicon or `_x64.dmg` for Intel. Intel packages are available from v1.4.4; build and signing checks passed, but Intel hardware capture and permission behavior remain unverified. DMG installers are not Apple-notarized. If first launch is blocked, choose Open Anyway in System Settings → Privacy & Security. See the permission notes below when upgrading from an older build.
- Windows x64: Unsigned preview EXE / MSI installers and, since v1.4.3, a portable ZIP are available; SmartScreen may warn. Extract `mimi_<version>_x64-portable.zip` and launch `mimi.exe` without installation. WebView2 must already be installed (it is normally present on Windows 11). The ZIP does not move settings, service credentials, or exported files into its folder; those remain in their existing user-selected or OS-managed locations. Update this copy by quitting Mimi and replacing it with a new ZIP from Releases. The portable build does not run the in-app installer updater.

- Linux x86_64 preview (Ubuntu 22.04+ baseline): Use the `.deb` package or AppImage. Requires PulseAudio or PipeWire with `pipewire-pulse`, a working default output device, and private local credential-file access. System audio uses only the output monitor. Restart the session after changing output devices. X11 is recommended; Wayland compositors may restrict positioning, always-on-top, and click-through. For Wayland keyboard shortcuts, use the commands shown in Settings to create system shortcuts. Use Settings if your desktop does not show a tray icon. Minimize it to keep subtitles running; closing it exits Mimi on Linux. Linux ARM64 packages are not provided. See [Linux setup and verification](development/linux.md).

### macOS system-audio permissions

For All Applications on macOS 14.2+, new installations request System Audio Recording Only. To switch an existing installation, quit Mimi, remove or disable its Screen & System Audio Recording entry in System Settings → Privacy & Security, then reopen Mimi and start subtitles. Allow System Audio Recording Only when asked, or enable Mimi under that section. Restart Mimi if macOS requests it. Switching is optional; existing screen authorization continues to work. Selected-application capture still requires the broader grant.

### macOS permissions after an update

The release pipeline now requires one fixed self-signed certificate across macOS builds. Earlier releases through v1.4.1 used ad-hoc signatures that changed with each build; installing the first release with the fixed identity may require one new recording grant. This source change does not alter an already installed app. Fixed signing prevents build-specific identity changes; it does not promise that macOS will never request consent again. Deleting a local signing certificate does not change the installed app's identity or fix this. Keep the release app at `/Applications/mimi.app`; use `/Applications/mimi-dev.app` for development. Each app keeps its own signing pin and recording authorization; a development build must not inherit a release app's changed identity.

Recording is enabled, but Mimi still reports permission denied: first quit and reopen Mimi and follow the normal permission prompt. If capture is still denied:

1. Quit the affected app. In System Settings → Privacy & Security → Screen & System Audio Recording (the label varies by macOS version), select and remove only its old entry: **mimi** for the release app or **mimi-dev** for development.
2. Use + to add the matching `/Applications/mimi.app` or `/Applications/mimi-dev.app`, enable its permission, and complete any system authentication yourself. Leave the other Mimi app and unrelated entries alone.
3. Reopen that same app, start subtitles with the shortcut or the Start control, and play audio containing speech to check that subtitles actually appear. An enabled switch alone does not confirm recovery.

This removes an old recording authorization, not a certificate or API key. If one attempt does not help, stop repeating the reset and [report the error](https://github.com/yuxino/mimi/issues), including your macOS and Mimi versions and installation source. Do not include API keys or subtitle content.

Desktop credentials are stored in a private plaintext file protected by file permissions. Existing OS credentials are automatically imported once and deleted after the saved copy is verified. An old-store authorization may be needed for that import; after it completes, switching, saving and updating use only the file. Missing or damaged local credentials do not fall back to Keychain. A `codesign` request for a signing private key is a separate build-time prompt.

## Service configurations

Each desktop service configuration can remember a recognition/translation language pair. Activate the configuration and choose the languages, then open its details in Settings → Speech & Translation and use **Remember current languages**. To change or remove the saved pair, use **Update to current languages** or **Remove saved pair** explicitly.

Switching to or reapplying a configuration restores its saved pair. Temporary language changes do not overwrite it. Tray and floating menus show the saved pair below the configuration name. Click the service information in the subtitle header to open the current configuration details.

## Audio and saved sessions

Desktop uses system audio by default. In Settings or the floating controls, select system audio, microphone, or both. Microphone capture uses the system's default microphone and requests permission when capture starts, if needed. With both selected, each input has its own recognition connection, subtitles and service usage. Changing inputs turns audio recording off; enable it again in Save & export if needed.

On macOS and Windows build 20348+, use the application picker to translate a selected app's audio. Linux captures the system output monitor and does not offer this picker.

Under Settings → Save & export, choose whether to save subtitles or record the selected audio inputs. Both switches are off by default. When enabled, content is written incrementally to private local session files; system and microphone audio stay separate. Turning an option off clears its current-session content; saved sessions require explicit deletion.

Starting another desktop session keeps the bounded confirmed subtitles already on screen. Clear removes them. This does not enable transcript saving, copy old lines into the new session's saved transcript, or restore unsaved subtitles after quitting the app.

Transcript retention is limited to 10,000 confirmed pairs / 2 MiB of text and audio to 64 MiB. Reaching a limit stops retention and shows a notice. Transcript timestamps mark confirmation time; WAV omits pauses and reconnect gaps, so the two are not synchronized.

## Subtitle display

In Settings, choose one of the five subtitle color swatches (white by default), or use the custom swatch after them to open the desktop color picker. System audio and microphone have separate color settings. The preview and floating subtitles update immediately, including immersive mode. In bilingual mode, the original stays visually softer than the translation.

Show time is available in subtitle settings and the floating controls, and is off by default. When enabled, it shows local confirmation time (HH:mm:ss) beside confirmed subtitles from both system audio and the microphone, including in immersive mode. This marks when the subtitle was confirmed, not when speech began. Retained microphone lines keep a small source icon when switching back to system audio. The floating controls also provide pause/resume.

Choose Translation only, Original + translation, or Original only
in Settings, the overlay control panel, or the tray. Switch instantly with
⌘⇧B on macOS or Ctrl+Shift+B on Windows/Linux X11. On Wayland, bind `mimi --cycle-subtitle-display` in desktop settings. This changes what you see
without restarting translation. Bilingual mode pairs confirmed sentences and
previews the recognized original while a translation is pending.

When Show subtitles earlier is enabled, the overlay shows drafts that may still
change. Turn it off to wait for confirmed results; uninterrupted speech can take
longer to appear. This display choice does not turn a draft into saved history.

## Recognition errors

Recognition errors stay visible in the subtitle window alongside existing
subtitles. Open the floating control panel for the full explanation and recovery
actions. A recognized configuration error offers Speech & Translation settings;
temporary connection or service errors offer Retry. Correcting or switching the
affected configuration returns the failed session to idle without starting audio.
Start Live Subtitles again when ready.

## More docs

[Contributing](../.github/CONTRIBUTING.md) · [Security & privacy](../.github/SECURITY.md) · [Platform differences and verification](development/platform-parity.md)
