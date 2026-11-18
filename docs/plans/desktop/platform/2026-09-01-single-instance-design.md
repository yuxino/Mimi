# Single-instance design

## Decision

Mimi is a resident tray application with one settings store, one global
shortcut registration, one audio-session owner, and one overlay set. Register
Tauri's single-instance plugin before every other plugin so a secondary launch
exits before it can initialize any of those resources. A repeated launch asks
the original process to show and focus the settings window; arguments and the
working directory are not interpreted or logged beyond non-content metadata.

On Windows, serialize single-instance bootstrap with a Mimi-owned named startup
mutex. The upstream 2.4.4 implementation creates its mutex before its hidden
activation window, leaving a short interval in which a simultaneous launch can
fail open; it also sends the working directory and arguments with
`WM_COPYDATA` before authenticating the listener. Mimi's local Windows plugin
instead creates the mutex and listener fail-closed, requires exactly one
matching HWND, compares the listener process executable's volume/file identity
with the launching executable, and sends only a registered payload-free
activation message with a finite timeout. Mimi holds the outer gate through the
complete application setup. A background dispatcher queues gate release back
to the original owner thread; the event-loop task runs only after setup has
returned, so activation cannot block unfinished WebView, tray, or shortcut
initialization. Wrong-thread destruction aborts the process and lets the kernel
abandon the mutex rather than failing open. The activation path restores a
minimized or tray-hidden settings window before showing and focusing it.

While holding that startup gate, a repeated launch now probes and verifies the
existing listener before creating the Tauri runtime. It sends the same bounded,
payload-free activation message and returns, releasing the gate on its owner
thread. This avoids serial runtime initialization/cleanup for cold contenders.
A primary probe releases its temporary mutex, then the first plugin retains
the real instance mutex and listener; the plugin's fail-closed recheck remains.

The Windows CI smoke deliberately delays the primary inside that outer gate,
launches four cold contenders, requires every contender to hand off and exit
successfully within a bound, and proves that the original PID is the only
process left for that exact executable path. It also pre-creates the plugin
mutex without a listener, a matching listener owned by another executable, a
synchronize-only mutex, and a different kernel object with the same name to
prove fail-closed behavior. A zero-byte temporary readiness marker binds the
intended primary before contenders start. The smoke then finds the settings
HWND by exact UI-test title and PID, verifies both minimized-window restoration
and `WM_CLOSE` hiding, and proves a warm launch restores, shows, foregrounds,
and reuses that specific window. This runs for the existing native x64 and
ARM64 jobs.

The cold-launch deadline is measured separately from each process's launch
time, including checking its actual exit time. Waiting for an earlier contender
must not grant later contenders another full timeout window.

## Boundaries

- Single-instance CI proves process ownership, handoff, native-window reuse,
  and foregrounded restoration in the runner session. Native acceptance must
  still cover the visible tray icon, taskbar overflow, virtual desktops, and
  DPI scenarios.
- The activation message carries no paths, arguments, subtitle text, audio, or
  credentials. The readiness marker is empty and deleted by the smoke test.
