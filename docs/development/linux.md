# Linux

Mimi's Linux packages target x86_64, built on Ubuntu 22.04. Use a desktop
session with PulseAudio or PipeWire's PulseAudio compatibility server
(`pipewire-pulse`) and a working default playback device. API keys use the
desktop's Secret Service (such as GNOME Keyring); unlock its login collection
before saving credentials. There is no file or environment-variable fallback.

## Install and update

Download the `.deb` or `.AppImage` from [Releases](https://github.com/yuxino/mimi/releases/latest).

- Ubuntu/Debian: install the downloaded `.deb` with the system package
  installer, or `sudo apt install ./mimi_<version>_amd64.deb`.
- AppImage: make the downloaded file executable and open it. Older FUSE-based
  AppImage launchers need FUSE 2 support (`libfuse2` on Ubuntu 22.04).
  The desktop must provide its normal EGL/GLVND graphics stack and GPU drivers.
  New packages built with the GLES packaging fix include the vendor-neutral
  entry library that WebKit loads dynamically. Existing v1.5.6/v1.5.7 packages
  do not include this fix. If they report `Couldn't open libGLESv2.so.2`,
  install your distribution's `libgles2` package
  (`sudo apt install libgles2` on Ubuntu/Debian).
- AppImage supports signed updates in Settings. Quit before updating `.deb`
  with a newer package; its Settings button opens Releases.

Mimi opens Settings at startup, so a tray extension is optional. Minimize
Settings to keep subtitles running; closing Settings exits Mimi on Linux. X11 is
recommended for the complete overlay experience. Wayland window placement,
always-on-top, and click-through depend on the compositor. On Wayland,
configure system keyboard shortcuts as described below or use Settings controls. No Linux ARM64 package
is currently produced.

## Keyboard shortcuts

On X11, Mimi registers Ctrl+Shift+Space (start/stop), Ctrl+Shift+M (Immersive
Mode), and Ctrl+Shift+B (subtitle display). On Wayland, assign commands in
your desktop's keyboard settings. Mimi does not install desktop bindings or
claim that an XWayland key grab is a working Wayland shortcut.

For a `.deb` installation:

| Action | Command |
| --- | --- |
| Start or stop subtitles | `mimi --toggle-session` |
| Toggle Immersive Mode | `mimi --toggle-immersive` |
| Cycle translation, bilingual, and original subtitles | `mimi --cycle-subtitle-display` |

On GNOME, open Settings → Keyboard → View and Customize Shortcuts → Custom
Shortcuts. Add an entry, paste the command, and choose an available key
combination. Mimi's Settings shows commands for the current installation.
AppImage users must use the quoted absolute AppImage path instead of `mimi`;
update the binding after moving or renaming the AppImage.

A command controls the running Mimi instance. When Mimi is closed, it opens
Mimi and performs the action; starting subtitles still requires a configured
service. Normal repeated launches bring the existing Settings window forward.
Desktop commands are debounced, and start/stop is ignored while connecting or
stopping, just like the native session shortcut.

Installed `.deb` and AppImage CI tests verify command forwarding, start/stop,
Immersive Mode, and absence of duplicate Settings windows under Xvfb. This
checks the command path, not GNOME/KDE shortcut setup or native Wayland focus.

## Audio and troubleshooting

System audio (the default) opens only the current default output's monitor.
Mimi verifies the source belongs to that output and never falls back to a
microphone. Enabling Microphone in Settings opens the default non-monitor
input; an output monitor is rejected. Both sources can be enabled together,
with independent capture and recognition sessions. With both enabled, automatic
echo cancellation reduces system playback in the microphone track before
recognition and optional recording. Restart the session after
changing the default device. A missing source
or disconnected sound server is an error, not permission to capture another
source.

If capture fails, check that the sound server is running and that the selected
system output or default microphone works in the system sound settings.

### Credential recovery

Mimi needs a compatible [Secret Service provider](https://specifications.freedesktop.org/secret-service/latest/)
on the current desktop's session D-Bus. A running D-Bus session or an installed
wallet executable alone does not provide this service.

- **Cannot connect to Secret Service:** use your distribution's package manager
  to install a compatible provider, such as GNOME Keyring, and enable its
  desktop-session integration. Follow your distribution's login setup; sign
  out and back in if it requires a fresh session. Installing Mimi's `.deb`
  normally installs its `gnome-keyring` dependency, but extracting the package
  manually bypasses that step. AppImage users must provide the service too.
- **Storage locked or access denied:** unlock the collection in your desktop's
  password manager and handle any system authorization prompt. A dismissed
  prompt is not a missing API key. GNOME Keyring can unlock the login collection
  through the distribution's [login integration](https://wiki.gnome.org/Projects/GnomeKeyring/Pam).
- **No credentials configured:** once storage is accessible, save the service's
  credentials in Mimi. A reachable server does not establish authentication.

Use **Check connection** after recovery. It retries failed credential reads
without restarting Mimi. Failed saves and connection checks retain unsaved
editor values; retry **Save** once the service is accessible. Save retries its
own failed credential reads too. If the service has no default collection,
the first Save can open the desktop keyring's normal collection-creation
dialog. Complete that system dialog; cancelling leaves the credentials unsaved
and allows another explicit Save. Reading settings never creates a collection.
Mimi does not
install or configure a keyring, change its password or permissions, or use file
or environment-variable credential fallbacks.

## Build and verification

Install Rust 1.88+, Node.js 22.13+, and the native dependencies listed by
`scripts/linux-ci-deps.sh` (Ubuntu). Then:

```bash
npm ci
./scripts/check.sh
./scripts/build-linux-packages.sh --config src-tauri/tauri.ci.conf.json
```

The CI config disables updater signing for development bundles. Formal release
builds use the existing signing key and verify the AppImage signature before
publishing. CI also exercises generated audio through an isolated null output,
an isolated Secret Service round trip, and three independent credential-free
Xvfb launches of each installed package format. Any failed launch stops the
check; it is not retried into a passing result.

Native validation also runs in an isolated Ubuntu 22.04.5 ARM64 virtual machine:
PulseAudio 15 and PipeWire 0.3.48 with WirePlumber 0.4.8 both capture a generated
997 Hz tone through the selected output monitor at 16/24 kHz, including
stop/restart with an unrelated default input. GNOME Keyring exercises real
save/read/update/delete operations with a synthetic credential. Locally built
ARM64 `.deb` and AppImage packages are for this validation only; the public
release target remains x86_64.

The VM's Openbox X11 desktop uses a compositor for transparent windows. It
also checks normal installed-app startup, language switching, privacy
defaults, and the `.deb` update entry. Xlib threading is initialized before
GTK starts, avoiding an intermittent native startup abort found by this test.
An installed production-mode build also connected to Alibaba Cloud using an
isolated Secret Service credential, captured generated English speech from
the system output, and displayed Chinese translations. This is a functional
check, not a provider latency benchmark. Physical Linux audio hardware, public
x86_64 artifacts, and other GNOME/KDE/Wayland compositors remain separate
acceptance checks.

### AppImage launcher permissions

Use `scripts/build-linux-packages.sh` for x86_64 Linux packages, including
signed release builds. It prepares Tauri's upstream AppRun cache with mode
755 before bundling and updater signing. Tauri otherwise creates it as 770;
linuxdeploy preserves that mode as `AppRun.wrapped`, blocking execution by
users outside the build owner's UID/group (reported in #66).
`verify-linux-bundles.sh` extracts the final AppImage and checks read/execute
bits for owner, group, and other on both launchers and `usr/bin/mimi` before
running the existing tray-free X11 smoke tests. It also checks the bundled
`libGLESv2.so.2` and its copyright notice, independently of libraries installed
on the build host. Only the GLVND entry library is included; EGL, GLdispatch,
and Mesa/NVIDIA GPU drivers remain supplied by the host.
FUSE availability, other runtime dependencies, and glibc compatibility remain
separate checks.

The isolated first-save test (`scripts/linux-keyring-first-save-smoke.sh`)
starts with no default collection, cancels then accepts the real GNOME Keyring
creation prompt using a synthetic password, and verifies the saved test value
in a fresh process. It does not access the desktop user's keyring or test PAM
login integration. Existing round-trip and late-service recovery tests remain
separate checks.

### Native Wayland input-region regression

`./scripts/linux-wayland-input-smoke.sh` runs an isolated GTK input-region
fixture under headless Weston. It requires the development dependencies and
Weston, and refuses an unavailable compositor rather than using an existing
desktop or X11. It checks empty regions while locked, default input restoration
while unlocked, and hide/show, unrealize/realize, and recreation through actual
Wayland protocol commits. No provider, audio capture, keyring, desktop input
permission, or accessibility authorization is involved.

This is a protocol regression for Mimi's Linux input-shape helper, not a
GNOME/KDE desktop or packaged-app acceptance result. Global placement,
always-on-top, workspace behavior, real underlying-window click delivery, and
compositor-specific shortcuts remain tracked in #92.
