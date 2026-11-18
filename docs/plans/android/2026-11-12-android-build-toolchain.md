# Android build toolchain update

Use AGP 9.5.1, Gradle 9.6.0 and Build Tools 36.0.0, following the
[official compatibility table](https://developer.android.com/build/releases/agp-9-5-0-release-notes).
Use AGP's built-in Kotlin 2.2.10 instead of the separate Kotlin plugin. Keep
JDK/Java target 17, compile/target SDK 35, minimum SDK 29, the pinned NDK and
four ABIs. Preserve application IDs, signing, credential storage and UI libraries.
The updated Kotlin compiler/stdlib and R8 require actual release acceptance;
an upgrade alone does not establish a size, memory or build-speed improvement.

Explicitly retain Debug resource values and both variants' JVM/JNI tests under
the new defaults. Use the current source-directory DSL for assets and JNI,
keeping native task dependencies and output paths aligned. Package only the
shared root JSON files in a test-runtime-only fixture JAR: this replaces the
removed source-set include filter without exposing Rust sources or build caches.
Both test variants consume the same generated artifact through its task provider.

Kotlin 2.1.20+ prioritizes Android's JSON stubs over the JVM test dependency
([KT-76891](https://youtrack.jetbrains.com/issue/KT-76891)). Prepend the existing
`org.json:json` artifact only for the two unit-test Kotlin compilation tasks,
using configuration-time providers and preserving all other classpath inputs.
Keep the existing structural JSON assertions and production compilation intact.

Generate the wrapper using Gradle and verify its published distribution and JAR
checksums. Keep the generated Windows script's CRLF working-tree form through
Git attributes. SDK installation requires accepted Google terms; updating the
wrapper does not authorize installing unrelated SDK platforms or toolchains.

Acceptance covers both test variants, lints, native packaging/mapping guards,
signed release rejection cases and isolated release UI/JNI checks. Use one
signer for baseline-to-candidate synthetic credential tests, then verify fresh
master-key creation. Compare artifacts from the same application source and
retain the limitations of emulator or offline measurements in the run ledger.
