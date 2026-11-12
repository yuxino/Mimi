#!/usr/bin/env python3
"""Select compile/test scope; ordinary changes never request installation packages."""
import json
import os
import subprocess
from pathlib import Path

ALL_OS = ["macos-latest", "windows-2025", "ubuntu-22.04"]
PACKAGES = {"macos": "macos-latest", "windows": "windows-2025", "linux": "ubuntu-22.04"}
NATIVE_DOCUMENT_INPUTS = {
    "docs/development/debugger-evidence.md",
    "docs/demos/english-film/response.json",
}


def is_editorial_document(path):
    # Release/demo fixtures and debugger-evidence.md are consumed by tests/code.
    # Unknown paths remain checked, even when they happen to end in .md.
    return path in {
        "AGENTS.md", "README.md", "README_EN.md",
        ".github/CONTRIBUTING.md", ".github/SECURITY.md",
        "docs/usage.md", "docs/usage.zh-CN.md", "docs/usage.ja.md",
    } or (path.endswith(".md")
          and path.startswith(("docs/development/", "docs/plans/", "docs/qa/", "docs/readme/"))
          and path not in NATIVE_DOCUMENT_INPUTS)


def plan(paths, full=False, package="none", release=False):
    if package not in ["none", "all", *PACKAGES]:
        raise ValueError("Unknown package selection")
    affected = set(ALL_OS) if full or release or paths is None else set()
    for path in paths or []:
        if path == "src-tauri/src/audio/windows.rs" or path.startswith("scripts/windows-"):
            affected.add("windows-2025")
        elif path == "src-tauri/src/audio/macos.rs" or path.startswith("scripts/macos-"):
            affected.add("macos-latest")
        elif path == "src-tauri/src/audio/linux.rs" or path.startswith("scripts/linux-"):
            affected.add("ubuntu-22.04")
        elif path in NATIVE_DOCUMENT_INPUTS or path.startswith((
                "src-tauri/", "scripts/", ".github/workflows/", "shared/",
                "android/app/src/main/java/app/yuxino/mimi/android/provider/")):
            affected.update(ALL_OS)
    bundles = ALL_OS[:] if package == "all" else [PACKAGES[package]] if package in PACKAGES else []
    affected.update(bundles)
    return {
        "frontend": full or release or paths is None or bool(bundles)
                    or any(not is_editorial_document(path) for path in paths or []),
        "native": bool(affected),
        "rust_os": [item for item in ALL_OS if item in affected] or ["ubuntu-22.04"],
        "windows_arm": "windows-2025" in affected,
        "arm_bundle": release or "windows-2025" in bundles,
        "bundle": bool(bundles) and not release,
        "bundle_os": bundles or ["ubuntu-22.04"],
        "release": release,
    }


def changed_paths():
    base = os.environ.get("CI_BASE_SHA", "")
    if not base or set(base) == {"0"}:
        try:
            base = subprocess.check_output(["git", "merge-base", "origin/main", "HEAD"], text=True).strip()
        except subprocess.CalledProcessError:
            return None  # Failed diff must expand checks, never silently bypass them.
    try:
        data = subprocess.check_output(["git", "diff", "--name-only", "--no-renames", "-z", base, "HEAD"])
    except subprocess.CalledProcessError:
        return None
    return [name.decode("utf-8", "surrogateescape") for name in data.split(b"\0") if name]


def main():
    result = plan(changed_paths(), full=os.environ.get("CI_VALIDATION") == "full", package=os.environ.get("CI_PACKAGES", "none") or "none", release=os.environ.get("GITHUB_REF_TYPE") == "tag")
    outputs = {key: json.dumps(value, separators=(",", ":")) for key, value in result.items()}
    if os.environ.get("GITHUB_OUTPUT"):
        with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
            for key, value in outputs.items():
                output.write(key + "=" + value + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
