#!/usr/bin/env python3
"""Reject development debugger commands/storage in a production executable."""
import pathlib
import sys

if len(sys.argv) != 2:
    raise SystemExit("Usage: check-production-native-debugger.py /path/to/executable")

data = pathlib.Path(sys.argv[1]).read_bytes()
markers = (
    b"development_debug_start",
    b"development_debug_audio",
    b"development_debug_replay",
    b"development_debug_export",
    b"development_debug_observe",
    b"MIMI_DEVELOPMENT_EVIDENCE_WORKSPACE",
)
if any(marker in data for marker in markers):
    raise SystemExit("Production executable contains development debugger functionality")
print("Production executable excludes development debugger commands and storage selector")
