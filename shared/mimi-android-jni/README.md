# Shared subtitle JNI build

Desktop and Android use `../mimi-core` for pure subtitle rules and
`../mimi-runtime` for the actual provider transports and session controller.
The stateless core API accepts a bounded serialized reducer state. The live API
owns bounded session/check handles and accepts PCM from native capture. Kotlin
loads the library without a fallback implementation.

Build requirements: Rust 1.88 or later, Python 3, JDK 17, and the Android SDK
platform/build-tools versions documented in `android/README.md`. Native builds
also require NDK **27.2.12479018**, pinned with minSdk and ABIs in
`android/shared-core.properties`.

```sh
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" "ndk;27.2.12479018"
cd android
./gradlew testDebugUnitTest testReleaseUnitTest assembleDebug assembleRelease
```

Unit tests load an actual host JNI library using a generated
`java.library.path`. Android assembly builds and packages arm64-v8a,
armeabi-v7a, x86_64 and x86 libraries, then verifies their ELF architecture,
16 KB LOAD alignment and uncompressed APK ZIP alignment. Missing toolchains,
native entry points or libraries fail the build. Build outputs and Rust caches
remain ignored under `android/app/build` and `shared/target`.

NDK r27 needs explicit `max-page-size=16384` linker flags; the build script
also sets `common-page-size=16384`. AGP 8.7 uses uncompressed native libraries
with 16 KB APK alignment. See the official
[16 KB page-size guidance](https://developer.android.com/guide/practices/page-sizes)
and [NDK toolchain integration](https://developer.android.com/ndk/guides/other_build_systems).
Artifact alignment does not establish execution on a 16 KB device; capture and
device runtime validation remain separate checks.

For an independent build without Gradle, run `scripts/build-shared-core.py`
with `--platform host|android`, `--profile debug|release`, and `--output DIR`.
Android builds use the pinned NDK under `ANDROID_HOME`, or the explicit
`--ndk DIR`. `MIMI_PYTHON` can select Python for Gradle on hosts without a
`python3` executable. `scripts/check-shared-core.sh` checks all shared Rust crates.

## Native dependency notices

`android/native-licenses/shared-core.txt` records the locked shared-runtime native
dependency versions and their upstream license texts. Gradle packages it as
an APK asset; artifact verification rejects an APK without these notices.


Live sessions also expose `NativeRuntimeConfiguration.exchangeRaw/pcmRaw`.
They construct `mimi-runtime`'s real desktop provider factory and publish the
same `TranslationSessionController` snapshot. The adapter bounds active live
handles and text-check handles to two each; stop/cancel removes a handle and
aborts its work. PCM uses the shared bounded desktop queue. A separate stateless
`SharedSubtitleCore` exchange remains for pure contracts and UI fixtures.

Android injects immutable platform trust roots and resolved speech/text proxy
routes. No fallback trust store, alternate Kotlin provider or transcript log is
used. Credential fields remain private to active sessions/checks. Text checks
are explicit user actions, share PC connection diagnostics, and can be cancelled.
