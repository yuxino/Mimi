# Linux first credential save and explicit retry

Issue #101 describes an unverified first-save candidate separately from the
already merged Secret Service recovery in #95. Current code confirms two
remaining gaps:

- `zbus-secret-service-keyring-store` 1.0.0 writes new items into the default
  collection, but cannot create it when no `default` alias exists. The backend
  maps the missing alias to `NoStorageAccess(NoResult)`. Existing isolated
  tests unlock a synthetic login keyring before saving, hiding this case.
- Structured speech/translation saves read previous values before writing.
  A failed read remains cached after the user restores storage, so another
  Save fails until Check connection clears that failed slot.

On Linux, only an explicit save that fails with the backend's exact
`NoStorageAccess(NoResult)` condition checks for a missing default collection.
If the default alias is absent, Mimi requests `CreateCollection` with the
`default` alias through an encrypted Secret Service session. The provider
owns its native password/authorization dialog and encryption policy. A
dismissed prompt stays an access error. Existing collections, locked-store
errors and unrelated failures never trigger collection creation or bypass
authorization. The original item attributes and in-place replacement remain
unchanged. The retry occurs once, then existing uncached save verification
checks the persisted value.

This uses `secret-service` 5.1 directly with `crypto-rust`, the same crate and
feature already present through the keyring backend; it adds no new runtime
service or credential requirement. A fork of the complete keyring backend
would be larger, while telling users to manually pre-create a collection
would leave normal first save broken.

Each structured Save also clears only failed reads belonging to the slots
that action needs. Speech-only and custom text-only saves remain independent;
successful cached reads and other profiles remain untouched. Ordinary reads
still cache failures to avoid repeated automatic prompts.

Verification combines focused fake-store regressions with
`scripts/linux-keyring-first-save-smoke.sh`. The native smoke uses a new
private D-Bus/display/data directory, starts GNOME Keyring without creating a
login collection, proves reads do not create a default collection, dismisses
the first save prompt, accepts the second with a synthetic nonempty password,
and reads/deletes the saved synthetic value from a fresh test process. This
exercises the production storage adapter and OS dialog, not UI-test storage.
It does not prove a full packaged-app restart, PAM login integration, or
compatibility with every Secret Service implementation.
