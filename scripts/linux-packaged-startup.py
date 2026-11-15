"""Measure native Linux overlay pause and resume with a synthetic session."""

from contextlib import contextmanager
import ctypes as c
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import time


class XImage(c.Structure):
    # The XImage fields through bits_per_pixel, as declared in Xlib.h.
    _fields_ = [(name, c.c_int) for name in ("width", "height", "xoffset", "format")] + [
        ("data", c.c_void_p),
    ] + [(name, c.c_int) for name in (
        "byte_order", "bitmap_unit", "bitmap_bit_order", "bitmap_pad", "depth", "bytes_per_line", "bits_per_pixel",
    )]


@contextmanager
def x11_capture():
    x11 = c.CDLL("libX11.so.6")
    x11.XOpenDisplay.argtypes = (c.c_char_p,)
    x11.XOpenDisplay.restype = c.c_void_p
    x11.XCloseDisplay.argtypes = (c.c_void_p,)
    x11.XGetImage.argtypes = (c.c_void_p, c.c_ulong, c.c_int, c.c_int, c.c_uint, c.c_uint, c.c_ulong, c.c_int)
    x11.XGetImage.restype = c.POINTER(XImage)
    x11.XDestroyImage.argtypes = (c.POINTER(XImage),)
    display = x11.XOpenDisplay(None)
    if not display:
        raise RuntimeError("Could not open the isolated X11 display")
    try:
        def bright_pixels(window_id, bounds):
            x, y, right, bottom = bounds
            width, height = right - x, bottom - y
            image = x11.XGetImage(display, int(window_id), x, y, width, height, 0xFFFFFFFF, 2)  # ZPixmap
            if not image:
                raise RuntimeError("Could not read the caption rectangle from X11")
            try:
                frame = image.contents
                if frame.byte_order != 0 or frame.bits_per_pixel != 32 or frame.bytes_per_line != width * 4:
                    raise RuntimeError("Caption probe requires the CI Xvfb 32-bit pixel layout")
                data = c.string_at(frame.data, frame.bytes_per_line * height)
                return {i for i, (pixel,) in enumerate(struct.iter_unpack("<I", data))
                        if (pixel & 0xE0E0E0) == 0xE0E0E0}
            finally:
                x11.XDestroyImage(image)

        yield bright_pixels
    finally:
        x11.XCloseDisplay(display)


def caption_commit(path):
    try:
        values = path.read_text().split()
    except FileNotFoundError:
        return None
    return tuple(map(int, values)) if len(values) == 5 else None


def caption_pixels(process, window_id, bounds, frame_marker, requested, bright_pixels):
    x, y, right, bottom = bounds
    if not (0 <= x < right <= 4096 and 0 <= y < bottom <= 4096 and (right-x) * (bottom-y) <= 16000):
        raise RuntimeError(f"Invalid native caption rectangle: {bounds}")
    observations = []
    frame_ms = None
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("Mimi exited while reading the caption pixels")
        readback = bright_pixels(window_id, bounds)
        now = time.monotonic()
        observations.append(((now - requested) * 1000, readback))
        if frame_ms is None and frame_marker.is_file():
            frame_ms = (now - requested) * 1000
        if frame_ms is not None and (now - requested) * 1000 >= frame_ms + 250:
            break
        time.sleep(0.01)
    if frame_ms is None:
        raise RuntimeError("Caption never reached the subsequent paint callback")
    # Match against the settled white glyph pixels in this exact sample. The
    # fixture disables text animation and has a dark background in this crop.
    reference = observations[-1][1] & observations[-2][1]
    if not 20 <= len(reference) <= (right-x) * (bottom-y) // 2:
        raise RuntimeError(f"Settled caption did not contain a bounded white glyph pattern ({len(reference)} pixels)")
    pixel_ms = next(ms for ms, pixels in observations if len(reference & pixels) >= 0.9 * len(reference))
    return pixel_ms, frame_ms, len(reference), observations[0][0]


def window(pattern):
    result = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--name", pattern],
        capture_output=True, text=True, timeout=2,
    )
    return next(iter(result.stdout.splitlines()), None)


def waiting_for(process, label, find):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Mimi exited before {label}: {process.returncode}")
        found = find()
        if found is not None and found is not False:
            return found
        time.sleep(0.015)
    raise RuntimeError(f"Mimi did not reach {label} within 30 seconds")


def subtitles_point(path):
    try:
        values = path.read_text().split()
    except FileNotFoundError:
        return None
    if len(values) != 2:
        return None
    return tuple(str(int(value)) for value in values)


def state(path):
    try:
        return path.read_text().strip()
    except FileNotFoundError:
        return "idle"


def control_point(path, expected):
    try:
        values = path.read_text().split()
    except FileNotFoundError:
        return None
    if len(values) != 3 or values[0] != expected:
        return None
    return tuple(str(int(value)) for value in values[1:])


SETTINGS_TITLE = "^mimi UI test settings$"


