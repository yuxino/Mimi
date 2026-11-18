# Intel macOS distribution

## Scope and validation boundary

Provide Intel Mac builds on macOS 13 or later, alongside the existing Apple
silicon and Windows packages. The initial Intel QA build passed compilation,
repository checks, stable signing and DMG consistency verification. The owner
then authorized formal distribution without Intel hardware acceptance; release
notes must explicitly disclose that installation, capture and permissions on
Intel hardware remain unverified. Preserve all existing privacy and credential
constraints.

## Packaging and updates

Keep architecture-specific DMGs. A universal app would increase download size
and still require validation of both architectures. Local QA uses
`./scripts/package-app.sh --target x86_64-apple-darwin`.

For public releases, the Apple silicon signing Mac runs
`./scripts/macos/prepare-macos-release.sh` to build both architectures sequentially
with the pinned identity and signed source revision. Native ARM builds reuse
the established cache and `mimi.app.tar.gz` name. Intel uses its own Cargo target
directory, `mimi_VERSION_x64.dmg` and `mimi_x64.app.tar.gz`; both archives contain
an app named `mimi.app`. No installed app is replaced.

CI verifies each architecture, source/version marker, stable requirement and
matching DMG before signing its updater archive with the existing update key.
Publication requires both Mac architectures and Windows assets. The manifest
maps `darwin-aarch64` and `darwin-x86_64` to separate URLs and signatures.
Missing assets, swapped architectures, URLs or signatures must fail closed.

## Verification

Run the canonical checks, focused manifest/signing regressions, both real
package builds, and public asset/checksum/updater verification. Keep package
proof separate from Intel hardware acceptance. A future Intel hardware check
should cover installation and normal OS approval, key storage/readback, system
sound translation, pause/resume/stop, and the overlay above full-screen video.
Keep all captured diagnostics content-free. Do not claim notarization or
password-free Keychain continuity from the stable self-signed identity.
