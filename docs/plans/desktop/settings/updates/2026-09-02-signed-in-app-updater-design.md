# Signed in-app updater

## Decision

Mimi uses the official Tauri v2 updater and process plugins for explicit,
user-initiated updates. The updater reads a single static HTTPS endpoint at
`https://github.com/yuxino/mimi/releases/latest/download/latest.json` and uses
the public key embedded in `tauri.conf.json` to verify every downloaded update
before installation. The application never downloads or installs an update in
the background.

The previous GitHub API comparison and browser handoff are superseded. Opening
the fixed GitHub Releases page remains available only after an updater error so
users have a recovery path. A custom Rust downloader was rejected because it
would duplicate Tauri's signature, platform installer, and resource-lifecycle
logic. A dynamic update service was rejected because GitHub Releases already
provides the required static distribution surface without a new service or
credential boundary.

## Interaction and lifecycle

The Settings -> General control owns one update resource and follows this
state machine:

1. `idle -> checking -> current | available | checkError`
2. `available -> downloading -> downloaded | downloadError`
3. `downloaded -> installing -> restartReady | installError` on macOS/Linux
4. `downloaded -> installing` on Windows, where the official installer exits
   Mimi as required by the platform
5. `restartReady -> restarting | restartError` only after the user explicitly
   chooses to relaunch

An available update displays its version and release notes before download.
`Started`, `Progress`, and `Finished` events report real downloaded bytes. A
percentage is shown only when `contentLength` is present and positive;
otherwise the UI announces an indeterminate download and the received byte
count. The `downloaded` state is entered only after `Update.download()`
resolves, because the official plugin verifies the complete artifact signature
before that promise succeeds. Busy states disable repeat actions and use an
atomic polite live region. Errors keep an explicit retry action and the fixed
Releases recovery link.

The component remains mounted while the settings category changes so a
user-started download does not lose its resource or state. Preview and UI-test
mode use deterministic local fixtures and never contact the update endpoint,
download an artifact, install a package, or restart the app.

## Platform behavior

On macOS, installation replaces the current bundle and returns control to the
UI; Mimi then displays **Restart and finish update** and calls the official
process plugin only after the user clicks it. On Windows, Tauri must exit the
running application before invoking the installer. The UI therefore labels the
action **Install and restart Mimi** and passes `restartAfterInstall: true`.
The native updater adapter owns this option, so both initial installation and
an installation retry request the same behavior. With the configured passive
NSIS installer, the official updater passes `/UPDATE /P /R`: Windows displays
installation progress and reopens Mimi after a successful update. The updater
does not restart Windows itself. Linux follows the macOS restart-ready path
if a Linux artifact is added in the future, but the current public matrix
remains macOS Apple Silicon and Windows x64.

## Signing and release contract

`bundle.createUpdaterArtifacts` is enabled. The updater private key and its
password exist only in protected local recovery storage and the
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` GitHub
Actions secrets. The repository and application contain only the public key.
Signature verification cannot be disabled or bypassed.

Release builds produce the existing DMG, NSIS EXE, and MSI plus the official
macOS updater archive and platform signature files. CI stages exact expected
names, rejects duplicates or unexpected updater assets, and generates
`latest.json` with only `darwin-aarch64` and `windows-x86_64`. The manifest
contains the release version, RFC 3339 publication time, release notes, fixed
versioned HTTPS asset URLs, and the literal signature contents. Before a draft
becomes public, CI cryptographically verifies every updater signature with the
embedded public key and checks names, platform keys, URLs, local SHA-256
values, uploaded GitHub digests, and tag-to-commit immutability.
It then verifies the public manifest and every referenced asset can be
downloaded.

The first published signed-updater release is a bootstrap release. The latest
previous public version, v1.3.6, does not contain the updater public key or
plugin and cannot discover it. The v1.3.7 tag never became a published Release
and has no public package assets. The README and bootstrap release notes must
therefore tell existing users to install v1.3.8 manually once; later releases
can use the in-app flow.

## Verification

Pure UI-model tests cover no update, release metadata, known and unknown total
sizes, busy-action protection, retry targets, and restart readiness. A fixture
adapter exercises successful downloads plus signature, network, install, and
relaunch failures without touching a real release. Repository checks cover
Rust, TypeScript, lint, build, and generated capability schemas. Release CI
proves cryptographically valid signed artifacts and manifest production.
Native UI testing uses the
stable `/Applications/mimi-dev.app` path in UI-only mode; exact published
artifact testing is reported separately from any real cross-version upgrade.
