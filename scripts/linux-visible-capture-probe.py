#!/usr/bin/env python3
"""Click the shipped Linux Settings switch and time a local ASR reply through visible pixels."""

import contextlib
import csv
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
from PIL import Image, ImageGrab

COMPOSITING_MODES = {"default", "never"}
TRACE_MODES = {"trace", "surface"} | COMPOSITING_MODES
RELEASE_FRONTEND = os.environ.get("MIMI_LINUX_RELEASE_FRONTEND") == "1"
COMPOSITED_ALPHA = os.environ.get("MIMI_LINUX_COMPOSITED_ALPHA") == "1"


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


def native_control(label, *, enabled_only=True):
    desktop = pyatspi.Registry.getDesktop(0)
    roles = {pyatspi.ROLE_TOGGLE_BUTTON, pyatspi.ROLE_CHECK_BOX, pyatspi.ROLE_PUSH_BUTTON,
             getattr(pyatspi, "ROLE_SWITCH", None)}
    for app in desktop:
        todo = [app]
        while todo:
            node = todo.pop()
            try:
                if node.name == label and node.getRole() in roles:
                    state = node.getState()
                    enabled = state.contains(pyatspi.STATE_ENABLED) and state.contains(pyatspi.STATE_SENSITIVE)
                    if (enabled or not enabled_only) and state.contains(pyatspi.STATE_SHOWING):
                        x, y, width, height = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                        if x >= 0 and y >= 0 and width > 0 and height > 0:
                            return (str(x + width // 2), str(y + height // 2)), enabled
                todo.extend(child for child in node if child is not None)
            except GLib.Error as error:
                # A WebKit accessibility subtree can disappear during page load.
                # A later lookup must still find an enabled control before clicking.
                if error.domain != "atspi_error" or "application no longer exists" not in error.message:
                    raise
    return None


def click_at(point):
    cursor_already_there = pointer_location() == point
    if not cursor_already_there:
        command("xdotool", "mousemove", *point)
    if pointer_location() != point:
        raise RuntimeError("The native pointer did not reach the expected control")
    clicked = time.monotonic_ns()
    command("xdotool", "mousedown", "1", "mouseup", "1")
    return clicked, cursor_already_there


def pulse_streams(kind):
    return command("pactl", "list", "short", kind).strip()


def peer_events(path):
    # The peer flushes each complete line. Ignore only a partial last write
    # when the playback and the event collector are both still running.
    data = path.read_text()
    return [json.loads(line) for line in data.split("\n")[:-1] if line]


def pause_and_resume(root, events_file, observations, results):
    with child("paplay", "--device=mimi-output", str(root / "sustained-tone.wav")) as player:
        wait_for("isolated playback and an active monitor stream", lambda:
                 True if pulse_streams("sink-inputs") and pulse_streams("source-outputs") else None)
        pause, _ = wait_for("the enabled Pause translating button", lambda: native_control("Pause translating"))
        pause_click, _ = click_at(pause)
        wait_for("first visible Resume translating accessibility label", lambda:
                 native_control("Resume translating", enabled_only=False))
        pause_feedback = time.monotonic_ns()
        resume, _ = wait_for("the enabled Resume translating button", lambda: native_control("Resume translating"))
        resume_enabled = time.monotonic_ns()
        paused = wait_for("native pause cleanup", lambda: observations.get("session paused"))
        if player.poll() is not None or not pulse_streams("sink-inputs") or pulse_streams("source-outputs"):
            raise RuntimeError("Pause did not release capture while the isolated source was still playing")

        first_close = wait_for("closed first recognition transport", lambda:
                               next((row for row in peer_events(events_file) if row["event"] == "close"
                                     and row["connection"] == 1), None))
        resume_click, _ = click_at(resume)
        wait_for("the enabled Pause translating button after resume", lambda: native_control("Pause translating"))
        next_pause_enabled = time.monotonic_ns()
        resumed = wait_for("native resume completion", lambda: observations.get("session resumed"))
        next_reply = wait_for("matching audio on the new recognition connection", lambda:
                              next((row for row in peer_events(events_file) if row["event"] == "reply"
                                    and row["connection"] == 2), None))
        if player.poll() is not None or not pulse_streams("source-outputs"):
            raise RuntimeError("Resume did not restore capture while the isolated source was still playing")

    # A ready capture may admit PCM before the app finishes publishing
    # resume feedback; require session ownership, not that incidental order.
    if not pause_click <= paused <= resume_click <= min(resumed, next_reply["monotonic_ns"]):
        raise RuntimeError("The native pause/resume observations did not follow their clicks")
    results.update(pause_click=pause_click, pause_feedback=pause_feedback, resume_enabled=resume_enabled,
                   pause_cleanup=paused, pause_socket_close=first_close["monotonic_ns"],
                   resume_click=resume_click, resume_native=resumed, resume_control_ready=next_pause_enabled,
                   resume_matched_reply=next_reply["monotonic_ns"])


def write_profile(config, port, application_id):
    credentials = config / "credentials"
    credentials.mkdir(parents=True, mode=0o700)
    files = {
        config / "preferences.json": {
            "ui_language": "en", "source_language": "en", "target_language": "original",
            "subtitle_display_mode": "original", "subtitle_animation": False, "pulse_animation": False,
            "show_intermediate_subtitles": True, "font_size": 31,
            "subtitle_background_opacity": 75 if COMPOSITED_ALPHA else 100,
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


def write_fixture(path, tone_seconds=0.5):
    rate = 24_000
    signal = b"".join(struct.pack("<h", round(6000 * math.sin(2 * math.pi * 997 * sample / rate)))
                      for sample in range(round(rate * tone_seconds)))
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


def recognize_picture(picture, *options):
    buffer = io.BytesIO()
    picture.save(buffer, format="PNG")
    return subprocess.run(["tesseract", "stdin", "stdout", "--psm", "6", *options], input=buffer.getvalue(),
                          capture_output=True, check=True, timeout=4).stdout.decode()


def found_caption(picture):
    return bool(re.search(r"mimi\s+synthetic\s+997\s*hz\s+caption", recognize_picture(picture).lower()))


def caption_rectangle(picture):
    lines = {}
    for row in csv.DictReader(io.StringIO(recognize_picture(picture, "tsv")), delimiter="\t"):
        if row["level"] == "5" and row["text"].strip():
            key = (row["block_num"], row["par_num"], row["line_num"])
            lines.setdefault(key, []).append(row)
    for words in lines.values():
        normalized = re.sub(r"[^a-z0-9]", "", "".join(row["text"].lower() for row in words))
        if normalized == "mimisynthetic997hzcaption":
            boxes = [(int(row["left"]), int(row["top"]), int(row["width"]), int(row["height"])) for row in words]
            left = min(x for x, _, _, _ in boxes)
            top = min(y for _, y, _, _ in boxes)
            right = max(x + width for x, _, width, _ in boxes)
            bottom = max(y + height for _, y, _, height in boxes)
            if not (0 <= left < right <= picture.width and 0 <= top < bottom <= picture.height):
                raise RuntimeError("The identified caption rectangle was outside the screenshot")
            return left, top, right, bottom
    raise RuntimeError("OCR confirmed the caption but did not localize its whole line")


def first_matching_caption_pixels(frames, confirmed_index):
    # The reference is independently identified by OCR. Compare just the
    # verified caption rectangle in earlier screenshots, ignoring other UI.
    reference = frames[confirmed_index][2]
    box = caption_rectangle(reference)
    expected = reference.crop(box).tobytes()
    for index, (_, observed_by, picture) in enumerate(frames[:confirmed_index + 1]):
        if picture.crop(box).tobytes() == expected:
            result = {"identical_caption_pixels": observed_by}
            if index:
                result["last_different_caption_pixels"] = frames[index - 1][0]
            return result
    raise AssertionError("The verified caption did not match its own reference pixels")


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
        times = wait_for("the synthetic caption in the actual overlay pixels", lambda: recognize(screenshot(crop)))
        return (*times, {})

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
    for index, frame in enumerate(frames):
        detection = recognize(frame, cache)
        if detection is not None:
            pixels = first_matching_caption_pixels(frames, index) if sampling in TRACE_MODES else {}
            return (*detection, pixels)
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


def app_run(executable, root, events_file, results, sampling):
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
            elif line.startswith("MIMI_CAPTION_COMPOSITING "):
                observations["compositing_policy"] = line.split()[1:]
            for label in ("session start entered", "audio capture started",
                          "audio3 asr draft length=", "audio3 asr final length=",
                          "session paused", "session resumed"):
                if label in line:
                    observations.setdefault(label, observed)

    with child(executable, stdout=subprocess.PIPE) as app:
        collector = threading.Thread(target=collect, args=(app,), daemon=True)
        collector.start()
        try:
            settings = wait_for("visible Settings", lambda: window("^Mimi$"))
            switch, _ = wait_for("enabled Live Subtitles switch", lambda: native_control("Live Subtitles"))
            if sampling in COMPOSITING_MODES:
                request, before, effective = wait_for("applied WebKit policy", lambda: observations.get("compositing_policy"))
                if request != sampling or effective == "unknown" or effective != (before if sampling == "default" else "never"):
                    raise RuntimeError(f"Unexpected WebKit policy: {request}, {before}, {effective}")
                results["compositing_policy"] = {"before": before, "effective": effective}
            started_early = "session start entered" in observations or "audio capture started" in observations
            if started_early or window("^Mimi Subtitles$"):
                raise RuntimeError("App started a session before receiving the Settings click")
            command("xdotool", "windowactivate", "--sync", settings)
            clicked, cursor_already_there = click_at(switch)
            requested = wait_for("native start request", lambda: observations.get("session start entered"))
            ready = wait_for("native capture readiness", lambda: observations.get("audio capture started"))
            playback_requested = time.monotonic_ns()
            with child("paplay", "--device=mimi-output", str(root / "tone.wav")) as player:
                overlay = wait_for("visible subtitle window", lambda: window("^Mimi Subtitles$"))
                visible, last_negative, pixel_observations = observe_caption(overlay, sampling)
                if player.wait(timeout=5) != 0:
                    raise RuntimeError("PulseAudio failed to play the synthetic tone")

            if RELEASE_FRONTEND:
                pause_and_resume(root, events_file, observations, results)
                from linux_compositing_interactions import verify_overlay_geometry
                results["geometry"] = verify_overlay_geometry(overlay, settings, command, wait_for)
                if COMPOSITED_ALPHA:
                    from linux_compositing_transparency import verify_blended_pixels
                    results["transparency"] = verify_blended_pixels(overlay, settings, command, wait_for)

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
            if sampling in TRACE_MODES:
                if not RELEASE_FRONTEND:
                    results.update(trace_milestones(observations, trace))
                results.update(pixel_observations)
        except Exception:
            print("\n".join(tail), file=sys.stderr)
            raise


def webkit_caption_frames(directory):
    frames = []
    for path in directory.glob("*.png"):
        requested_us, replied_us = map(int, path.stem.split("-"))
        frames.append((requested_us, replied_us, path))
    for requested_us, replied_us, path in sorted(frames):
        with Image.open(path) as image:
            # Match the desktop test's subtitle band. Composite transparent
            # widget pixels on the same neutral background before OCR.
            crop = image.crop((10, image.height // 2, image.width - 10, image.height - 5)).convert("RGBA")
            background = Image.new("RGBA", crop.size, (18, 18, 18, 255))
            background.alpha_composite(crop)
            if found_caption(background.convert("RGB")):
                return {
                    "webkit_requested": requested_us * 1000,
                    "webkit_replied": replied_us * 1000,
                }
    raise RuntimeError(f"No recognized caption in {len(frames)} WebKit snapshots")


def sample(executable, peer_script, root, pair, sampling):
    if sampling in COMPOSITING_MODES:
        os.environ["MIMI_LINUX_CAPTION_COMPOSITING"] = sampling
    else:
        os.environ.pop("MIMI_LINUX_CAPTION_COMPOSITING", None)
    surface_directory = root / "runtime" / "webkit-frames"
    if sampling == "surface":
        surface_directory.mkdir(mode=0o700)
        os.environ["MIMI_LINUX_WEBKIT_SNAPSHOTS"] = "1"
    else:
        os.environ.pop("MIMI_LINUX_WEBKIT_SNAPSHOTS", None)
    port_file, event_file = root / "port", root / "peer-events.jsonl"
    with event_file.open("w") as events_out, child(
        sys.executable, "-B", peer_script, "--ready-file", str(port_file), stdout=events_out,
    ) as peer:
        port = int(wait_for("loopback ASR peer", lambda: port_file.read_text() if port_file.is_file() else None))
        application_id = "app.yuxino.mimi.dev" if sampling in TRACE_MODES else "app.yuxino.mimi"
        write_profile(Path(os.environ["XDG_CONFIG_HOME"]) / application_id, port, application_id)
        write_fixture(root / "tone.wav")
        if RELEASE_FRONTEND:
            write_fixture(root / "sustained-tone.wav", tone_seconds=6)
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
                app_run(executable, root, event_file, results, sampling)
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
        if len(replies) != (2 if RELEASE_FRONTEND else 1):
            raise RuntimeError("Expected one actual audio-triggered recognition reply per connection")
        connection = replies[0]["connection"]
        timestamps = {item["event"]: item["monotonic_ns"] for item in events
                      if item.get("connection") == connection}
        results.update(first_pcm_at_peer=timestamps["first_pcm"], matching_pcm=timestamps["matching_pcm"],
                       reply=timestamps["reply"])
        if not results["click"] <= results["start"] <= results["ready"] <= results["playback"] \
           <= results["first_pcm_at_peer"] <= results["matching_pcm"] <= results["reply"] <= results["visible"]:
            raise RuntimeError(f"Timing observations fell out of order: {results}")
        if sampling in TRACE_MODES and not results["reply"] <= results["identical_caption_pixels"] <= results["visible"]:
            raise RuntimeError("Matching caption pixels did not follow the provider reply")
        if sampling == "surface":
            results.update(webkit_caption_frames(surface_directory))
            if not results["reply"] <= results["webkit_requested"] <= results["webkit_replied"]:
                raise RuntimeError(f"WebKit snapshot did not follow the provider reply: {results}")
        if results["draft_log"] < results["reply"]:
            raise RuntimeError("The application logged a draft before the fixture sent it")
        if "final" in timestamps:
            results.update(final_from_peer=timestamps["final"])
        milliseconds = {key: round((value - results["click"]) / 1_000_000, 3)
                        for key, value in results.items() if key not in {"click", "same_pointer", "compositing_policy", "geometry", "transparency"}}
        print(json.dumps({"pair": pair, "sampling": sampling, "sink_state": state,
                          "pointer_already_on_switch": results["same_pointer"],
                          "compositing_policy": results.get("compositing_policy"),
                          "geometry": results.get("geometry"),
                          "transparency": results.get("transparency"),
                          "elapsed_from_click_ms": milliseconds}), flush=True)
        return results.get("compositing_policy")


if __name__ == "__main__":
    executable, peer_script = sys.argv[1:]
    if COMPOSITED_ALPHA and not RELEASE_FRONTEND:
        raise RuntimeError("Composited alpha requires the release frontend probe")
    with child("openbox"), contextlib.ExitStack() as compositor:
        def window_manager_ready():
            try:
                return command("wmctrl", "-m")
            except subprocess.CalledProcessError:
                return None

        wait_for("Openbox readiness", window_manager_ready)
        if COMPOSITED_ALPHA:
            from linux_compositing_transparency import wait_for_compositor
            compositor.enter_context(child("xcompmgr", "-n"))
            wait_for_compositor(wait_for)
        orders = ([("default", "never"), ("never", "default")] * 2 if RELEASE_FRONTEND or os.environ.get("MIMI_LINUX_CAPTION_TRACE") else
                  [("online", "offline"), ("offline", "online")] * 2)
        for pair, order in enumerate(orders, start=1):
            effective = {}
            for sampling in order:
                with tempfile.TemporaryDirectory(prefix="mimi-visible-capture-") as directory:
                    root = Path(directory)
                    for key in ("CONFIG", "DATA", "CACHE", "RUNTIME"):
                        location = root / key.lower()
                        location.mkdir(mode=0o700)
                        os.environ[f"XDG_{key}_{'DIR' if key == 'RUNTIME' else 'HOME'}"] = str(location)
                    os.environ["PULSE_SERVER"] = f"unix:{root / 'pulse.sock'}"
                    os.environ["PULSE_RUNTIME_PATH"] = str(root / "runtime")
                    policy = sample(executable, peer_script, root, pair, sampling)
                    if policy is not None:
                        effective[sampling] = policy["effective"]
            if effective.keys() == COMPOSITING_MODES and len(set(effective.values())) == 1:
                print(json.dumps({"compositing_contrast": False, "effective_by_mode": effective}), flush=True)
                break
