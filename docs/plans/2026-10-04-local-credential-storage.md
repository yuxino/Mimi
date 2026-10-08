# Local credential files and one-time OS-store import

The user explicitly superseded the OS-keychain-only requirement. API credentials
will live in a dedicated private file in the app configuration directory on
macOS, Windows and Linux. Use JSON because the current catalog already uses it;
a database or encryption key stored beside the ciphertext adds complexity without
solving this user's authorization problem. This is plaintext local storage, not
encryption. File permissions protect against other local users; processes running
as the same user can read the file. Keep credentials out of preferences, profile
metadata, ordinary snapshots, logs, diagnostics, exports and the repository.

Use a private credentials subdirectory and atomic writes. Unix directories are
0700 and files 0600. Windows uses the current user's app-config directory and
private ACLs. Reject symlinks, non-files, malformed/unknown schemas and oversized
files; preserve damaged data and report a sanitized error rather than falling
back to another store. Writes and in-process access are serialized. The dev app
has its own directory; UI-only fixtures do not touch either store.

The file records the existing profile slots eligible for import on first upgrade.
New installations and new slots never access native credential services. Normal startup,
settings snapshots and profile switches consult only the file. Do not add a
migration button, banner or explanatory product copy. One-time automatic startup
migration imports existing speech and independent text slots. Each
successful slot is durably checkpointed; failures retain only the pending slots
for retry. Existing single-slot Alibaba storage is imported only when the profile
slot is absent and its legacy tombstone permits it. After each successful write and read-back verification, delete its old OS item.
Checkpoint pending cleanup so an interruption retries deletion without rereading
the old secret. Only Mimi-owned profile and obsolete Alibaba slots are retired.
A saved replacement or deletion also retires that slot's import, preventing old
secrets from reappearing. Once import completes, no native store call is made on
reads, saves, deletion, switching, restarts or updates. A separate non-secret completion marker prevents a missing local credential file
from reopening OS authorization after an upgrade. The marker is authoritative
on every read, including file loss in the same process and restoration of an
older document containing pending import/cleanup entries. Native APIs remain only for
legacy reads and verified retirement; new credential writes always go to the file.

The latest scope is credential management only. Preserve the current signing
identity, audio permissions and floating-panel UI during this change.

Verification covers migration checkpoints/retry, missing values, malformed data,
atomic replacement, profile/text isolation, deletion/restart, zero native-store
calls after completion, and current-user file protection. Run credential-free
native settings smoke checks. Current setup is documented in the
[development guide](../development/local-dev-credentials.md).

Reference: CC Switch serializes provider settings into its local SQLite database
([provider DAO](https://github.com/farion1231/cc-switch/blob/0d0dd0a5487dd72b1d0b21c648d411d2cd99396a/src-tauri/src/database/dao/providers.rs)).
The relevant behavior is durable local storage without native credential-store
reads in normal use. Mimi's small existing JSON configuration model does not
need a database dependency for the same behavior. Older OS-only binaries do
not understand the new file; verified retirement therefore prevents their old
keys from being reused if the app is downgraded.
