"""Build a scratch input driver with the existing native window and clipping checks."""
from pathlib import Path
import os
import subprocess
import sys

suite, output = map(Path, sys.argv[1:])
product = (suite / "translating-scroll.swift").read_text()
prefix, entry, _ = product.partition("@main @MainActor")
if not entry:
    raise SystemExit("Expected existing native test entry point")
def replace_once(before, after):
    global prefix
    if prefix.count(before) != 1:
        raise SystemExit(f"Native test helper changed: {before}")
    prefix = prefix.replace(before, after)
replace_once("inspectHistory: Bool = false) throws -> Sample", "inspectHistory: Bool = false, expectedNames: [String: String]? = nil) throws -> Sample")
replace_once("let expected = inspectHistory ? historyNames : names", "let expected = expectedNames ?? (inspectHistory ? historyNames : names)")
source = output.parent / "HoverInputCombined.swift"
source.write_text(prefix + (Path(__file__).parent / "HoverInput.swift").read_text())
cache = os.environ.get("SWIFT_MODULECACHE_PATH")
args = ["-module-cache-path", cache] if cache else []
subprocess.run(["xcrun", "swiftc", *args, str(source), str(suite / "Capture.swift"), "-o", str(output)], check=True)
