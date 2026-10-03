# Shared subtitle JNI build

Desktop and Android use `../mimi-core`. This crate only carries JSON strings
across JNI: each call provides its bounded serialized state and receives the
next state and snapshot. It owns no reducers, registries, audio, credentials or
network clients. Kotlin loads it without a fallback implementation.

The adapter pins `jni` to 0.21.1, already present in the desktop dependency
lock. Its new direct use is required to execute the same Rust subtitle reducer
on Android rather than maintaining a second Kotlin implementation.

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
`python3` executable. `scripts/check-shared-core.sh` checks both Rust crates.

## Native dependency notices

`android/native-licenses/shared-core.txt` records the locked JNI/Serde native
dependency versions and their upstream license texts. Gradle packages it as
an APK asset; artifact verification rejects an APK without these notices.
