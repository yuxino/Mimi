# History operation state

Fix #77 without changing IPC or persisted history. Export/save operations do not
invalidate loaded transcript pages or their pending reads; archive count changes
still trigger reads through the existing effect. Selecting an already selected
record is a no-op so it cannot discard an unchanged page.

Use the existing synchronous operation guard for history deletion as well as
export/settings operations. Disable confirmation buttons and expose aria-busy
while deletion is pending. Capture the deleted ID, mount lifetime, interaction
revision and visibility revision. A successful deletion removes that ID from the
list and returns to current only if navigation has not changed. A completion
from a previous visible panel cannot alter the reopened panel. Failures keep the
record and confirmation, release the guard, and permit retry. Selection changes
clear previous operation feedback; old successes/errors cannot attach to a newer
selection. All component fixtures are synthetic and use mock IPC.

A hidden panel ignores old completion UI updates; its history list is read again
when it becomes visible. Native save-dialog and filesystem behavior requires
separate acceptance using disposable synthetic content.
