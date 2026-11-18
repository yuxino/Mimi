#!/usr/bin/env bash
# Prints the unique code-signing identity for local mimi builds, or fails when
# no stable identity exists. Selection order:
#   1. MIMI_CODESIGN_IDENTITY (explicit override)
#   2. the formal app pin, or the separate dev app pin with --development
#   3. the SHA-1 fingerprint of the one self-signed
#      "mimi Local Development" identity in the login keychain
#   4. fail closed (no ad-hoc fallback)
#
# Ad-hoc signatures change on every build (the cdhash is derived from the
# binary), which makes macOS forget Screen & System Audio Recording grants.
# A stable identity keeps the designated requirement identical across builds
# so TCC can track the app. File-based Keychain access has an additional
# partition check; a self-signed identity without an Apple Team ID cannot
# promise password-free access across every rebuilt binary. Selecting by
# fingerprint also prevents two same-named certificates from being chosen
# nondeterministically. See
# docs/development/common-regressions.md.

set -euo pipefail

DEVELOPMENT=0
case "$#:${1:-}" in
  0:) ;;
  1:--development) DEVELOPMENT=1 ;;
  *)
    echo "Usage: $0 [--development]" >&2
    exit 2
    ;;
esac

if [[ -n "${MIMI_CODESIGN_IDENTITY:-}" ]]; then
  if [[ "$MIMI_CODESIGN_IDENTITY" == "-" ]]; then
    echo "error: ad-hoc signing is forbidden for Mimi." >&2
    exit 1
  fi
  echo "$MIMI_CODESIGN_IDENTITY"
  exit 0
fi

IDENTITY_LIST="$(security find-identity -v -p codesigning 2>/dev/null || true)"
# This file stores a public certificate fingerprint, never a private key or API
# credential. A missing/expired pinned certificate must not silently fall back
# to self-signing and change the installed app's identity again.
# Dev and formal bundles have separate TCC identities. A formal certificate
# migration must not change the next development build's selected certificate.
if [[ "$DEVELOPMENT" == "1" ]]; then
  LOCAL_IDENTITY_FILE="${MIMI_DEV_CODESIGN_IDENTITY_FILE:-$HOME/Library/Application Support/app.yuxino.mimi.dev/local-codesign-identity.txt}"
else
  LOCAL_IDENTITY_FILE="${MIMI_LOCAL_CODESIGN_IDENTITY_FILE:-$HOME/Library/Application Support/app.yuxino.mimi/local-codesign-identity.txt}"
fi
if [[ -e "$LOCAL_IDENTITY_FILE" || -L "$LOCAL_IDENTITY_FILE" ]]; then
  [[ -f "$LOCAL_IDENTITY_FILE" && ! -L "$LOCAL_IDENTITY_FILE" ]] || {
    echo "error: the local signing pin must be a regular file." >&2
    exit 1
  }
  LOCAL_PIN="$(tr -d '[:space:]' < "$LOCAL_IDENTITY_FILE" | tr '[:lower:]' '[:upper:]')"
  [[ "$LOCAL_PIN" =~ ^[0-9A-F]{40}$ ]] || {
    echo "error: invalid local signing certificate fingerprint." >&2
    exit 1
  }
  AVAILABLE_PINS="$(printf '%s\n' "$IDENTITY_LIST" | sed -n \
    's/^[[:space:]]*[0-9][0-9]*) \([0-9A-Fa-f][0-9A-Fa-f]*\) ".*"$/\1/p' | tr '[:lower:]' '[:upper:]')"
  PIN_MATCHES="$(printf '%s\n' "$AVAILABLE_PINS" | awk -v pin="$LOCAL_PIN" '$0 == pin { count += 1 } END { print count + 0 }')"
  [[ "$PIN_MATCHES" == 1 ]] || {
    echo "error: the pinned local signing certificate is unavailable or ambiguous." >&2
    exit 1
  }
  printf '%s\n' "$LOCAL_PIN"
  exit 0
fi
MATCHING_IDENTITIES="$({
  printf '%s\n' "$IDENTITY_LIST" \
    | /usr/bin/sed -n \
      's/^[[:space:]]*[0-9][0-9]*) \([0-9A-Fa-f][0-9A-Fa-f]*\) "mimi Local Development"$/\1/p'
} || true)"
MATCHING_COUNT="$(
  printf '%s\n' "$MATCHING_IDENTITIES" \
    | /usr/bin/awk 'NF { count += 1 } END { print count + 0 }'
)"

case "$MATCHING_COUNT" in
  0)
    echo "error: no stable mimi Local Development signing identity is available." >&2
    exit 1
    ;;
  1)
    printf '%s\n' "$MATCHING_IDENTITIES" | /usr/bin/tr '[:lower:]' '[:upper:]'
    ;;
  *)
    echo "error: multiple valid code-signing identities are named mimi Local Development." >&2
    echo "Remove the duplicate identity or set MIMI_CODESIGN_IDENTITY to one fingerprint." >&2
    exit 1
    ;;
esac
