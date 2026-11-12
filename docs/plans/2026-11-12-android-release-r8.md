# Android release shrinking and mapping retention

Release builds enable R8 and resource shrinking with Android's default
`proguard-android-optimize.txt`. Debug remains unminified. The four supported
ABIs, nine UI languages, minimum API 29 and pinned Kotlin, AGP, SDK, NDK and
Gradle versions stay unchanged. No broad keep rule or extra dependency is added.

Release APK verification declares the generated mapping as a required Gradle
input and passes it explicitly. The packaging guard retains direct DEX,
four-ABI, ELF/ZIP alignment and license checks. It also checks original class
headers and qualified method/field owners in the mapping, including code moved
by inlining. A missing, stale or invalid mapping fails the build rather than
disabling the architecture check for optimized code.

The mapping body hash covers the original bytes after its hash-marker LF. The
default content-derived ID and compiler/minimum-API metadata must match the
APK's R8 marker. This is precise build pairing under R8's format, not complete
cryptographic provenance: the DEX marker contains a short ID, not the full
mapping hash. Conflicting R8 markers fail closed pending inspection of a real
optimized artifact. The existing source architecture guard remains required.

`android/sign-release.sh [apk] [mapping]` defaults to the canonical release
outputs. It snapshots the supplied mapping, validates that same copy before and
after signing, then retains it as `mimi_<version>_android.mapping.txt` beside the
signed APK. Local and unified release SHA-256 inventories include both files.
The unified Android artifact includes the pair; ordinary Android CI uploads
mapping separately to preserve the unsigned APK artifact's flat layout.
The mapping is a release asset for future stack-trace retracing, not an updater
payload. Updater targets, signing identities, tag checks, draft staging and
separate publication authorization remain unchanged.

Acceptance requires actual release build/guard checks and signed-artifact
verification in addition to the synthetic mapping regressions. Compare size
against the same unminified input and inspect native UI, JNI entry points and
all supported locales. Python or shell checks alone do not prove R8 retention
or runtime behavior. No tag or public release is created by this change.
