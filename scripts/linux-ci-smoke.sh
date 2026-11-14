#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux || $# -ne 1 ]]; then
  echo "usage (Linux): $0 <mimi-executable-or-AppImage>" >&2
  exit 2
fi
executable="$(realpath "$1")"
[[ -x "$executable" ]] || { echo "Executable not found: $executable" >&2; exit 1; }

# Each invocation gets its own display, session bus, and profile directories.
# UI-test mode exercises only synthetic local session state; it never reads
# credentials, contacts a provider, or opens system-audio capture.
if [[ "${MIMI_LINUX_SMOKE_SESSION:-}" != 1 ]]; then
  exec dbus-run-session -- xvfb-run -a env MIMI_LINUX_SMOKE_SESSION=1 "$0" "$executable"
fi

smoke_dir="$(mktemp -d -t mimi-ui-test-linux-smoke.XXXXXX)"
app_pid=""
wm_pid=""
cleanup() {
  if [[ -n "$app_pid" ]]; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  if [[ -n "$wm_pid" ]]; then
    kill "$wm_pid" 2>/dev/null || true
    wait "$wm_pid" 2>/dev/null || true
  fi
  rm -rf "$smoke_dir"
}
trap cleanup EXIT
export XDG_CONFIG_HOME="$smoke_dir/config"
export XDG_DATA_HOME="$smoke_dir/data"
export XDG_CACHE_HOME="$smoke_dir/cache"
export XDG_RUNTIME_DIR="$smoke_dir/runtime"
mkdir -m 700 -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
export GDK_BACKEND=x11
export XDG_SESSION_TYPE=x11
unset WAYLAND_DISPLAY
export LIBGL_ALWAYS_SOFTWARE=1
export MIMI_UI_TEST=1
export MIMI_UI_TEST_STANDARD_OVERLAY=1
export MIMI_UI_TEST_PREFERENCES_DIR="$smoke_dir"
export MIMI_AUTO_START=0
export MIMI_UI_TEST_SESSION_STATE_FILE="$smoke_dir/session-state"
export MIMI_UI_TEST_FRONTEND_READY_DIR="$smoke_dir/frontend-ready"

openbox >"$smoke_dir/openbox.log" 2>&1 &
wm_pid=$!
wm_ready=0
for _ in {1..40}; do
  if ! kill -0 "$wm_pid" 2>/dev/null; then
    cat "$smoke_dir/openbox.log" >&2
    exit 1
  fi
  if wmctrl -m >/dev/null 2>&1; then wm_ready=1; break; fi
  sleep 0.25
done
[[ "$wm_ready" == 1 ]] || { echo "Openbox did not become ready." >&2; exit 1; }
if [[ "${MIMI_LINUX_STARTUP_PROBE:-0}" == 1 ]]; then
  timeout 300s /usr/bin/python3 -B scripts/linux-packaged-startup.py "$executable" "$smoke_dir"
  exit
fi
if [[ "$executable" == *.AppImage ]]; then
  # Hosted CI may not expose /dev/fuse. This still executes the packaged
  # AppRun and bundled libraries instead of the loose build executable.
  "$executable" --appimage-extract-and-run --toggle-session >"$smoke_dir/app.log" 2>&1 &
else
  "$executable" --toggle-session >"$smoke_dir/app.log" 2>&1 &
fi
app_pid=$!

ready=0
for _ in {1..60}; do
  if ! kill -0 "$app_pid" 2>/dev/null; then
    echo "mimi exited before the Linux UI smoke test completed." >&2
    cat "$smoke_dir/app.log" >&2
    exit 1
  fi
  if [[ -f "$MIMI_UI_TEST_SESSION_STATE_FILE" ]] \
    && [[ "$(cat "$MIMI_UI_TEST_SESSION_STATE_FILE")" == listening ]] \
    && [[ -f "$MIMI_UI_TEST_FRONTEND_READY_DIR/settings" ]] \
    && [[ -f "$MIMI_UI_TEST_FRONTEND_READY_DIR/overlay" ]] \
    && xdotool search --onlyvisible --name '^mimi UI test settings$' >/dev/null \
    && xdotool search --onlyvisible --name '^mimi Subtitles$' >/dev/null; then
    ready=1
    break
  fi
  sleep 0.5
