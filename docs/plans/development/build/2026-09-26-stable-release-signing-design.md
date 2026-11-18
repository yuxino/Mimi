# Fixed macOS release identity

## Problem

Local Mimi packages already use a stable self-signed identity, but tag CI
explicitly used `APPLE_SIGNING_IDENTITY=-` and verification allowed it. The
resulting public app's designated requirement was its changing CDHash. Thus
updates changed the identity associated with recording permission. Kiri's
installed package uses a fixed self-signed certificate; paid Developer ID is
not required just to keep a recording identity stable.

## Decision

Keep the existing `mimi Local Development` private key on the maintainer Mac.
Pin its public certificate fingerprint in `scripts/macos/macos-release-identity.txt`.
Prepare macOS release artifacts locally; keep the separate updater signing
key in the existing Actions secrets. Do not export the code-signing private key or generate another certificate.

The preparation script requires committed source and embeds its commit ID in
the signed Info.plist. Stage the DMG and updater archive in a draft before pushing the version tag.
Tag CI safely extracts the archive, then verifies the pinned certificate,
complete designated requirement, source commit, version, and matching DMG.
Only after these checks does CI sign the archive with the existing updater key
and verify that signature. The existing publish job
still waits for Windows and macOS checks before publishing the bilingual notes
and updater manifest. A missing draft or incorrect signature fails closed.

The expected requirement contains only the bundle identifier and certificate
root fingerprint. Reject ad-hoc signatures and extra build-specific conditions.
Local packaging remains available without the updater private key. Development
keeps its separate bundle ID and `/Applications/mimi-dev.app` installation.

## Boundaries

The first transition from historical ad-hoc releases is an identity migration;
a new recording grant can be needed once. Do not remove certificates, Keychain
credentials, or global privacy grants. Existing installs remain unchanged until
a fixed-signed version is installed. Self-signing does not provide Apple
notarization or stable Apple Team ID Keychain partition behavior. Do not promise
that it eliminates API-key access prompts or every macOS consent request.

## Verification

Automated tests reject ad-hoc signatures, wrong certificates, changing
requirements, wrong signed source/version, missing identities, and unsafe
archive entries. Local packaging must verify both the app and its DMG. A second
locally signed copy with changed signed metadata can establish that CDHash
changes while the designated requirement remains constant. This is signature
continuity evidence, not proof of a real installed update preserving capture.
A public release and native capture/update acceptance must be reported separately.
