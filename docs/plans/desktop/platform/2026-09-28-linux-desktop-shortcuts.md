# Linux desktop shortcuts

Issue: https://github.com/yuxino/mimi/issues/56

Mimi uses X11 global key grabs on Linux. Wayland sessions need desktop-owned
bindings. Keep the X11 path, and skip both plugin initialization and key grabs
when the session advertises Wayland. Expose three explicit command actions:
`--toggle-session`, `--toggle-immersive`, and `--cycle-subtitle-display`.

The Linux-only Tauri single-instance plugin forwards commands through the
session bus before a second settings store, session, or renderer is initialized.
Queue callbacks on the main thread to wait for startup state and respect GTK
thread affinity. Apply a 500 ms per-action debounce and the existing session
lifecycle gate. Invalid or combined arguments never trigger a session action.
Ordinary subsequent launches show the current Settings window.

Settings exposes installation-specific commands on Wayland, including the
original AppImage path. Other windows omit unsupported shortcut hints. Desktop
configuration remains user-controlled. This does not implement GlobalShortcuts
Portal registration or change audio permissions, providers, or retention.

Verification: argument, session-detection, and quoting unit tests; existing
cross-platform checks; installed Linux .deb/AppImage command forwarding under
Xvfb. GNOME/KDE native Wayland binding and focus require desktop acceptance;
Xvfb does not establish that evidence.
