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
from gi.repository import Atspi, GLib
from PIL import ImageGrab

from linux_pause_button_pixels import record_button_pixels

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
                            return (str(x + width // 2), str(y + height // 2)), enabled, node
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


def measure_control(node, label):
    first_label = None
    first_disabled = None
    durations = []
    saw_disabled = False

    def observe():
        nonlocal first_label, saw_disabled, first_disabled
        started = time.monotonic_ns()
        # Use the held object in both arms so the screenshot sampler is
        # the only difference during the click-to-button measurement.
        Atspi.Accessible.clear_cache(node)
        name = node.name
        state = node.getState()
        showing = state.contains(pyatspi.STATE_SHOWING)
        actionable = state.contains(pyatspi.STATE_ENABLED) and state.contains(pyatspi.STATE_SENSITIVE)
        ready = actionable if showing and name == label else None
        after = time.monotonic_ns()
        if showing and not actionable:
            first_disabled = first_disabled or after
        durations.append(after - started)
        if ready is not None:
            if first_label is None:
                first_label = after
            saw_disabled |= not ready
        return after if ready else None

    enabled = wait_for(f"enabled {label} on the held control", observe, seconds=3)
    return first_label, enabled, {
        "probe_count": len(durations), "max_query_ms": round(max(durations) / 1_000_000, 3),
        "observed_disabled": saw_disabled, "first_disabled_ns": first_disabled,
    }


def wait_until(target):
    remaining = target - time.monotonic_ns()
    if remaining > 0:
        time.sleep(remaining / 1_000_000_000)


def measure_action(point, node, next_label, sampling, *, scheduled_click=None):
    recorder = record_button_pixels(point) if sampling == "pixels" else contextlib.nullcontext()
    with recorder as pixels:
        if pointer_location() != point:
            command("xdotool", "mousemove", *point)
        if pointer_location() != point:
            raise RuntimeError("The native pointer did not reach the measured button")
        hover_ready = time.monotonic_ns() + 320_000_000
        if scheduled_click is not None and hover_ready > scheduled_click:
            raise RuntimeError("The prior pause did not leave time for the fixed resume click")
        wait_until(scheduled_click if scheduled_click is not None else hover_ready)

        clicked, pointer_already_there = click_at(point)
        if not pointer_already_there:
            raise RuntimeError("The pointer moved during the measured action")
        injected = time.monotonic_ns()
        feedback, enabled, observer = measure_control(node, next_label)
        wait_until(max(clicked + 450_000_000, enabled + 220_000_000))

    visual = pixels.summarize(clicked, enabled) if pixels is not None else None
    return clicked, injected, feedback, enabled, observer, visual


def require_overlay_center(point, overlay):
    geom = command("xwininfo", "-id", overlay)
    x, y, width, height = (int(re.search(rf"{field}:\s*(-?\d+)", geom).group(1)) for field in
                          ("Absolute upper-left X", "Absolute upper-left Y", "Width", "Height"))
    cx, cy = map(int, point)
    if not (x <= cx < x + width and y <= cy < y + height):
        raise RuntimeError("The measured control is outside the subtitle overlay")


def pause_and_resume(root, events_file, observations, results, overlay, sampling):
    with child("paplay", "--device=mimi-output", str(root / "sustained-tone.wav")) as player:
        wait_for("isolated playback and an active monitor stream", lambda:
                 True if pulse_streams("sink-inputs") and pulse_streams("source-outputs") else None)
        pause, _, node = wait_for("the enabled Pause translating button", lambda: native_control("Pause translating"))
        require_overlay_center(pause, overlay)
        pause_click, pause_injected, pause_feedback, resume_enabled, pause_observer, pause_visual = measure_action(
            pause, node, "Resume translating", sampling,
        )
        pause_requested = wait_for("native pause request", lambda: observations.get("session pause requested"))
        paused = wait_for("native pause cleanup", lambda: observations.get("session paused"))
        if player.poll() is not None or not pulse_streams("sink-inputs") or pulse_streams("source-outputs"):
            raise RuntimeError("Pause did not release capture while the isolated source was still playing")

        first_close = wait_for("closed first recognition transport", lambda:
                               next((row for row in peer_events(events_file) if row["event"] == "close"
                                     and row["connection"] == 1), None))
        resume, _, resume_node = wait_for("the enabled Resume translating button", lambda: native_control("Resume translating"))
        require_overlay_center(resume, overlay)
        if resume != pause:
            raise RuntimeError("The overlay action moved when the session paused")
        capture_samples = []
        capture_monitor_done = threading.Event()
        capture_monitor_errors = []

        def monitor_capture():
            try:
                while not capture_monitor_done.is_set():
                    started = time.monotonic_ns()
                    active = bool(pulse_streams("source-outputs"))
                    capture_samples.append((started, time.monotonic_ns(), active))
                    capture_monitor_done.wait(0.02)
            except Exception as error:
                capture_monitor_errors.append(type(error).__name__)

        capture_monitor = threading.Thread(target=monitor_capture, daemon=True)
        capture_monitor.start()
        try:
            resume_click, resume_injected, resume_feedback, next_pause_enabled, resume_observer, resume_visual = measure_action(
                resume, resume_node, "Pause translating", sampling, scheduled_click=pause_click + 1_050_000_000,
            )
        finally:
            capture_monitor_done.set()
            capture_monitor.join(timeout=5)
        if capture_monitor.is_alive() or capture_monitor_errors:
            raise RuntimeError("The independent resume capture observer failed")
        resume_requested = wait_for("native resume request", lambda: observations.get("session resume requested"))
        resumed = wait_for("native resume completion", lambda: observations.get("session resumed"))
        next_reply = wait_for("matching audio on the new recognition connection", lambda:
                              next((row for row in peer_events(events_file) if row["event"] == "reply"
                                    and row["connection"] == 2), None))
        if player.poll() is not None or not pulse_streams("source-outputs"):
            raise RuntimeError("Resume did not restore capture while the isolated source was still playing")

    if not pause_click <= pause_requested <= paused <= resume_click <= resume_requested <= min(resumed, next_reply["monotonic_ns"]):
        raise RuntimeError("The native pause/resume observations did not follow their clicks")
    results.update(pause_click=pause_click, pause_injected=pause_injected, pause_requested=pause_requested,
                   pause_feedback=pause_feedback, resume_enabled=resume_enabled, pause_cleanup=paused,
                   pause_socket_close=first_close["monotonic_ns"], resume_click=resume_click,
                   resume_injected=resume_injected, resume_requested=resume_requested, resume_feedback=resume_feedback,
                   resume_native=resumed, resume_control_ready=next_pause_enabled,
                   resume_matched_reply=next_reply["monotonic_ns"],
                   observer={"pause": pause_observer, "resume": resume_observer},
                   visual={"pause": pause_visual, "resume": resume_visual},
                   capture_samples=capture_samples)


def write_profile(config, port, application_id):
    credentials = config / "credentials"
    credentials.mkdir(parents=True, mode=0o700)
    files = {
        config / "preferences.json": {
            "ui_language": "en", "source_language": "en", "target_language": "original",
            "subtitle_display_mode": "original", "subtitle_animation": False, "pulse_animation": False,
            "show_intermediate_subtitles": True, "font_size": 31,
            "subtitle_background_opacity": 75,
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


def observe_caption(overlay):
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
            pixels = first_matching_caption_pixels(frames, index)
            return (*detection, pixels)
    raise RuntimeError("No synthetic caption in the bounded offline screenshot burst")


def app_run(executable, root, events_file, results, sampling):
    observations = {}
    tail = []

    def collect(process):
        for line in process.stdout:
            observed = time.monotonic_ns()
            tail.append(line.rstrip())
            del tail[:-25]
            if "overlay software GL acceleration disabled=" in line:
                observations["compositing_policy"] = "disabled=true" in line
            for label in ("session start entered", "audio capture started",
                          "audio3 asr draft length=", "audio3 asr final length=",
                          "session pause requested", "session paused", "session resume requested", "session resumed"):
                if label in line:
                    observations.setdefault(label, observed)

    with child(executable, stdout=subprocess.PIPE) as app:
        collector = threading.Thread(target=collect, args=(app,), daemon=True)
        collector.start()
        try:
            settings = wait_for("visible Settings", lambda: window("^Mimi$"))
            switch, _, _ = wait_for("enabled Live Subtitles switch", lambda: native_control("Live Subtitles"))
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
                visible, last_negative, pixel_observations = observe_caption(overlay)
                if player.wait(timeout=5) != 0:
                    raise RuntimeError("PulseAudio failed to play the synthetic tone")

            pause_and_resume(root, events_file, observations, results, overlay, sampling)

            # Normal Settings close requests let the app finalize the session.
            command("wmctrl", "-ic", settings)
            if app.wait(timeout=10) != 0:
                raise RuntimeError("Mimi did not close cleanly")
            collector.join(timeout=1)
            if observations.get("compositing_policy") is not True:
                raise RuntimeError("The expected software GL policy did not apply")
            draft_log = observations.get("audio3 asr draft length=")
            if draft_log is None:
                raise RuntimeError("No accepted ASR draft was observed in the production log")
            results.update(click=clicked, same_pointer=cursor_already_there, start=requested, ready=ready,
                           playback=playback_requested, visible=visible, last_negative=last_negative, draft_log=draft_log)
            if (final_log := observations.get("audio3 asr final length=")) is not None:
                results.update(final_log=final_log)
            results.update(pixel_observations)
        except Exception:
            print("\n".join(tail), file=sys.stderr)
            raise


def sample(executable, peer_script, root, pair, resume_delay_ms):
    sampling = "pixels"
    port_file, event_file = root / "port", root / "peer-events.jsonl"
    with event_file.open("w") as events_out, child(
        sys.executable, "-B", peer_script, "--ready-file", str(port_file),
        "--resume-delay-ms", str(resume_delay_ms), stdout=events_out,
    ) as peer:
        port = int(wait_for("loopback ASR peer", lambda: port_file.read_text() if port_file.is_file() else None))
        application_id = "app.yuxino.mimi"
        write_profile(Path(os.environ["XDG_CONFIG_HOME"]) / application_id, port, application_id)
        write_fixture(root / "tone.wav")
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
        if len(replies) != 2:
            raise RuntimeError("Expected one actual audio-triggered recognition reply per connection")
        connection = replies[0]["connection"]
        timestamps = {item["event"]: item["monotonic_ns"] for item in events
                      if item.get("connection") == connection}
        results.update(first_pcm_at_peer=timestamps["first_pcm"], matching_pcm=timestamps["matching_pcm"],
                       reply=timestamps["reply"])
        if not results["click"] <= results["start"] <= results["ready"] <= results["playback"] \
           <= results["first_pcm_at_peer"] <= results["matching_pcm"] <= results["reply"] <= results["visible"]:
            raise RuntimeError(f"Timing observations fell out of order: {results}")
        if not results["reply"] <= results["identical_caption_pixels"] <= results["visible"]:
            raise RuntimeError("Matching caption pixels did not follow the provider reply")
        if results["draft_log"] < results["reply"]:
            raise RuntimeError("The application logged a draft before the fixture sent it")
        if "final" in timestamps:
            results.update(final_from_peer=timestamps["final"])
        resume_events = {item["event"]: item["monotonic_ns"] for item in events
                         if item.get("connection") == 2}
        results.update(resume_setup_received=resume_events["session_update_received"],
                       resume_setup_ready=resume_events["session_ready"])
        if not results["resume_click"] <= results["resume_setup_received"] <= results["resume_setup_ready"] <= results["resume_matched_reply"]:
            raise RuntimeError("Resume transport setup and audio did not follow the native click")
        samples = results.pop("capture_samples")
        waiting_samples = [active for started, ended, active in samples
                           if results["resume_setup_received"] <= started <= ended <= results["resume_setup_ready"]]
        if any(waiting_samples) or (resume_delay_ms and len(waiting_samples) < 3):
            raise RuntimeError("Resume did not remain without capture during the observed setup wait")
        actual_delay_ms = (results["resume_setup_ready"] - results["resume_setup_received"]) / 1_000_000
        if actual_delay_ms < resume_delay_ms:
            raise RuntimeError("The peer did not apply the requested readiness delay")
        milliseconds = {key: round((value - results["click"]) / 1_000_000, 3)
                        for key, value in results.items() if key not in {"click", "same_pointer", "observer", "visual"}}
        print(json.dumps({"pair": pair, "sampling": sampling, "resume_delay_ms": resume_delay_ms, "capture_queries_during_setup": len(waiting_samples), "sink_state": state,
                          "pointer_already_on_switch": results["same_pointer"],
                          "observer": results["observer"],
                          "visual": results["visual"],
                          "elapsed_from_click_ms": milliseconds}), flush=True)


if __name__ == "__main__":
    executable, peer_script = sys.argv[1:]
    with child("openbox"), contextlib.ExitStack() as compositor:
        def window_manager_ready():
            try:
                return command("wmctrl", "-m")
            except subprocess.CalledProcessError:
                return None

        wait_for("Openbox readiness", window_manager_ready)
        from linux_compositing_transparency import wait_for_compositor
        compositor.enter_context(child("xcompmgr", "-n"))
        wait_for_compositor(wait_for)
        orders = [(0, 300), (300, 0)] * 2
        for pair, order in enumerate(orders, start=1):
            for resume_delay_ms in order:
                with tempfile.TemporaryDirectory(prefix="mimi-visible-capture-") as directory:
                    root = Path(directory)
                    for key in ("CONFIG", "DATA", "CACHE", "RUNTIME"):
                        location = root / key.lower()
                        location.mkdir(mode=0o700)
                        os.environ[f"XDG_{key}_{'DIR' if key == 'RUNTIME' else 'HOME'}"] = str(location)
                    os.environ["PULSE_SERVER"] = f"unix:{root / 'pulse.sock'}"
                    os.environ["PULSE_RUNTIME_PATH"] = str(root / "runtime")
                    sample(executable, peer_script, root, pair, resume_delay_ms)
