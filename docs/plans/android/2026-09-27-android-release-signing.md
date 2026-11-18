# Android release packaging

The previous Android release workflow built assembleDebug, kept the internal
version at 0.1.0, and raced the immutable desktop release to upload an attachment.
Renaming that artifact does not make it a release build.

Android owns android/version.properties; increment versionCode on each public
update. CI tests and lints both variants and builds the unsigned release without
signing secrets. android/sign-release.sh accepts only the expected compiled
package/version with debugging disabled, aligns it, signs with an existing key
and verifies the pinned certificate and alignment before exposing the output.
It never generates a key or falls back to debug signing.

From v1.5.2, publishing uses the shared v* tag and one release for Android and
all desktop platforms. The reusable Android workflow only produces a verified
signed APK artifact. The main publisher waits for it and all desktop builds,
requires the APK in its exact asset inventory, includes it in SHA256SUMS.txt,
and uploads everything to the same draft before immutable publication.
Android versionName must equal the desktop version; versionCode increases for
updates. Existing releases fail closed. The desktop updater remains limited to
its four supported desktop targets; Android users install the same-key APK.

The first release requires a backed-up Android keystore, four repository secrets
documented in android/README.md, and its public certificate SHA-256 variable.
Missing credentials fail before building. Never commit or log private signing
material, or include it in build artifacts. All future APKs must use the same
identity. Debug installations require an explicit uninstall; the bilingual
release notes explain the resulting settings/credential loss.
