# Android manual update check

Android's More page shows the installed version and a manual **Check for updates**
action. It reads the public GitHub latest-release endpoint for `yuxino/Mimi`,
without credentials. It never checks on launch or starts a download by itself.

A newer stable numeric `major.minor.patch` release must include exactly one
uploaded, non-empty `mimi_<version>_android.apk`. Its release URL and download URL
must match that repository and version. Drafts, previews, malformed metadata,
missing Android assets and failed requests produce a retryable error beside the
check action. Equal or older remote versions show a transient up-to-date toast;
they never offer a downgrade. Debug builds keep their existing numeric
`versionName`; an unknown version is not guessed or coerced.

The available version and **Download APK** action remain beside the check.
That action opens the validated official asset URL in the user's browser. The
browser and Android own downloading and installation; this feature does not add
an installer permission, automatic installation, or a signature-bypass path.
It does not copy desktop updater behavior into the provider runtime.

The native control owns one cancellable request. Navigation away, backgrounding
and destruction cancel it and invalidate its generation. Delayed completion
cannot change a hidden or newly checked page. Kotlin captures platform trust and
system proxy routing without service configuration, then calls the existing JNI
exchange. A bounded native registry owns at most two update tickets and drops or
aborts each on completion/cancellation. The shared HTTP builder keeps redirects
disabled; the request has a five-second connect timeout, a fifteen-second total
timeout and a 256 KiB response cap. Kotlin never creates a second HTTP transport.
No networking dependency or architecture-check exception is added. All nine UI
languages use equivalent native resources.

Focused Rust and JVM tests cover bounded responses, request ownership, version
ordering, release eligibility and destination validation. Actual JNI fixtures
keep transport on loopback. Native instrumentation covers busy state, repeat clicks, navigation
cancellation, stale callbacks, retry, available/current states and browser
handoff with synthetic results. Those fixtures make no external request, start
no capture and do not establish successful APK installation.

Reference: [GitHub latest-release API](https://docs.github.com/en/rest/releases/releases#get-the-latest-release).

The HTTP builder initializes the existing Rustls ring provider itself. An
update check may be the first network operation in a fresh process; it must
not depend on opening a translation session first. A fresh-process Rust
regression and the standalone JNI update tests cover that initialization.
