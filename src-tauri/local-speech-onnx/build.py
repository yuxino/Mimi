#!/usr/bin/env python3
"""Build-time preparation only. App startup never runs this downloader.

Pinned release archives are verified before extracting regular library files.
No scripts from the runtime/model repositories are executed.
"""
import hashlib
import json
import os
import platform
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent
HEADER = ("https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/v1.13.8/sherpa-onnx/c-api/c-api.h", "b224a0c25910507d5d1b8a46cae8816bf49724ff1198ca55c98d15f56f3406c2")
JSON_HEADER = ("https://raw.githubusercontent.com/nlohmann/json/v3.12.0/single_include/nlohmann/json.hpp", "aaf127c04cb31c406e5b04a63f1ae89369fccde6d8fa7cdda1ed4f32dfc5de63")

def verified_download(url, destination, digest, size=None):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_file() and hashlib.sha256(destination.read_bytes()).hexdigest() == digest:
        return
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        partial = Path(temporary) / "download"
        request = urllib.request.Request(url, headers={"User-Agent": "Mimi-build"})
        with urllib.request.urlopen(request, timeout=60) as response, partial.open("xb") as output:
            if not response.url.startswith("https://"):
                raise RuntimeError("non-HTTPS redirect")
            count = 0
            while chunk := response.read(1024 * 1024):
                count += len(chunk)
                if count > (size if size is not None else 4 * 1024 * 1024):
                    raise RuntimeError("download size exceeded")
                output.write(chunk)
        if (size is not None and partial.stat().st_size != size) or hashlib.sha256(partial.read_bytes()).hexdigest() != digest:
            raise RuntimeError("download integrity failed")
        os.replace(partial, destination)

def prepare(target):
    assets = json.loads((ROOT / "runtime-assets.json").read_text())
    destination = ROOT.parent / "binaries" / "local-speech-onnx"
    if target not in assets:
        # Preserve the app and custom-program entry on unbundled targets.
        if destination.exists():
            shutil.rmtree(destination)
        destination.mkdir(parents=True)
        (destination / "unsupported.txt").write_text("No bundled ONNX runtime for this target.\n")
        print("ONNX runtime unavailable for build target: " + target)
        return
    asset = assets[target]
    cache = ROOT / ".build" / target
    archive = cache / asset["name"]
    verified_download("https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.8/" + asset["name"], archive, asset["sha256"], asset["bytes"])
    include = cache / "include"
    verified_download(HEADER[0], include / "sherpa-onnx/c-api/c-api.h", HEADER[1])
    verified_download(JSON_HEADER[0], include / "nlohmann/json.hpp", JSON_HEADER[1])
    lib = cache / "lib"
    lib.mkdir(exist_ok=True)
    with tarfile.open(archive) as source:
        for member in source.getmembers():
            parts = Path(member.name).parts
            if len(parts) == 3 and parts[1] in ("lib", "bin") and member.isfile():
                name = parts[2]
                if name in (".", ".."):
                    raise RuntimeError("unsafe archive")
                with source.extractfile(member) as content, (lib / name).open("wb") as output:
                    shutil.copyfileobj(content, output)
    build = cache / "worker"
    command = ["cmake", "-S", str(ROOT), "-B", str(build), "-DCMAKE_BUILD_TYPE=Release", "-DSHERPA_ROOT=" + str(cache)]
    if target.endswith("apple-darwin"):
        command += ["-DCMAKE_OSX_ARCHITECTURES=" + ("arm64" if target.startswith("aarch64") else "x86_64"), "-DCMAKE_OSX_DEPLOYMENT_TARGET=13.0"]
    if target.endswith("windows-msvc"):
        command += ["-A", "ARM64" if target.startswith("aarch64") else "x64"]
    subprocess.run(command, check=True)
    subprocess.run(["cmake", "--build", str(build), "--config", "Release", "--parallel", "2"], check=True)
    test_env = os.environ.copy()
    if "windows" in target:
        test_env["PATH"] = str(lib) + os.pathsep + test_env.get("PATH", "")
    machine = platform.machine().lower()
    native_arch = target.startswith("aarch64") == (machine in ("arm64", "aarch64"))
    if native_arch:
        subprocess.run(["ctest", "--test-dir", str(build), "-C", "Release", "--output-on-failure"], check=True, env=test_env)
    else:
        print("Cross-architecture worker compiled; native tests require the target device.")
    exe = "mimi-local-onnx.exe" if "windows" in target else "mimi-local-onnx"
    binary = build / ("Release" if "windows" in target else "") / exe
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=destination.parent) as temporary:
        staged = Path(temporary) / "runtime"
        staged.mkdir()
        shutil.copy2(binary, staged / exe)
        (staged / exe).chmod(0o755)
        for path in lib.iterdir():
            if path.suffix in (".so", ".dll", ".dylib"):
                shutil.copy2(path, staged / path.name)
                (staged / path.name).chmod(0o644)
        if destination.exists():
            shutil.rmtree(destination)
        os.replace(staged, destination)

if __name__ == "__main__":
    prepare(sys.argv[1])
