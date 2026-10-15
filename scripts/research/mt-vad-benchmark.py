#!/usr/bin/env python3
"""Offline comparison of two Silero hosts; never captures audio or calls MT.

Use the existing development-batch matrix, with explicit local WAV inputs.
This is a research probe, not a second production recognizer or batch runner.
The common PCM clock/window and matching hysteresis isolate host differences.
"""
import argparse
import hashlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import platform
import re
import time
import wave

RATE = 16_000
FRAME = 512
CONTEXT = 64
MODEL_REVISION = "1e261b036686cd0017d500ee96acd1c4ba572a9d"
MODEL_SHA256 = "7ed98ddbad84ccac4cd0aeb3099049280713df825c610a8ed34543318f1b2c49"
MAX_SECONDS = 120
MAX_SCENES = 32


class ActivityGate:
    """Match sherpa-onnx's Silero IsSpeech hysteresis, not semantic finality.

    Algorithm reference: sherpa-onnx/csrc/silero-vad-model.cc (Apache-2.0),
    k2-fsa/sherpa-onnx v1.13.8. Start/end samples describe *detection time*,
    including confirmation delay, not ground-truth speech boundaries.
    """
    def __init__(self, silence_ms, speech_ms=250):
        self.min_silence = round(silence_ms * RATE / 1000)
        self.min_speech = round(speech_ms * RATE / 1000)
        self.current = self.start = self.end = 0
        self.triggered = False

    def push(self, probability):
        self.current += FRAME
        if probability > 0.5 and self.end:
            self.end = 0
        if probability > 0.5 and not self.start:
            self.start = self.current
            return False
        if probability > 0.5 and self.start and not self.triggered:
            if self.current - self.start < self.min_speech:
                return False
            self.triggered = True
            return True
        if probability < 0.5 and not self.triggered:
            self.start = self.end = 0
            return False
        if probability > 0.35 and self.triggered:
            return True
        if probability > 0.5 and not self.triggered:
            self.triggered = True
            return True
        if probability < 0.5 and self.triggered:
            if not self.end:
                self.end = self.current
            if self.current - self.end < self.min_silence:
                return True
            self.start = self.end = 0
            self.triggered = False
        return False


def validate_wave(path):
    with wave.open(str(path), "rb") as source:
        if (source.getframerate(), source.getnchannels(), source.getsampwidth(),
                source.getcomptype()) != (RATE, 1, 2, "NONE"):
            raise ValueError("input must be uncompressed mono PCM16 at 16 kHz")
        frames = source.getnframes()
        if not 0 < frames <= RATE * MAX_SECONDS:
            raise ValueError("input duration must be in (0, 120] seconds")
        # Hash PCM incrementally: no recording or full input is kept in memory.
        digest = hashlib.sha256()
        actual = 0
        while block := source.readframes(FRAME):
            actual += len(block) // 2
            digest.update(block)
        if actual != frames:
            raise ValueError("truncated WAV payload")
        return frames, digest.hexdigest()


def load_matrix(path):
    matrix = json.loads(path.read_text(encoding="utf-8"))
    scenes = matrix.get("scenes", [])
    if not 0 < len(scenes) <= MAX_SCENES:
        raise ValueError("matrix must contain 1 to 32 scenes")
    labels = set()
    checked = []
    for scene in scenes:
        label = scene["label"]
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,95}", label) or label in labels:
            raise ValueError("scene labels must be unique non-sensitive identifiers")
        labels.add(label)
        provenance = scene.get("provenance", {})
        if not provenance.get("license") or not provenance.get("sourceUrl"):
            raise ValueError("each input needs explicit source and license metadata")
        wav = Path(scene["wavPath"]).expanduser()
        if not wav.is_absolute():
            wav = path.parent / wav
        count, digest = validate_wave(wav)
        checked.append({"label": label, "wav": wav, "samples": count,
                        "pcm_sha256": digest, "license": provenance["license"],
                        "source_url": provenance["sourceUrl"],
                        "reference_status": provenance.get("referenceStatus", "unknown"),
                        "expected_speech": scene.get("expectedSpeech")})
    return checked


def windows(path):
    import numpy as np
    context = np.zeros(CONTEXT, dtype=np.float32)
    with wave.open(str(path), "rb") as source:
        while data := source.readframes(FRAME):
            chunk = np.frombuffer(data, dtype="<i2").astype(np.float32) / 32768.0
            chunk = np.pad(chunk, (0, FRAME - len(chunk)))
            yield np.concatenate((context, chunk))
            context = chunk[-CONTEXT:]


class OrtHost:
    def __init__(self, model, silence_ms):
        import numpy as np
        import onnxruntime as ort
        options = ort.SessionOptions()
        options.inter_op_num_threads = 1
        options.intra_op_num_threads = 1
        self.session = ort.InferenceSession(str(model), sess_options=options,
                                           providers=["CPUExecutionProvider"])
        self.state = np.zeros((2, 1, 128), dtype=np.float32)
        self.rate = np.array(RATE, dtype=np.int64)
        self.gate = ActivityGate(silence_ms)
        self.silence_ms = silence_ms

    def reset(self):
        self.state.fill(0)
        self.gate = ActivityGate(self.silence_ms)

    def push(self, samples):
        probability, self.state = self.session.run(None, {
            "input": samples[None, :], "state": self.state, "sr": self.rate,
        })
        return self.gate.push(float(probability[0][0]))