done
if [[ "$ready" != 1 ]]; then
  echo "mimi did not render settings and subtitle frontends with a listening UI-test session." >&2
  cat "$smoke_dir/app.log" >&2
  exit 1
fi
echo "Linux UI smoke passed: both frontends rendered; windows visible; synthetic session listening."

# Verify native attachment as well as rendering: no invisible click-catching
# area below the island, and no control window stranded at the desktop origin.
overlay_window="$(xdotool search --onlyvisible --name '^mimi Subtitles$' | head -1)"
control_window=""
for _ in {1..20}; do
  control_window="$(xdotool search --onlyvisible --name '^mimi$' | head -1 || true)"
  [[ -n "$control_window" ]] && break
  sleep 0.25
done
[[ -n "$control_window" ]] || { echo "Overlay control was not mapped." >&2; exit 1; }
window_geometry() {
  xwininfo -id "$1" | awk '
    /Absolute upper-left X:/ { x=$4 }
    /Absolute upper-left Y:/ { y=$4 }
    /Width:/ { w=$2 }
    /Height:/ { h=$2 }
    END { print x, y, w, h }'
}
window_is_visible() {
  xwininfo -id "$1" | grep -q 'Map State: IsViewable'
}
control_island_geometry_matches() {
  local ox="$1" oy="$2" ow="$3" oh="$4" cx="$5" cy="$6" cw="$7" ch="$8"
  # The compact island hugs its measured content (80..512px); only the expanded
  # panel has a fixed 280px width. Preserve the exact anchor and height, and
  # reject a detached, oversized or click-catching surface outside this overlay.
  [[ "$cx" == "$((ox + 18))" && "$cy" == "$((oy + 16))" && "$ch" == 30 ]] \
    && (( cw >= 80 && cw <= 512 && cx + cw <= ox + ow && cy + ch <= oy + oh ))
}
assert_control_attached() {
  local ox oy ow oh cx cy cw ch
  for _ in {1..20}; do
    read -r ox oy ow oh < <(window_geometry "$overlay_window")
    read -r cx cy cw ch < <(window_geometry "$control_window")
    if control_island_geometry_matches "$ox" "$oy" "$ow" "$oh" "$cx" "$cy" "$cw" "$ch" \
      && [[ -z "${1:-}" || ( "$ox" == "$1" && "$oy" == "$2" ) ]] \
      && window_is_visible "$overlay_window" && window_is_visible "$control_window"; then return; fi
    sleep 0.25
  done
  echo "Overlay control geometry failed: parent=$ox,$oy,$ow,$oh control=$cx,$cy,$cw,$ch" >&2
  exit 1
}
assert_control_attached
wmctrl -ir "$overlay_window" -e 0,120,140,-1,-1
assert_control_attached 120 140
echo "Linux overlay geometry passed: initial attachment, exact island bounds, native move following."

# Leave a geometry debounce pending, then stop before it fires. Neither that
# callback nor a later mapping event may resurrect a stopped control window.
wmctrl -ir "$overlay_window" -e 0,160,160,-1,-1
xdotool key --clearmodifiers ctrl+shift+space
for _ in {1..40}; do
  [[ "$(cat "$MIMI_UI_TEST_SESSION_STATE_FILE")" == idle ]] && break
  sleep 0.05
done
[[ "$(cat "$MIMI_UI_TEST_SESSION_STATE_FILE")" == idle ]] || {
  echo "Linux session shortcut did not stop the UI-test session." >&2; exit 1;
}
sleep 0.75
if xdotool search --onlyvisible --name '^mimi$' >/dev/null; then
  echo "A stopped overlay control became visible again." >&2; exit 1
fi
xdotool key --clearmodifiers ctrl+shift+space
for _ in {1..40}; do
  [[ "$(cat "$MIMI_UI_TEST_SESSION_STATE_FILE")" == listening ]] && break
  sleep 0.05
done
[[ "$(cat "$MIMI_UI_TEST_SESSION_STATE_FILE")" == listening ]] || {
  echo "Linux session shortcut did not restart the UI-test session." >&2; exit 1;
}
assert_control_attached
echo "Linux overlay lifecycle passed: move-stop stays hidden; restart reattaches."

