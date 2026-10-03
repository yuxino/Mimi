# macOS releases with a fixed identity

New macOS releases use the certificate pinned in
   `scripts/macos-release-identity.txt`. The existing private key stays in the
maintainer Mac's Keychain. Do not export it to GitHub or replace it to unblock a
build. The updater signing key is separate and must continue matching the
public key in `src-tauri/tauri.conf.json`.

## Prepare

1. Update all version files, including Android `versionName` and its increasing
   `versionCode`, and `docs/releases/vVERSION.md` on `main`. Release
   notes must contain `## English` followed by `## 中文`, covering the same
   changes and limits. For the first fixed-signed release, explain the possible
   one-time recording grant, unsigned-by-Apple/not-notarized status, and separate
   Keychain prompt limitation. Do not claim that installed updates were tested
   unless they were.
2. Run `./scripts/check.sh` and complete native QA in
   `/Applications/mimi-dev.app`. Commit reviewed source, then push `main` through
   the normal authorized publication flow. Do not push the version tag yet.
3. On the Apple silicon signing Mac, install the Rust `x86_64-apple-darwin`
   target, then run `./scripts/prepare-macos-release.sh` from the clean
   committed checkout. It produces and verifies:

   - `src-tauri/target/release/bundle/dmg/mimi_VERSION_aarch64.dmg`
   - `src-tauri/target/release/bundle/macos/mimi.app.tar.gz`
   - `src-tauri/target/x86_64-apple-darwin/release/bundle/dmg/mimi_VERSION_x64.dmg`
   - `src-tauri/target/x86_64-apple-darwin/release/bundle/macos/mimi_x64.app.tar.gz`

   The app contains the signed `MimiSourceRevision` for that exact commit.
   The default build excludes development debugger commands, private evidence storage and audio observation. Release verification also rejects debugger command/storage markers in the executable. Both archives contain `mimi.app`; separate filenames keep architecture
   updates distinct. CI applies the existing updater signature only after
   verifying the expected architecture, pinned
   app identity, signed source/version and matching DMG. The updater private
   key/password remain in the existing Actions secrets; no local secret export
   or key rotation is needed. A QA build lacks the signed source marker.

The preparation script honors `CARGO_TARGET_DIR` for both build outputs and
verification paths, so managed worktrees can reuse an existing compilation cache.
When this is set, use that target directory for the four upload paths below.

## Stage and publish

Use the exact version, commit, and verified files printed by the preparation
step. These commands upload a draft, then trigger public release CI; run them
only when release publication is authorized:

```bash
version="$(node -p 'require("./package.json").version')"
revision="$(git rev-parse HEAD)"
tag="v$version"
gh release create "$tag" --draft --target "$revision" \
  --title "mimi $tag" --notes-file "docs/releases/$tag.md" \
  "src-tauri/target/release/bundle/dmg/mimi_${version}_aarch64.dmg" \
  src-tauri/target/release/bundle/macos/mimi.app.tar.gz \
  "src-tauri/target/x86_64-apple-darwin/release/bundle/dmg/mimi_${version}_x64.dmg" \
  src-tauri/target/x86_64-apple-darwin/release/bundle/macos/mimi_x64.app.tar.gz
gh release view "$tag" --json isDraft,assets
# Confirm all four uploads completed before triggering tag CI.
git tag "$tag" "$revision"
git push origin "$tag"
```

For an existing draft, inspect its target and assets first and upload only the
reviewed replacements with `gh release upload ... --clobber`; never overwrite
an already published release. If a tag already exists, verify it resolves to
the exact source revision, finish staging, then rerun its failed workflow.
Never move a published tag.

The tag-only macOS job needs `contents: write` because GitHub hides unpublished
drafts from read-only tokens. Ordinary CI remains read-only; the final publish
job still controls publication.

GitHub macOS CI verifies each architecture in a separate matrix job. It safely
extracts the app, checks
its complete pinned identity and signed source/version, and compares its CDHash
with the app inside the DMG. It then creates and verifies the updater signature
with the existing key. On the disposable runner, only the embedded public
certificate matching the pinned fingerprint is trusted for code signing; that
trust cleanup is attempted after verification with a 15-second timeout. If
macOS authorization services do not respond, the disposable VM is discarded
when the job ends. No private code-signing key is imported,
and user machines are not asked to change certificate trust. Windows still
builds in CI. Only the final publish
job may publish the draft only after macOS, Windows, Linux and the signed Android
APK pass. The APK shares this release and its checksum list; do not create an
`android-v*` release. Read back the published
assets, digests and `latest.json`; a local build alone does not establish a
working public update.

## Installation and migration

Keep the formal app at `/Applications/mimi.app`. Compare old and new identities
with `verify-macos-install-identity.sh` before any replacement. An old ad-hoc app
will fail that check: migration must be deliberate and may require a new grant.
If recording stays denied despite an enabled permission, use the bounded
single-app recovery in [common regressions](common-regressions.md).

Fixed self-signing removes build-specific identity changes. It does not provide
Apple notarization or a stable Apple-issued Team ID for Keychain partitions.
Do not delete saved API keys, local signing certificates, or global grants.
