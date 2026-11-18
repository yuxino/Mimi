# Completed updater staging cleanup

Tauri updater 2.11.0 downloads into an in-memory resource. Successful installation
closes that byte resource. macOS and Linux installation use scoped temporary
directories, which normally remove their backups and extraction files on return.
Windows intentionally keeps `mimi-VERSION-updater-RANDOM` directories and exits
before the external installer finishes, leaving the installer and its directory.

On a normal Windows release startup, clean only stable target versions no newer
than the running app. A future target remains available to an unfinished update;
development and UI-test apps never retire production installers. Cleanup runs
off the UI thread and does not alter the installer/relaunch flow.

Accept only the exact six-character alphanumeric staging suffix and Mimi's
known EXE/MSI filenames for that directory's version, including older ZIP bundle
names. Skip unknown contents, directories, links and Windows reparse points.
Delete individual approved files, then the empty directory; never recursively
delete temp folders or touch Downloads, caches, credentials or user data.
Locked files remain for a later launch. Record counts only, without paths.

Focused filesystem tests cover completed/future versions, legacy names,
malformed/development versions, mixed contents, links and repeated cleanup.
These portable tests do not substitute for Windows installer/locked-file QA.
