#!/usr/bin/env python3
"""Local CustomOpenAIASR timing peer for an isolated synthetic capture run."""

import argparse
import asyncio
import base64
import itertools
import json
import logging
import math
import os
from pathlib import Path
import signal
import struct
import tempfile
import time


SAMPLE_RATE = 24_000
TONE_HZ = 997
WINDOW_SAMPLES = SAMPLE_RATE // 10
PCM_WINDOW = struct.Struct(f"<{WINDOW_SAMPLES}h")
TONE_PHASES = [math.tau * TONE_HZ * i / SAMPLE_RATE for i in range(WINDOW_SAMPLES)]
TONE_COS = tuple(map(math.cos, TONE_PHASES))
TONE_SIN = tuple(map(math.sin, TONE_PHASES))
PCM_FORMAT = {"type": "audio/pcm", "rate": SAMPLE_RATE}
MAX_PCM_BYTES = 128 * 1024
CAPTION = "Mimi synthetic 997 Hz caption"
TRANSCRIPTION_EVENT = "conversation.item.input_audio_transcription"


class ProtocolError(Exception):
    pass


class ToneDetector:
    def __init__(self):
        self.buffer = bytearray()

    def push(self, pcm):
        self.buffer.extend(pcm)
        while len(self.buffer) >= PCM_WINDOW.size:
            samples = PCM_WINDOW.unpack_from(self.buffer)
            del self.buffer[:PCM_WINDOW.size]
            power = sum(sample * sample for sample in samples)
            if power < WINDOW_SAMPLES * (0.015 * 32768) ** 2:
                continue
            cosine = sum(sample * coefficient for sample, coefficient in zip(samples, TONE_COS))
            sine = sum(sample * coefficient for sample, coefficient in zip(samples, TONE_SIN))
            if 2 * (cosine ** 2 + sine ** 2) >= 0.70 * WINDOW_SAMPLES * power:
                self.buffer.clear()
                return True
        return False


def record(event, connection=None, *, at_ns=None, **counts):
    row = {"event": event, "monotonic_ns": time.monotonic_ns() if at_ns is None else at_ns}
    if connection is not None:
        row["connection"] = connection
    print(json.dumps({**row, **counts}, separators=(",", ":")), flush=True)


def read_message(raw):
    if not isinstance(raw, str):
        raise ProtocolError
    try:
        message = json.loads(raw)
    except ValueError:
        raise ProtocolError from None
    if not isinstance(message, dict):
        raise ProtocolError
    return message


def acknowledge_session(message):
    try:
        session = message["session"]
        audio = session["audio"]["input"]
        model = audio["transcription"]["model"]
        if (session["type"] != "transcription" or audio["format"] != PCM_FORMAT
                or audio["turn_detection"] is not None or not isinstance(model, str) or not model):
            raise ProtocolError
    except (KeyError, TypeError):
        raise ProtocolError from None
    return {
        "type": "session.updated",
        "session": {"type": "transcription", "audio": {"input": {
            "format": PCM_FORMAT, "transcription": {"model": model}, "turn_detection": None,
        }}},
    }


def audio_chunk(message):
    try:
        pcm = base64.b64decode(message["audio"], validate=True)
    except (KeyError, TypeError, ValueError):
        raise ProtocolError from None
    if len(pcm) > MAX_PCM_BYTES or len(pcm) % 2:
        raise ProtocolError
    return pcm


async def send(socket, message):
    await socket.send(json.dumps(message, separators=(",", ":")))


async def serve_connection(socket, connection):
    detector = ToneDetector()
    setup = False
    first_pcm = True
    caption_started = False
    matched_turn = False
    turn = 1

    async for raw in socket:
        received_ns = time.monotonic_ns()
        message = read_message(raw)
        kind = message.get("type")
        if kind == "session.update":
            await send(socket, acknowledge_session(message))
            setup = True
            record("session_ready", connection)
            continue
        if not setup:
            raise ProtocolError

        item = {"item_id": f"fixture-{turn}", "content_index": 0}
        if kind == "input_audio_buffer.append":
            pcm = audio_chunk(message)
            if pcm and first_pcm:
                record("first_pcm", connection, at_ns=received_ns, bytes=len(pcm))
                first_pcm = False
            if not caption_started and detector.push(pcm):
                record("matching_pcm", connection, at_ns=received_ns, window_samples=WINDOW_SAMPLES)
                # The client can bind a live delta before its local VAD commits.
                await send(socket, {"type": f"{TRANSCRIPTION_EVENT}.delta", **item, "delta": CAPTION})
                record("reply", connection)
                caption_started = True
                matched_turn = True
        elif kind == "input_audio_buffer.commit":
            record("commit", connection, at_ns=received_ns)
            await send(socket, {"type": "input_audio_buffer.committed", "item_id": item["item_id"]})
            await send(socket, {"type": f"{TRANSCRIPTION_EVENT}.completed", **item,
                                "transcript": CAPTION if matched_turn else ""})
            if matched_turn:
                record("final", connection)
            matched_turn = False
            detector = ToneDetector()
            turn += 1
        elif kind == "input_audio_buffer.clear":
            matched_turn = False
            detector = ToneDetector()
            await send(socket, {"type": "input_audio_buffer.cleared"})
        else:
            raise ProtocolError


def publish_port(ready_file, port):
    # Publish a complete port without replacing any caller file.
    with tempfile.NamedTemporaryFile(mode="w", dir=ready_file.parent) as temporary:
        temporary.write(f"{port}\n")
        temporary.flush()
        os.link(temporary.name, ready_file)


async def run(ready_file):
    from websockets import serve
    from websockets.exceptions import ConnectionClosed

    stopped = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stopped.set)

    connections = itertools.count(1)
    # Ubuntu 22.04's websockets uses named loggers and has no logger argument.
    logging.getLogger("websockets.server").disabled = True
    logging.getLogger("websockets.protocol").disabled = True

    async def handler(socket, _path=None):
        connection = next(connections)
        record("open", connection)
        try:
            await serve_connection(socket, connection)
        except ProtocolError:
            record("protocol_error", connection)
            await socket.close(code=1002)
        except ConnectionClosed:
            pass
        finally:
            record("close", connection, code=socket.close_code)

    async with serve(handler, "127.0.0.1", 0, compression=None, max_size=1024 * 1024,
                     max_queue=8, close_timeout=0.5) as server:
        port = server.sockets[0].getsockname()[1]
        publish_port(ready_file, port)
        record("ready", port=port)
        await stopped.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ready-file", required=True, type=Path,
                        help="New file in a caller-owned directory for the loopback port")
    args = parser.parse_args()
    asyncio.run(run(args.ready_file))


if __name__ == "__main__":
    main()