def exercise_pause_resume(process, work, preferences, settings, overlay, session, package, number):
    activation = work / "settings-activation"
    session_controls = work / "session-control"

    def minimize_settings():
        subprocess.run(["xdotool", "windowminimize", settings], check=True, timeout=2)
        waiting_for(process, "minimized Settings window", lambda: window(SETTINGS_TITLE) is None)

    def assert_settings_hidden():
        if activation.exists() or window(SETTINGS_TITLE) is not None:
            raise RuntimeError("An overlay action opened Settings without an explicit request")

    def wait_settings_opened():
        waiting_for(process, "the original Settings window", lambda: window(SETTINGS_TITLE) == settings)
        waiting_for(process, "explicit Settings activation", lambda: state(activation) == "focus-requested")

    def move_to_control(point):
        subprocess.run(["xdotool", "mousemove", "--window", overlay, *point], check=True, timeout=2)

    def press_primary():
        subprocess.run(["xdotool", "mousedown", "1", "mouseup", "1"], check=True, timeout=2)

    def click(point):
        move_to_control(point)
        started = time.monotonic()
        press_primary()
        return started

    minimize_settings()
    assert_settings_hidden()
    for cycle in range(1, 4):
        for action, target, next_action in (("pause", "paused", "resume"), ("resume", "listening", "pause")):
            point = waiting_for(process, f"enabled {action} button", lambda: control_point(session_controls, f"{action}-control"))
            started = click(point)
            try:
                waiting_for(process, f"session state {target}", lambda: state(session) == target)
            except RuntimeError:
                print(f"Expected {action} input package={package} sample={number} cycle={cycle} "
                      f"overlay={overlay} point={point[0]},{point[1]}", file=sys.stderr)
                for command in (
                    ["xdotool", "getmouselocation", "--shell"],
                    ["xdotool", "getwindowfocus"],
                    ["xprop", "-root", "_NET_ACTIVE_WINDOW"],
                    ["wmctrl", "-lG"],
                    ["xwininfo", "-id", overlay],
                ):
                    try:
                        subprocess.run(command, stdout=sys.stderr, stderr=sys.stderr, timeout=2, check=False)
                    except subprocess.TimeoutExpired:
                        print(f"Could not collect {command[0]} diagnostic", file=sys.stderr)
                raise
            state_ms = (time.monotonic() - started) * 1000
            waiting_for(process, f"enabled {next_action} button", lambda: control_point(session_controls, f"{next_action}-control"))
            button_ms = (time.monotonic() - started) * 1000
            assert_settings_hidden()
            print(f"MimiLinuxPauseResume package={package} sample={number} cycle={cycle} action={action} "
                  f"state_ms={state_ms:.3f} button_ms={button_ms:.3f}", flush=True)

    gear = waiting_for(process, "enabled Settings button", lambda: control_point(work / "settings-control", "settings-control"))
    click(gear)
    wait_settings_opened()

    minimize_settings()
    activation.unlink()
    assert_settings_hidden()
    pause = waiting_for(process, "enabled Pause button", lambda: control_point(session_controls, "pause-control"))
    move_to_control(pause)
    cursor = subprocess.check_output(["xdotool", "getmouselocation", "--shell"], text=True, timeout=2)
    screen_coordinates = dict(line.split("=", 1) for line in cursor.splitlines())
    previous_x, previous_y = screen_coordinates["X"], screen_coordinates["Y"]
    failure = Path(preferences, "fail-next-pause")
    failure.touch()
    try:
        press_primary()
        waiting_for(process, "configuration error after Pause", lambda: state(session) == "error")
        configure = waiting_for(process, "explicit configuration button", lambda: control_point(work / "configure-control", "configure-control"))
    finally:
        failure.unlink()

    subprocess.run(["xdotool", "mousemove", previous_x, previous_y], check=True, timeout=2)
    press_primary()
    deadline = time.monotonic() + 0.5
    while time.monotonic() < deadline:
        assert_settings_hidden()
        time.sleep(0.025)
    assert_settings_hidden()
    click(configure)
    wait_settings_opened()
    print(f"MimiLinuxPauseConfigError package={package} sample={number} settings_reopened_explicitly=1", flush=True)