class SherpaHost:
    def __init__(self, model, silence_ms):
        import sherpa_onnx
        config = sherpa_onnx.VadModelConfig()
        config.silero_vad.model = str(model)
        config.silero_vad.threshold = 0.5
        # v1.13.8 Python bindings use the native default threshold - 0.15.
        config.silero_vad.min_silence_duration = silence_ms / 1000
        config.silero_vad.min_speech_duration = 0.25
        config.silero_vad.window_size = FRAME
        config.sample_rate = RATE
        config.num_threads = 1
        config.provider = "cpu"
        # Low-level model avoids VoiceActivityDetector's retained audio queue.
        self.model = sherpa_onnx.VadModel.create(config)
        if self.model.window_size() != FRAME + CONTEXT:
            raise ValueError("sherpa model window does not match the pinned model")

    def reset(self):
        self.model.reset()

    def push(self, samples):
        return self.model.is_speech(samples)


def percentile(values, fraction):
    values = sorted(values)
    return values[max(0, math.ceil(len(values) * fraction) - 1)]


def run_host(host, scene, repeats):
    elapsed = []
    reference = None
    decisions = None
    for iteration in range(repeats + 1):
        host.reset()
        decisions = []
        for samples in windows(scene["wav"]):
            start = time.perf_counter_ns()
            active = bool(host.push(samples))
            duration_ms = (time.perf_counter_ns() - start) / 1e6
            decisions.append(active)
            if iteration:
                elapsed.append(duration_ms)
        if reference is not None and decisions != reference:
            raise ValueError("state reset did not reproduce identical decisions")
        reference = decisions
    transitions = []
    previous = False
    for frame, active in enumerate(decisions):
        if active != previous:
            transitions.append({"detected_ms": (frame + 1) * FRAME * 1000 / RATE,
                                "active": active})
        previous = active
    # EOF is not fabricated into a silence boundary.
    return {"frames": len(decisions), "active_frames": sum(decisions),
            "active_at_eof": decisions[-1], "transitions": transitions,
            "median_ms": percentile(elapsed, 0.5),
            "p95_ms": percentile(elapsed, 0.95),
            "p99_ms": percentile(elapsed, 0.99),
            "max_ms": max(elapsed),
            "processing_to_audio_ratio": sum(elapsed) / repeats /
                (scene["samples"] * 1000 / RATE)}, decisions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--silence-ms", type=int, choices=[250, 400, 600], default=400)
    parser.add_argument("--repeats", type=int, choices=range(1, 6), default=3)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output exists; use a fresh evidence path")
    if hashlib.sha256(args.model.read_bytes()).hexdigest() != MODEL_SHA256:
        parser.error("model does not match the pinned 16 kHz Silero SHA256")
    scenes = load_matrix(args.matrix.resolve())
    # Validate every input before initializing either runtime.
    output = {"schema_version": 1, "kind": "offline-vad-host-comparison",
              "model_revision": MODEL_REVISION, "model_sha256": MODEL_SHA256,
              "model_bytes": args.model.stat().st_size,
              "host": {"system": platform.system(), "machine": platform.machine(),
                       "python": platform.python_version()},
              "packages": {name: importlib.metadata.version(name) for name in
                           ("numpy", "onnxruntime", "sherpa-onnx", "sherpa-onnx-core")},
              "config": {"sample_rate": RATE, "frame_samples": FRAME,
                         "context_samples": CONTEXT, "threads": 1,
                         "silence_ms": args.silence_ms, "speech_ms": 250,
                         "repeats": args.repeats, "warmup_passes": 1},
              "initialization_ms": {}, "scenes": [],
              "limits": ["No MT, ASR, native overlay or packaged app was measured.",
                         "Host-call timings include Python binding overhead, exclude PCM I/O.",
                         "Detection timestamps include confirmation delay; they are not speech labels.",
                         "Same weights and hysteresis test hosts, not two model accuracies."]}
    hosts = {}
    for name, factory in [("onnxruntime", OrtHost), ("sherpa-onnx", SherpaHost)]:
        start = time.perf_counter_ns()
        hosts[name] = factory(args.model, args.silence_ms)
        output["initialization_ms"][name] = (time.perf_counter_ns() - start) / 1e6
    for scene in scenes:
        row = {key: value for key, value in scene.items() if key != "wav"}
        row["duration_ms"] = scene["samples"] * 1000 / RATE
        row["hosts"] = {}
        tracks = []
        # Reverse host order on alternating scenes to expose ordering bias.
        names = list(hosts) if len(output["scenes"]) % 2 == 0 else list(reversed(hosts))
        for name in names:
            row["hosts"][name], decisions = run_host(hosts[name], scene, args.repeats)
            tracks.append(decisions)
        if len(tracks[0]) != len(tracks[1]):
            raise ValueError("host frame counts differ")
        row["different_activity_frames"] = sum(a != b for a, b in zip(*tracks))
        output["scenes"].append(row)
        print(json.dumps({"label": scene["label"], "frames": len(tracks[0]),
                          "different_activity_frames": row["different_activity_frames"]}))
    args.output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(args.output, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
        json.dump(output, stream, ensure_ascii=False, indent=2)
        stream.write("\n")


if __name__ == "__main__":
    main()
