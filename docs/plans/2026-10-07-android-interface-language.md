# Android interface language

Android settings expose a visible interface-language picker above the service
and appearance sections. It offers Follow system and the desktop language catalog:
简体中文, 繁體中文, English, 日本語, Deutsch, 한국어 and Français;
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

English is the default resource fallback, matching desktop for unsupported
system languages. Chinese resources use explicit Hans/Hant qualifiers; script
takes precedence over region, with TW/HK/MO resolving to Traditional Chinese.
Every language includes all native UI strings, including service errors and help.
JUnit compares the picker and Android locale configuration with desktop's
`UI_LANGUAGES`, verifies resource completeness, and preserves format and protocol
tokens. Desktop catalog changes also trigger Android CI.

Native checks use blank API 32 and API 35 emulators: exercise all eight choices,
back navigation, service editors, light/dark and narrow/large-font layouts, and
force-stop/relaunch persistence. They must not access real credentials, start
capture or call providers. The owning API contract is documented in
[Android's per-app language guide](https://developer.android.com/guide/topics/resources/app-languages).
