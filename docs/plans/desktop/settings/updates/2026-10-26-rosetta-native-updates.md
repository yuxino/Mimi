# Rosetta to native macOS updates

## Accepted scope

Mimi already ships separate Apple silicon and Intel packages. Tauri's default
updater chooses by the running binary's compile target, so an Intel package on
an Apple silicon Mac otherwise keeps receiving Intel updates. The macOS 28
Rosetta warning motivates a native migration path; the warning alone is not
proof that a currently installed ARM app is incompatible.

## Policy

- Only a macOS x86_64 binary whose `sysctl.proc_translated` query succeeds with
  an exact integer result of 1 selects `darwin-aarch64`.
- A failed query, malformed result, native ARM process, real Intel host or other
  platform keeps the default updater target and strict version comparison.
- A confirmed migration may install a same-version native package, allowing the
  first release containing the fix to complete migration without waiting for
  another release. Older versions remain rejected. After restarting as ARM,
  ordinary strict version comparison resumes.
- Preserve updater signature validation, explicit installation/restart actions,
  bundle identifier, signing identity, configuration and credentials. Do not
  modify an active formal installation during verification.

## Verification

Cover platform/architecture and query-result boundaries, equal/newer/older
versions, and native versus Rosetta execution of the detection code. Compile
both macOS targets. Full in-place update acceptance and macOS 28 execution
remain separate from policy tests; do not claim them from those tests.

## Existing installations

Already-shipped binaries cannot gain the new target policy until they receive a
release containing it. The existing Apple silicon DMG is the immediate manual
path. Historical Intel QA launches may also leave compatibility records after
returning to ARM; follow [the local recovery guidance](../../../../development/common-regressions.md#rosetta-compatibility-warnings-on-an-apple-silicon-mac).
