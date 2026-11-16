#!/usr/bin/env python3
"""Click the shipped Linux Settings switch and time a local ASR reply through visible pixels."""

import contextlib
import hashlib
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
from gi.repository import GLib
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


def pointer_location():
    values = dict(line.split("=", 1) for line in command("xdotool", "getmouselocation", "--shell").splitlines())
    return values["X"], values["Y"]


def start_switch():
    desktop = pyatspi.Registry.getDesktop(0)
    roles = {pyatspi.ROLE_TOGGLE_BUTTON, pyatspi.ROLE_CHECK_BOX, pyatspi.ROLE_PUSH_BUTTON,
             getattr(pyatspi, "ROLE_SWITCH", None)}
    for app in desktop:
        todo = [app]
        while todo:
            node = todo.pop()
            try:
                if node.name == "Live Subtitles" and node.getRole() in roles:
                    state = node.getState()
                    if state.contains(pyatspi.STATE_ENABLED) and state.contains(pyatspi.STATE_SHOWING):
                        x, y, width, height = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                        if x >= 0 and y >= 0 and width > 0 and height > 0:
                            return str(x + width // 2), str(y + height // 2)
                todo.extend(child for child in node if child is not None)
            except GLib.Error as error:
                # A WebKit accessibility subtree can disappear during page load.
                # A later lookup must still find an enabled control before clicking.
                if error.domain != "atspi_error" or "application no longer exists" not in error.message:
                    raise
    return None


def write_profile(config, port, application_id):
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
            "entries": {f"{application_id}.credentials.profiles": {
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


def overlay_crop(overlay):
    info = command("xwininfo", "-id", overlay)
    x, y, width, height = (int(re.search(rf"{field}:\s*(-?\d+)", info).group(1)) for field in
                           ("Absolute upper-left X", "Absolute upper-left Y", "Width", "Height"))
    # Sample the subtitle half of the desktop, away from toolbar labels.
    return x + 10, y + height // 2, x + width - 10, y + height - 5


def screenshot(crop):
    before = time.monotonic_ns()
    picture = ImageGrab.grab(bbox=crop)
    return before, time.monotonic_ns(), picture


def found_caption(picture):
    buffer = io.BytesIO()
    picture.save(buffer, format="PNG")
    read = subprocess.run(["tesseract", "stdin", "stdout", "--psm", "6"], input=buffer.getvalue(),
                          capture_output=True, check=True, timeout=4).stdout.decode().lower()
    return bool(re.search(r"mimi\s+synthetic\s+997\s*hz\s+caption", read))


def observe_caption(overlay, sampling):
    crop = overlay_crop(overlay)
    last_negative = None

    def recognize(frame, cache=None):
        nonlocal last_negative
        before, after, picture = frame
        if cache is None:
            match = found_caption(picture)
        else:
            fingerprint = hashlib.sha256(picture.tobytes()).digest()
            if fingerprint not in cache:
                cache[fingerprint] = found_caption(picture)
            match = cache[fingerprint]
        if match:
            if last_negative is None:
                raise RuntimeError("Caption was already present in the first frame")
            return after, last_negative
        last_negative = before
        return None

    if sampling == "online":
        return wait_for("the synthetic caption in the actual overlay pixels", lambda: recognize(screenshot(crop)))

    # Collect a bounded burst before asking OCR to recognize any images. X11
    # capture is the only work on the clock; OCR can't delay the next sample.
    frames = []
    deadline = time.monotonic() + 1.5
    while time.monotonic() < deadline and len(frames) < 75:
        started = time.monotonic()
        frames.append(screenshot(crop))
        remaining = 0.020 - (time.monotonic() - started)
        if remaining > 0:
            time.sleep(remaining)
    cache = {}
    for frame in frames:
        detection = recognize(frame, cache)
        if detection is not None:
            return detection
    raise RuntimeError("No synthetic caption in the bounded offline screenshot burst")


def trace_milestones(observations, trace):
    assert observations["MIMI_CAPTION_LOSS"] == [0, 0, 0], observations["MIMI_CAPTION_LOSS"]
    before_us, after_us = observations["MIMI_CAPTION_EPOCH"]
    assert after_us - before_us < 1000, (before_us, after_us)
    epoch = (before_us + after_us) * 500
    entries = sorted(trace, key=lambda entry: entry["id"])

    def pick(label, predicate):
        for entry in entries:
            if predicate(entry["event"]):
                return entry
        counts = {}
        for entry in entries:
            kind = entry["event"]["kind"]
            counts[kind] = counts.get(kind, 0) + 1
        raise RuntimeError(f"Missing {label} trace milestone; event counts={counts}")

    admitted = pick("admitted source draft", lambda event:
                    event["kind"] == "provider"
                    and event["observation"]["eventKind"] == "sourceDraft"
                    and event["observation"]["admission"] == "accepted"
                    and event["observation"]["producer"] == "session"
                    and event["observation"]["sourceCharacters"] > 0)
    generation = admitted["event"]["observation"]["generation"]
    reduced = pick("first nonempty source state", lambda event:
                   event["kind"] == "reduced" and event["changed"] and event["generation"] == generation
                   and event["after"]["sourceCharacters"] > 0)
    snapshot = pick("first nonempty source snapshot", lambda event:
                    event["kind"] == "snapshot" and event["generation"] == generation
                    and event["summary"]["sourceCharacters"] > 0)
    snapshot_id = snapshot["event"]["snapshotId"]
    publication = pick("source snapshot publication", lambda event:
                       event["kind"] == "published" and event["snapshotId"] == snapshot_id and event["delivered"])
    milestones = {"draft_admitted": admitted, "draft_reduced": reduced, "snapshot_created": snapshot,
                  "snapshot_published": publication}
    for stage in ("wireReceived", "storeApplied", "overlayCommitted"):
        def matches_stage(event):
            if event["kind"] != "frontend":
                return False
            item = event["observation"]
            return item["window"] == "overlay" and item["stage"] == stage and item["snapshotId"] == snapshot_id \
                and (stage != "overlayCommitted" or (item["visibleCharacters"] or 0) > 0)
        milestones[f"trace_{stage}_received"] = pick(stage, matches_stage)
    stopped = pick("flushed metadata trace", lambda event: event["kind"] == "stopped")
    assert not stopped["event"]["unflushedWindows"], stopped
    print(json.dumps({"trace_snapshot_id": snapshot_id,
                      "trace_event_ids": {key: item["id"] for key, item in milestones.items()}}), flush=True)
    return {key: epoch + item["elapsedMs"] * 1_000_000 for key, item in milestones.items()}


def app_run(executable, root, results, sampling):
    observations = {}
    trace = []
    tail = []

    def collect(process):
        for line in process.stdout:
            observed = time.monotonic_ns()
            tail.append(line.rstrip())
            del tail[:-25]
            if line.startswith(("MIMI_CAPTION_EPOCH ", "MIMI_CAPTION_LOSS ")):
                marker, *values = line.split()
                observations[marker] = [int(value) for value in values]
            elif line.startswith("MIMI_CAPTION_TRACE "):
                trace.append(json.loads(line.split(" ", 1)[1]))
            for label in ("session start entered", "audio capture started",
                          "audio3 asr draft length=", "audio3 asr final length="):
                if label in line:
                    observations.setdefault(label, observed)

    with child(executable, stdout=subprocess.PIPE) as app:
        collector = threading.Thread(target=collect, args=(app,), daemon=True)
        collector.start()
        try:
            settings = wait_for("visible Settings", lambda: window("^Mimi$"))
            x, y = wait_for("enabled Live Subtitles switch", start_switch)
            if observations or window("^Mimi Subtitles$"):
                raise RuntimeError("App started a session before receiving the Settings click")
            command("xdotool", "windowactivate", "--sync", settings)
            cursor_already_there = pointer_location() == (x, y)
            if not cursor_already_there:
                command("xdotool", "mousemove", x, y)
            if pointer_location() != (x, y):
                raise RuntimeError("The native pointer did not reach the Settings switch")
            clicked = time.monotonic_ns()
            command("xdotool", "mousedown", "1", "mouseup", "1")
            requested = wait_for("native start request", lambda: observations.get("session start entered"))
            ready = wait_for("native capture readiness", lambda: observations.get("audio capture started"))
            playback_requested = time.monotonic_ns()
            with child("paplay", "--device=mimi-output", str(root / "tone.wav")) as player:
                overlay = wait_for("visible subtitle window", lambda: window("^Mimi Subtitles$"))
                visible, last_negative = observe_caption(overlay, sampling)
                if player.wait(timeout=5) != 0:
                    raise RuntimeError("PulseAudio failed to play the synthetic tone")

            # Normal Settings close requests let the app finalize the session.
            command("wmctrl", "-ic", settings)
            if app.wait(timeout=10) != 0:
                raise RuntimeError("Mimi did not close cleanly")
            collector.join(timeout=1)
            draft_log = observations.get("audio3 asr draft length=")
            if draft_log is None:
                raise RuntimeError("No accepted ASR draft was observed in the production log")
            results.update(click=clicked, same_pointer=cursor_already_there, start=requested, ready=ready,
                           playback=playback_requested, visible=visible, last_negative=last_negative, draft_log=draft_log)
            if (final_log := observations.get("audio3 asr final length=")) is not None:
                results.update(final_log=final_log)
            if sampling == "trace":
                results.update(trace_milestones(observations, trace))
        except Exception:
            print("\n".join(tail), file=sys.stderr)
            raise


def sample(executable, peer_script, root, pair, sampling):
    port_file, event_file = root / "port", root / "peer-events.jsonl"
    with event_file.open("w") as events_out, child(
        sys.executable, "-B", peer_script, "--ready-file", str(port_file), stdout=events_out,
    ) as peer:
        port = int(wait_for("loopback ASR peer", lambda: port_file.read_text() if port_file.is_file() else None))
        application_id = "app.yuxino.mimi.dev" if sampling == "trace" else "app.yuxino.mimi"
        write_profile(Path(os.environ["XDG_CONFIG_HOME"]) / application_id, port, application_id)
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
                app_run(executable, root, results, sampling)
            except Exception:
                pulse_log.seek(0)
                print(pulse_log.read()[-2400:], file=sys.stderr)
                if event_file.is_file():
                    print(event_file.read_text(), file=sys.stderr)
                raise
        if peer.poll() is not None:
            raise RuntimeError("The loopback peer exited prematurely")
        events = [json.loads(line) for line in event_file.read_text().splitlines()]
        replies = [item for item in events if item["event"] == "reply"]
        if len(replies) != 1:
            raise RuntimeError("Expected one actual audio-triggered local recognition reply")
        connection = replies[0]["connection"]
        timestamps = {item["event"]: item["monotonic_ns"] for item in events
                      if item.get("connection") == connection}
        results.update(first_pcm_at_peer=timestamps["first_pcm"], matching_pcm=timestamps["matching_pcm"],
                       reply=timestamps["reply"])
        if not results["click"] <= results["start"] <= results["ready"] <= results["playback"] \
           <= results["first_pcm_at_peer"] <= results["matching_pcm"] <= results["reply"] <= results["visible"]:
            raise RuntimeError(f"Timing observations fell out of order: {results}")
        if results["draft_log"] < results["reply"]:
            raise RuntimeError("The application logged a draft before the fixture sent it")
        if "final" in timestamps:
            results.update(final_from_peer=timestamps["final"])
        milliseconds = {key: round((value - results["click"]) / 1_000_000, 3)
                        for key, value in results.items() if key not in {"click", "same_pointer"}}
        print(json.dumps({"pair": pair, "sampling": sampling, "sink_state": state,
                          "pointer_already_on_switch": results["same_pointer"],
                          "elapsed_from_click_ms": milliseconds}), flush=True)


if __name__ == "__main__":
    executable, peer_script = sys.argv[1:]
    with child("openbox"):
        def window_manager_ready():
            try:
                return command("wmctrl", "-m")
            except subprocess.CalledProcessError:
                return None

        wait_for("Openbox readiness", window_manager_ready)
        orders = ([("trace",)] * 4 if os.environ.get("MIMI_LINUX_CAPTION_TRACE") else
                  [("online", "offline"), ("offline", "online")] * 2)
        for pair, order in enumerate(orders, start=1):
            for sampling in order:
                with tempfile.TemporaryDirectory(prefix="mimi-visible-capture-") as directory:
                    root = Path(directory)
                    for key in ("CONFIG", "DATA", "CACHE", "RUNTIME"):
                        location = root / key.lower()
                        location.mkdir(mode=0o700)
                        os.environ[f"XDG_{key}_{'DIR' if key == 'RUNTIME' else 'HOME'}"] = str(location)
                    os.environ["PULSE_SERVER"] = f"unix:{root / 'pulse.sock'}"
                    os.environ["PULSE_RUNTIME_PATH"] = str(root / "runtime")
                    sample(executable, peer_script, root, pair, sampling)