def sample(executable, root, number, bright_pixels):
    work = root / f"sample-{number}"
    work.mkdir(mode=0o700)
    for key in ("CONFIG", "DATA", "CACHE"):
        (work / key.lower()).mkdir(mode=0o700)
    with tempfile.TemporaryDirectory(prefix="mimi-ui-test-linux-startup-") as preferences:
        Path(preferences, "preferences.json").write_text(json.dumps({
            "ui_language": "en", "subtitle_animation": False, "pulse_animation": True,
        }))
        ready = work / "frontend-ready"
        session = work / "session-state"
        phases = work / "phases"
        environment = os.environ | {
            "XDG_CONFIG_HOME": str(work / "config"),
            "XDG_DATA_HOME": str(work / "data"),
            "XDG_CACHE_HOME": str(work / "cache"),
            "MIMI_UI_TEST_PREFERENCES_DIR": preferences,
            "MIMI_UI_TEST_FRONTEND_READY_DIR": str(ready),
            "MIMI_UI_TEST_SESSION_STATE_FILE": str(session),
            "MIMI_UI_TEST_STARTUP_PHASE_FILE": str(phases),
            "MIMI_UI_TEST_FIRST_CAPTION_DIR": str(work),
            "MIMI_UI_TEST_SETTINGS_ACTIVATION_FILE": str(work / "settings-activation"),
        }
        command = [str(executable)]
        if executable.suffix == ".AppImage":
            command.append("--appimage-extract-and-run")
        log = work / "app.log"
        with log.open("wb") as output:
            started = time.monotonic()
            process = subprocess.Popen(command, env=environment, stdout=output, stderr=output)
            try:
                settings = waiting_for(process, "visible Settings window", lambda: window(SETTINGS_TITLE))
                window_ms = (time.monotonic() - started) * 1000
                x, y = waiting_for(process, "enabled Live Subtitles switch", lambda: subtitles_point(ready / "settings-switch"))
                settings_ms = (time.monotonic() - started) * 1000
                if state(session) != "idle" or window("^mimi Subtitles$"):
                    raise RuntimeError("A session or overlay appeared before explicit start")

                activated = time.monotonic()
                subprocess.run(["xdotool", "mousemove", "--window", settings, x, y, "click", "1"], check=True, timeout=2)
                waiting_for(process, "listening state", lambda: state(session) == "listening")
                listening_ms = (time.monotonic() - activated) * 1000
                waiting_for(process, "caption observer", (work / "draft-observer").is_file)
                def overlay_ready():
                    return (ready / "overlay").is_file() and window("^mimi Subtitles$") is not None

                if any((work / marker).exists() for marker in ("draft-issued", "draft-published", "draft-committed", "draft-frame")):
                    raise RuntimeError("A caption appeared before the fixture request")

                overlay_marker_pending = not (ready / "overlay").is_file()
                requested = time.monotonic()
                (work / "draft-request").touch()
                waiting_for(process, "injected caption", (work / "draft-issued").is_file)
                issued_ms = (time.monotonic() - requested) * 1000
                waiting_for(process, "caption in the native snapshot", (work / "draft-published").is_file)
                published_ms = (time.monotonic() - requested) * 1000
                *bounds, observed_pulse = waiting_for(process, "committed caption", lambda: caption_commit(work / "draft-committed"))
                committed_ms = (time.monotonic() - requested) * 1000
                if observed_pulse != 1:
                    raise RuntimeError("Caption window did not enable the status animation")
                overlay_id = waiting_for(process, "visible caption window", lambda: window("^mimi Subtitles$"))
                pixel_ms, frame_ms, glyph_pixels, first_read_ms = caption_pixels(
                    process, overlay_id, bounds, work / "draft-frame", requested, bright_pixels,
                )
                waiting_for(process, "visible overlay with committed frontend", overlay_ready)
                waiting_for(process, "Settings listening smoke marker", (ready / "settings").is_file)
                package = "appimage" if executable.suffix == ".AppImage" else "deb"
                exercise_pause_resume(process, work, preferences, settings, overlay_id, session, package, number)

                subprocess.run(["wmctrl", "-ic", settings], check=True, timeout=2)
                if process.wait(timeout=10) != 0:
                    raise RuntimeError(f"Mimi did not quit cleanly: {process.returncode}")
                print(f"MimiLinuxFirstCaption package={package} sample={number} window_ms={window_ms:.3f} "
                      f"settings_ms={settings_ms:.3f} listening_ms={listening_ms:.3f} issued_ms={issued_ms:.3f} "
                      f"published_ms={published_ms:.3f} committed_ms={committed_ms:.3f} frame_ms={frame_ms:.3f} "
                      f"pixel_ms={pixel_ms:.3f} first_read_ms={first_read_ms:.3f} glyph_pixels={glyph_pixels} "
                      f"overlay_marker_pending={int(overlay_marker_pending)} "
                      f"request_after_start_ms={(requested - activated) * 1000:.3f}", flush=True)
            except Exception:
                print(log.read_text(errors="replace"), file=sys.stderr)
                if phases.is_file():
                    print(phases.read_text(), file=sys.stderr)
                print(f"Session state at failure: {state(session)}", file=sys.stderr)
                for name in ("draft-issued", "draft-committed", "draft-frame", "session-control", "settings-control", "settings-activation"):
                    if (work / name).is_file():
                        print(f"Overlay probe marker {name}: {(work / name).read_text().strip()}", file=sys.stderr)
                raise
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)


if __name__ == "__main__":
    executable, root = map(Path, sys.argv[1:])
    with x11_capture() as bright_pixels:
        for number in range(1, 7):
            sample(executable, root, number, bright_pixels)
