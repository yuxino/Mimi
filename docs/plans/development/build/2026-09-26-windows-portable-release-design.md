# Windows extract-and-run release

## Decision

The Windows release job builds Mimi once, verifies its MSI and NSIS updater
signatures, then packages that same `mimi.exe` with a `mimi.portable` marker in
`mimi_<version>_x64-portable.zip`. It extracts the ZIP and compares the executable
hash with the build before staging it. The publish job requires the ZIP in the
exact asset list and includes it in `SHA256SUMS.txt`; `latest.json` continues to
reference only signed installer updater assets.

Settings checks the marker through a backend command before creating the
updater. A marked Windows copy shows a Releases button for manual ZIP updates.
An unavailable mode check also avoids offering an installer on Windows. The
normal installed Windows and macOS update paths remain as they are. Native UI
test mode retains its deterministic updater fixture.

This is installation-free distribution, not a self-contained data mode.
Preferences remain in the app config directory, service credentials remain in
Windows Credential Manager, and exports remain where the user saved them. The
ZIP relies on the system WebView2 runtime. No new audio, subtitle, or credential
storage path is introduced.

## Verification boundary

Automated checks cover the portable update route, release archive contents,
hash parity, asset/checksum inclusion, and a native Windows UI-test launch from
the extracted ZIP without provider networks or system-audio capture. A real
Windows system-audio session and manual replacement of a prior ZIP remain
separate device acceptance checks. No release is created by this source change
alone.