xdotool key --clearmodifiers ctrl+shift+m
for _ in {1..40}; do
  if ! window_is_visible "$control_window"; then break; fi
  sleep 0.1
done
if window_is_visible "$control_window"; then
  echo "Linux immersive key did not hide the overlay control." >&2; exit 1
fi
sleep 0.6
xdotool key --clearmodifiers ctrl+shift+m
assert_control_attached
xdotool key --clearmodifiers ctrl+shift+b
for _ in {1..40}; do
  if grep -sEq '"subtitle_display_mode"[[:space:]]*:[[:space:]]*"bilingual"' "$smoke_dir/preferences.json"; then
    break
  fi
  sleep 0.1
done
grep -Eq '"subtitle_display_mode"[[:space:]]*:[[:space:]]*"bilingual"' "$smoke_dir/preferences.json" || {
  echo "Linux display key did not switch to bilingual subtitles." >&2; exit 1;
}
echo "Linux global shortcuts passed: session toggle, immersive toggle and subtitle display."

# Desktop commands use the same session bus as the primary. An old build
# starts another app here instead of stopping the existing session.
run_desktop_command() {
  if [[ "$executable" == *.AppImage ]]; then
    timeout 20 env MIMI_AUTO_START=0 "$executable" --appimage-extract-and-run "$@" >>"$smoke_dir/commands.log" 2>&1
  else
    timeout 20 env MIMI_AUTO_START=0 "$executable" "$@" >>"$smoke_dir/commands.log" 2>&1
  fi
}
wait_session_state() {
  for _ in {1..40}; do
    [[ "$(cat "$MIMI_UI_TEST_SESSION_STATE_FILE")" == "$1" ]] && return
    sleep 0.1
  done
  echo "Desktop shortcut did not reach session state: $1" >&2
  cat "$smoke_dir/app.log" "$smoke_dir/commands.log" >&2
  exit 1
}
sleep 0.6
run_desktop_command --toggle-session
wait_session_state idle
sleep 0.6
run_desktop_command --toggle-session
wait_session_state listening
kill -0 "$app_pid"
[[ "$(xdotool search --name '^mimi UI test settings$' | wc -l)" -eq 1 ]] || {
  echo "Desktop command created another Mimi instance." >&2; exit 1;
}
assert_control_attached
run_desktop_command --toggle-immersive
for _ in {1..40}; do
  if ! window_is_visible "$control_window"; then break; fi
  sleep 0.1
done
if window_is_visible "$control_window"; then
  echo "Desktop command did not enter immersive mode." >&2; exit 1
fi
sleep 0.6
run_desktop_command --toggle-immersive
assert_control_attached
run_desktop_command --cycle-subtitle-display
run_desktop_command
kill -0 "$app_pid"
[[ "$(xdotool search --name '^mimi UI test settings$' | wc -l)" -eq 1 ]] || {
  echo "Ordinary relaunch created another Mimi instance." >&2; exit 1;
}
echo "Linux desktop commands passed: existing-instance start/stop and immersive mode."

# A desktop without a tray must still have a reliable exit path. Closing
# Settings exits on Linux; minimizing it is the keep-running action.
settings_window="$(xdotool search --onlyvisible --name '^mimi UI test settings$' | head -1)"
# Send the window manager's normal close request to Settings directly. An
# Alt+F4 keypress can reach the overlay if startup changes focus concurrently.
# Do not use XDestroyWindow: it bypasses the app's CloseRequested handler.
xprop -id "$settings_window" WM_PROTOCOLS | grep -q WM_DELETE_WINDOW
wmctrl -ic "$settings_window"
for _ in {1..20}; do
  if ! kill -0 "$app_pid" 2>/dev/null; then
    wait "$app_pid"
    app_pid=""
    echo "Linux close-to-exit smoke passed."
    exit 0
  fi
  sleep 0.25
done
echo "Closing Linux Settings did not exit Mimi." >&2
cat "$smoke_dir/app.log" "$smoke_dir/openbox.log" >&2
wmctrl -l >&2 || true
xprop -root _NET_ACTIVE_WINDOW >&2 || true
exit 1
