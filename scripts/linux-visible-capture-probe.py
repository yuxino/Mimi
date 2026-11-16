#!/usr/bin/env python3
"""Click the shipped Linux Settings switch and time a local ASR reply through visible pixels."""

import contextlib
import io
import json
import math
import os
import re
import struct
import subprocess
import sys
import tempfile
import threading
import time
import wave
from pathlib import Path

import pyatspi
from PIL import ImageGrab


def command(*args):
    return subprocess.run(args, capture_output=True, text=True, check=True, timeout=4).stdout


def wait_for(label, observe, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = observe()
        if value is not None:
            return value
        time.sleep(0.02)
    raise RuntimeError(f"Timed out waiting for {label}")


@contextlib.contextmanager
def child(*args, stdout=subprocess.DEVNULL):
    with subprocess.Popen(args, stdout=stdout, stderr=subprocess.STDOUT, text=True) as process:
        try:
            yield process
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=4)


def window(pattern):
    output = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--name", pattern], capture_output=True, text=True, timeout=2,
    ).stdout
    return next(iter(output.splitlines()), None)


def start_switch():
    desktop = pyatspi.Registry.getDesktop(0)
    roles = {pyatspi.ROLE_TOGGLE_BUTTON, pyatspi.ROLE_CHECK_BOX, pyatspi.ROLE_PUSH_BUTTON,
             getattr(pyatspi, "ROLE_SWITCH", None)}
    for app in desktop:
        todo = [app]
        while todo:
            node = todo.pop()
            if node.name == "Live Subtitles" and node.getRole() in roles:
                state = node.getState()
                if state.contains(pyatspi.STATE_ENABLED) and state.contains(pyatspi.STATE_SHOWING):
                    x, y, width, height = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                    if x >= 0 and y >= 0 and width > 0 and height > 0:
                        return str(x + width // 2), str(y + height // 2)
            todo.extend(child for child in node if child is not None)
    return None


def write_profile(config, port):
    credentials = config / "credentials"
    credentials.mkdir(parents=True, mode=0o700)
    files = {
        config / "preferences.json": {
            "ui_language": "en", "source_language": "en", "target_language": "original",
            "subtitle_display_mode": "original", "subtitle_animation": False, "pulse_animation": False,
            "show_intermediate_subtitles": True, "subtitle_background_opacity": 100, "font_size": 31,
        },
        config / "service-profiles.json": {
            "schemaVersion": 1, "activeProfileId": "local-timing",
            "profiles": [{"id": "local-timing", "name": "Local timing", "provider": "customOpenAIASR"}],
        },
        credentials / "credentials.json": {
            "schemaVersion": 1, "pendingImport": [], "pendingCleanup": [],
            "entries": {"app.yuxino.mimi.credentials.profiles": {
                "provider-profile:local-timing:customOpenAIASR:api-key": json.dumps({
                    "kind": "customSpeech", "endpoint": f"ws://127.0.0.1:{port}/asr",
                    "model": "local-timing", "apiKey": "synthetic-fixture",
                }),
            }},
        },
    }
    for path, value in files.items():
        path.write_text(json.dumps(value), encoding="utf-8")
        path.chmod(0o600)
    (config / "credentials-file-mode-complete").touch(mode=0o600)


def write_fixture(path):
    rate = 24_000
    signal = b"".join(struct.pack("<h", round(6000 * math.sin(2 * math.pi * 997 * sample / rate)))
                      for sample in range(rate // 2))
    with wave.open(str(path), "wb") as stream:
        stream.setparams((1, 2, rate, 0, "NONE", "not compressed"))
        stream.writeframes(signal + b"\0" * rate * 2)


def visible_caption(overlay):
    info = command("xwininfo", "-id", overlay)
    x, y, width, height = (int(re.search(rf"{field}:\s*(-?\d+)", info).group(1)) for field in
                           ("Absolute upper-left X", "Absolute upper-left Y", "Width", "Height"))
    # Sample the subtitle half of the composited desktop, away from toolbar labels.
    taken = time.monotonic_ns()
    picture = ImageGrab.grab(bbox=(x + 10, y + height // 2, x + width - 10, y + height - 5))
    buffer = io.BytesIO()
    picture.save(buffer, format="PNG")
    read = subprocess.run(["tesseract", "stdin", "stdout", "--psm", "6"], input=buffer.getvalue(),
                          capture_output=True, check=True, timeout=4).stdout.decode().lower()
    if re.search(r"mimi\s+synthetic\s+997\s*hz\s+caption", read):
        return taken
    return None


def app_run(executable, root, results):
    observations = {}
    tail = []

    def collect(process):
        for line in process.stdout:
            observed = time.monotonic_ns()
            tail.append(line.rstrip())
            del tail[:-25]
            for label in ("session start entered", "audio capture started", "session stopped"):
                if label in line:
                    observations.setdefault(label, observed)

    with child(executable, stdout=subprocess.PIPE) as app:
        threading.Thread(target=collect, args=(app,), daemon=True).start()
        try:
            settings = wait_for("visible Settings", lambda: window("^Mimi$"))
            x, y = wait_for("enabled Live Subtitles switch", start_switch)
            if observations or window("^Mimi Subtitles$"):
                raise RuntimeError("App started a session before receiving the Settings click")
            command("xdotool", "windowactivate", "--sync", settings)
            command("xdotool", "mousemove", "--sync", x, y)
            clicked = time.monotonic_ns()
            command("xdotool", "mousedown", "1", "mouseup", "1")
            requested = wait_for("native start request", lambda: observations.get("session start entered"))
            ready = wait_for("native capture readiness", lambda: observations.get("audio capture started"))
            playback_requested = time.monotonic_ns()
            with child("paplay", "--device=mimi-output", str(root / "tone.wav")) as player:
                overlay = wait_for("visible subtitle window", lambda: window("^Mimi Subtitles$"))
                visible = wait_for("the synthetic caption in the actual overlay pixels", lambda: visible_caption(overlay))
                if player.wait(timeout=5) != 0:
                    raise RuntimeError("PulseAudio failed to play the synthetic tone")

            # Normal Settings close requests let the app finalize the session.
            command("wmctrl", "-ic", settings)
            if app.wait(timeout=10) != 0:
                raise RuntimeError("Mimi did not close cleanly")
            results.update(click=clicked, start=requested, ready=ready, playback=playback_requested, visible=visible)
        except Exception:
            print("\n".join(tail), file=sys.stderr)
            raise


def sample(executable, peer_script, root, number):
    port_file, event_file = root / "port", root / "peer-events.jsonl"
    with event_file.open("w") as events_out, child(
        sys.executable, "-B", peer_script, "--ready-file", str(port_file), stdout=events_out,
    ) as peer:
        port = int(wait_for("loopback ASR peer", lambda: port_file.read_text() if port_file.is_file() else None))
        write_profile(Path(os.environ["XDG_CONFIG_HOME"]) / "app.yuxino.mimi", port)
        write_fixture(root / "tone.wav")
        with (root / "pulse.log").open("w+") as pulse_log, child(
            "pulseaudio", "--daemonize=no", "--exit-idle-time=-1", "--use-pid-file=no", "-n",
            f"--load=module-native-protocol-unix socket={root / 'pulse.sock'} auth-anonymous=1",
            "--load=module-null-sink sink_name=mimi-output norewinds=1", stdout=pulse_log,
        ):
            results = {}
            try:
                def sink_state():
                    try:
                        result = command("pactl", "list", "sinks")
                    except subprocess.CalledProcessError:
                        return None
                    return re.search(r"State:\s+(\w+)", result).group(1) if "Name: mimi-output" in result else None

                state = wait_for("the isolated PulseAudio null sink", sink_state)
                if state != "IDLE":
                    raise RuntimeError(f"Expected an idle virtual sink, got {state}")
                command("pactl", "set-default-sink", "mimi-output")
                app_run(executable, root, results)
            except Exception:
                pulse_log.seek(0)
                print(pulse_log.read()[-2400:], file=sys.stderr)
                if event_file.is_file():
                    print(event_file.read_text(), file=sys.stderr)
                raise
        if peer.poll() is not None:
            raise RuntimeError("The loopback peer exited prematurely")
        events = [json.loads(line) for line in event_file.read_text().splitlines()]
        timestamps = {item["event"]: item["monotonic_ns"] for item in events}
        if sum(item["event"] == "reply" for item in events) != 1:
            raise RuntimeError("Expected one actual audio-triggered local recognition reply")
        results.update({key: timestamps[key] for key in ("first_pcm", "matching_pcm", "reply")})
        if not results["click"] <= results["start"] <= results["ready"] <= results["playback"] \
           <= results["first_pcm"] <= results["matching_pcm"] <= results["reply"] <= results["visible"]:
            raise RuntimeError(f"Timing observations fell out of order: {results}")
        milliseconds = {key: round((value - results["click"]) / 1_000_000, 3)
                        for key, value in results.items() if key != "click"}
        print(json.dumps({"sample": number, "sink_state": state, "elapsed_from_click_ms": milliseconds}), flush=True)


if __name__ == "__main__":
    executable, peer_script = sys.argv[1:]
    with child("openbox"):
        def window_manager_ready():
            try:
                return command("wmctrl", "-m")
            except subprocess.CalledProcessError:
                return None

        wait_for("Openbox readiness", window_manager_ready)
        for number in range(1, 5):
            with tempfile.TemporaryDirectory(prefix="mimi-visible-capture-") as directory:
                root = Path(directory)
                for key in ("CONFIG", "DATA", "CACHE", "RUNTIME"):
                    location = root / key.lower()
                    location.mkdir(mode=0o700)
                    os.environ[f"XDG_{key}_{'DIR' if key == 'RUNTIME' else 'HOME'}"] = str(location)
                os.environ["PULSE_SERVER"] = f"unix:{root / 'pulse.sock'}"
                os.environ["PULSE_RUNTIME_PATH"] = str(root / "runtime")
                sample(executable, peer_script, root, number)
