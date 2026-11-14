"""Measure the first synthetic draft before and after overlay readiness."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

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


def sample(executable, root, number, phase):
    work = root / f"sample-{number}-{phase}"
    work.mkdir(mode=0o700)
    for key in ("CONFIG", "DATA", "CACHE"):
        (work / key.lower()).mkdir(mode=0o700)
    with tempfile.TemporaryDirectory(prefix="mimi-ui-test-linux-startup-") as preferences:
        Path(preferences, "preferences.json").write_text('{"ui_language":"en"}\n')
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
        }
        command = [str(executable)]
        if executable.suffix == ".AppImage":
            command.append("--appimage-extract-and-run")
        log = work / "app.log"
        with log.open("wb") as output:
            started = time.monotonic()
            process = subprocess.Popen(command, env=environment, stdout=output, stderr=output)
            try:
                settings = waiting_for(process, "visible Settings window", lambda: window("^mimi UI test settings$"))
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

                if phase == "ready":
                    waiting_for(process, "visible overlay with committed frontend", overlay_ready)
                if any((work / marker).exists() for marker in ("draft-issued", "draft-committed", "draft-frame")):
                    raise RuntimeError("A caption appeared before the fixture request")

                requested = time.monotonic()
                (work / "draft-request").touch()
                waiting_for(process, "injected caption", (work / "draft-issued").is_file)
                issued_ms = (time.monotonic() - requested) * 1000
                waiting_for(process, "committed caption", (work / "draft-committed").is_file)
                committed_ms = (time.monotonic() - requested) * 1000
                waiting_for(process, "caption with a subsequent paint opportunity", (work / "draft-frame").is_file)
                frame_ms = (time.monotonic() - requested) * 1000
                waiting_for(process, "visible overlay with committed frontend", overlay_ready)
                waiting_for(process, "Settings listening smoke marker", (ready / "settings").is_file)

                subprocess.run(["wmctrl", "-ic", settings], check=True, timeout=2)
                if process.wait(timeout=10) != 0:
                    raise RuntimeError(f"Mimi did not quit cleanly: {process.returncode}")
                package = "appimage" if executable.suffix == ".AppImage" else "deb"
                print(f"MimiLinuxFirstCaption package={package} sample={number} phase={phase} window_ms={window_ms:.3f} "
                      f"settings_ms={settings_ms:.3f} listening_ms={listening_ms:.3f} issued_ms={issued_ms:.3f} "
                      f"committed_ms={committed_ms:.3f} frame_ms={frame_ms:.3f} "
                      f"request_after_start_ms={(requested - activated) * 1000:.3f}", flush=True)
            except Exception:
                print(log.read_text(errors="replace"), file=sys.stderr)
                if phases.is_file():
                    print(phases.read_text(), file=sys.stderr)
                raise
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=10)


if __name__ == "__main__":
    executable, root = map(Path, sys.argv[1:])
    for number in range(1, 7):
        phases = ("early", "ready")
        for phase in phases if number % 2 else reversed(phases):
            sample(executable, root, number, phase)
