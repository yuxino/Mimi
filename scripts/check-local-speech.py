#!/usr/bin/env python3
"""Explicit Mac acceptance with synthetic speech and already installed weights.

Never downloads, captures system/microphone audio, or prints recognized text.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import select
import subprocess
import tempfile
import threading
import time
import wave


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, default=Path("/Applications/mimi-dev.app"))
    parser.add_argument("--models", type=Path, required=True)
    parser.add_argument("--model", choices=["qwen-small", "qwen-standard"], required=True)
    args = parser.parse_args()
    fixtures = [
        ("en", "Samantha", "This is a local speech recognition test. The audio stays on this computer.", ["local", "speech", "audio", "computer"]),
        ("zh", "Tingting", "这是本地语音识别测试。声音会留在这台电脑。", ["本地", "语音", "电脑"]),
        ("ja", "Kyoko", "これは音声認識のテストです。音声はこのコンピューターで処理します。", ["音声", "認識"]),
    ]
    for language, voice, sentence, expected in fixtures:
        with tempfile.TemporaryDirectory(prefix="mimi-local-speech-") as temporary:
            aiff, wav = Path(temporary) / "speech.aiff", Path(temporary) / "speech.wav"
            subprocess.run(["/usr/bin/say", "-v", voice, "-o", str(aiff), sentence], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            subprocess.run(["/usr/bin/afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", str(aiff), str(wav)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            with wave.open(str(wav)) as source:
                assert source.getframerate() == 16000 and source.getnchannels() == 1
                audio = source.readframes(source.getnframes())
            assert len(audio) <= 256_000
            started = time.monotonic()
            worker = subprocess.Popen([
                str(args.app / "Contents/MacOS/mimi-local-speech"), args.model,
                str(args.models / args.model), language,
                str(args.app / "Contents/Resources/mlx.metallib"),
            ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            buffer = bytearray()

            def read_event():
                deadline = time.monotonic() + 90
                while b"\n" not in buffer:
                    remaining = deadline - time.monotonic()
                    assert remaining > 0 and select.select([worker.stdout], [], [], remaining)[0], "Worker response timed out"
                    chunk = os.read(worker.stdout.fileno(), 4096)
                    assert chunk, "Worker closed output"
                    buffer.extend(chunk)
                    assert len(buffer) <= 64 * 1024
                line, _, tail = buffer.partition(b"\n")
                buffer[:] = tail
                return json.loads(line)

            def send(command):
                worker.stdin.write(json.dumps(command).encode() + b"\n")
                worker.stdin.flush()

            try:
                assert read_event()["type"] == "ready"
                loaded = time.monotonic()
                # Keep stdin open: catches pipe reads waiting for a full buffer/EOF.
                send({"type": "ping", "id": 1})
                assert read_event() == {"type": "pong", "id": 1}
                send({"type": "start", "id": 1, "revision": 0})
                send({"type": "audio", "audio": base64.b64encode(bytes(640)).decode()})
                send({"type": "clear"})
                send({"type": "ping", "id": 2})
                assert read_event() == {"type": "pong", "id": 2}

                for turn in (2, 3):
                    began = time.monotonic()

                    def write_turn():
                        send({"type": "start", "id": turn, "revision": turn})
                        for offset in range(0, len(audio), 3200):
                            send({"type": "audio", "audio": base64.b64encode(audio[offset:offset + 3200]).decode()})
                        send({"type": "commit"})

                    writer = threading.Thread(target=write_turn, daemon=True)
                    writer.start()
                    drafts = 0
                    while True:
                        event = read_event()
                        assert event["type"] in ("draft", "final")
                        assert event["id"] == turn and event["revision"] == turn
                        if event["type"] == "draft":
                            drafts += 1
                            continue
                        assert all(word in event["text"].lower() for word in expected), "Synthetic speech accuracy check failed"
                        break
                    writer.join(timeout=5)
                    assert not writer.is_alive()
                    print(json.dumps({"model": args.model, "language": language, "turn": turn, "audioSeconds": round(len(audio) / 32000, 2), "loadSeconds": round(loaded - started, 2), "inferenceSeconds": round(time.monotonic() - began, 2), "draftCount": drafts, "finalChecked": True}), flush=True)
                send({"type": "finish"})
                assert read_event()["type"] == "finished"
                worker.wait(timeout=5)
                assert worker.returncode == 0
            finally:
                if worker.poll() is None:
                    worker.kill()
                worker.wait()


if __name__ == "__main__":
    main()
