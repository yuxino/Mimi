# Android interface language

Android settings expose a visible interface-language picker above the service
and appearance sections. It offers Follow system, 简体中文, English and 日本語;
language self-names stay readable when the current interface is unfamiliar.
This choice is independent of speech recognition and subtitle translation.

Use the existing AppCompat 1.7 per-app language API. Android 13+ owns the saved
choice and synchronizes it with the system App language screen through an
explicit locale configuration. AppCompat auto-storage handles Android 10–12;
there is no second preference or credential-storage dependency. Activity
recreation refreshes strings and preserves the selected settings tab.

Refresh service-owned overlay controls and notification labels in place when
the interface language changes, preserving the capture session, subtitle state
and reading position. Before Android 13, service UI uses an explicitly localized
context because AppCompat applies locale overrides to activity contexts only.

Native checks use blank API 32 and API 35 emulators: exercise all four choices,
back navigation, service editors, light/dark and narrow/large-font layouts, and
force-stop/relaunch persistence. They must not access real credentials, start
capture or call providers. The owning API contract is documented in
[Android's per-app language guide](https://developer.android.com/guide/topics/resources/app-languages).
