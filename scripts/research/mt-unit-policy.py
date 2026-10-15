#!/usr/bin/env python3
"""Offline source-candidate prototype. Never calls ASR/MT or changes Mimi.

This deliberately does not claim semantic understanding. Punctuation and dwell
time nominate provisional units; provider finals remain authoritative. Before
product use this policy must live in shared Rust with actual JNI coverage.
"""
from dataclasses import dataclass
import re
from typing import Optional

MAX_TEXT_BYTES = 16_384
MAX_WAIT_MS = 2_000
STABLE_MS = 600
MIN_REQUEST_SPACING_MS = 1_500
MAX_REQUEST_AGE_MS = 12_000


def complete_prefix(text):
    """Conservative punctuation candidate, not a semantic completeness score."""
    end = 0
    for match in re.finditer(r"[。！？!?]|\.", text):
        index = match.start()
        if text[index] == ".":
            # Decimals, initials, common abbreviations and ellipses are unsafe.
            before = text[:index]
            after = text[index + 1:]
            word = re.search(r"([A-Za-z]+)$", before)
            if (after and (after[0].isalnum() or after[0] == ".")) or before.endswith("."):
                continue
            if word and (len(word[1]) <= 1 or word[1].lower() in {
                    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "vs", "etc"}):
                continue
        end = index + 1
        while end < len(text) and text[end] in '\"\'”’」』）)':
            end += 1
    return text[:end]


@dataclass(frozen=True)
class Request:
    identity: int
    generation: int
    source_prefix: str
    suffix: str
    kind: str
    started_ms: int


class UnitPolicy:
    """One bounded state per audio source/utterance; no audio or history cache.

    A pause alone can request a replaceable preview, never a pinned unit. Only
    successful, still-current responses may advance a provisional source bound.
    The caller must retract presentation on `invalidations` changing. Nothing
    here creates a durable subtitle or suppresses a provider correction/final.
    """
    def __init__(self):
        self.generation = 0
        self.identity = 0
        self.invalidations = 0
        self.last_request_ms: Optional[int] = None
        self.last_clock_ms = 0
        self.reset()

    def reset(self):
        self.generation += 1
        self.text = self.pinned = self.candidate = self.last_attempt = ""
        self.last_attempt_kind = ""
        self.candidate_since = self.changed_ms = self.opened_ms = None
        self.pending: Optional[Request] = None
        # Keep the HTTP budget across reset/reconnect/utterance changes.

    def clock(self, at_ms):
        if at_ms < self.last_clock_ms:
            raise ValueError("non-monotonic replay clock")
        self.last_clock_ms = at_ms

    def observe(self, at_ms, text):
        self.clock(at_ms)
        if len(text.encode("utf-8")) > MAX_TEXT_BYTES:
            raise ValueError("oversized source")
        if not text.strip():
            # A content-free draft is not a clear operation or a new utterance.
            return
        if self.pinned and not text.startswith(self.pinned):
            self.invalidations += 1
            self.reset()
        if self.pending and not text.startswith(self.pending.source_prefix):
            self.pending = None
            self.last_attempt = ""
        if text != self.text:
            self.changed_ms = at_ms
        if self.opened_ms is None and text.strip():
            self.opened_ms = at_ms
        self.text = text
        prefix = complete_prefix(text)
        if prefix != self.candidate:
            self.candidate = prefix
            self.candidate_since = at_ms

    def next(self, at_ms, paused=False):
        self.clock(at_ms)
        if self.pending:
            if at_ms - self.pending.started_ms < MAX_REQUEST_AGE_MS:
                return None
            # An expired response may not move the source boundary.
            self.pending = None
            self.last_attempt = ""
        if self.last_request_ms is not None and at_ms - self.last_request_ms < MIN_REQUEST_SPACING_MS:
            return None
        if not self.text.strip() or self.opened_ms is None:
            return None
        stable = bool(self.candidate and self.candidate.startswith(self.pinned)
                      and len(self.candidate) > len(self.pinned)
                      and at_ms - self.candidate_since >= STABLE_MS)
        maximum_wait = at_ms - self.opened_ms >= MAX_WAIT_MS
        pause_preview = paused and self.changed_ms is not None and at_ms - self.changed_ms >= STABLE_MS
        if stable:
            prefix, kind = self.candidate, "provisional-unit"
        elif maximum_wait or pause_preview:
            prefix, kind = self.text, "replaceable-preview"
        else:
            return None
        if prefix == self.pinned or (prefix, kind) == (self.last_attempt, self.last_attempt_kind):
            return None
        self.identity += 1
        request = Request(self.identity, self.generation, prefix,
                          prefix[len(self.pinned):].strip(), kind, at_ms)
        if not any(character.isalnum() for character in request.suffix):
            return None
        self.pending = request
        self.last_attempt = prefix
        self.last_attempt_kind = kind
        self.last_request_ms = at_ms
        return request

    def complete(self, at_ms, request, *, nonempty_success):
        self.clock(at_ms)
        if (request != self.pending or request.generation != self.generation
                or at_ms - request.started_ms >= MAX_REQUEST_AGE_MS
                or not self.text.startswith(request.source_prefix)):
            return False
        self.pending = None
        if not nonempty_success:
            self.last_attempt = ""
            return False
        if request.kind == "provisional-unit":
            self.pinned = request.source_prefix
            # The remaining tail gets its own bounded preview wait.
            self.opened_ms = at_ms
        return True

    def provider_final(self, at_ms, text):
        self.clock(at_ms)
        if len(text.encode("utf-8")) > MAX_TEXT_BYTES:
            raise ValueError("oversized final")
        self.reset()
        return text  # Authoritative whole source; never drop a frozen prefix.
