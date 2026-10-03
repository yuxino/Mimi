#!/usr/bin/env python3
"""Build the stateless JNI adapter using Rust and the pinned Android NDK."""
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "shared/mimi-android-jni/Cargo.toml"
CONFIG = ROOT / "android/shared-core.properties"
TARGETS = {
    "arm64-v8a": ("aarch64-linux-android", "aarch64-linux-android"),
    "armeabi-v7a": ("armv7-linux-androideabi", "armv7a-linux-androideabi"),
    "x86_64": ("x86_64-linux-android", "x86_64-linux-android"),
    "x86": ("i686-linux-android", "i686-linux-android"),
}


def native_config(path=CONFIG):
    values = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            key, value = line.split("=", 1)
            values[key] = value
    if values.get("ndkVersion") != "27.2.12479018" or values.get("minSdk") != "29":
        raise ValueError("Shared core requires the reviewed NDK version and minSdk")
    abis = values.get("abis", "").split(",")
    if len(abis) != len(set(abis)) or set(abis) != set(TARGETS):
        raise ValueError("Shared core ABI configuration must include all four supported ABIs")
    return values, abis


def ndk_toolchain(ndk, expected_version):
    properties = ndk / "source.properties"
    if not properties.is_file():
        raise ValueError("Pinned Android NDK is missing; install the version in android/shared-core.properties")
    revisions = [line.split("=", 1)[1].strip() for line in properties.read_text().splitlines()
                 if line.split("=", 1)[0].strip() == "Pkg.Revision"]
    if revisions != [expected_version]:
        raise ValueError("Android NDK does not match the pinned version")
    host = {"Darwin": "darwin-x86_64", "Linux": "linux-x86_64", "Windows": "windows-x86_64"}.get(platform.system())
    if host is None:
        raise ValueError("Unsupported Android NDK build host")
    toolchain = ndk / "toolchains/llvm/prebuilt" / host / "bin"
    if not toolchain.is_dir():
        raise ValueError("Pinned Android NDK has no toolchain for this host")
    return toolchain


def host_triple():
    details = subprocess.check_output(["rustc", "-vV"], text=True)
    for line in details.splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise ValueError("Rust did not report its host target")


def build(args):
    config, abis = native_config()
    target_dir = Path(args.target_dir).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    profile = "release" if args.profile == "release" else "debug"
    base_env = os.environ.copy()
    base_env["CARGO_TARGET_DIR"] = str(target_dir)
    command = ["cargo", "build", "--locked", "--manifest-path", str(MANIFEST)]
    if profile == "release":
        command.append("--release")
    if args.platform == "host":
        target = host_triple()
        # Always specify the host so a caller's CARGO_BUILD_TARGET cannot turn
        # JVM test output into an Android library.
        subprocess.run(command + ["--target", target], cwd=ROOT, env=base_env, check=True)
        name = "mimi_android_jni.dll" if "windows" in target else (
            "libmimi_android_jni.dylib" if "apple" in target else "libmimi_android_jni.so")
        shutil.copy2(target_dir / target / profile / name, output / name)
        return
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    ndk = Path(args.ndk).resolve() if args.ndk else (
        Path(sdk) / "ndk" / config["ndkVersion"] if sdk else None)
    if ndk is None:
        raise ValueError("Set ANDROID_HOME or pass --ndk for the pinned Android NDK")
    toolchain = ndk_toolchain(ndk, config["ndkVersion"])
    suffix = ".cmd" if platform.system() == "Windows" else ""
    executable = ".exe" if platform.system() == "Windows" else ""
    for abi in abis:
        target, clang_target = TARGETS[abi]
        linker = toolchain / f"{clang_target}{config['minSdk']}-clang{suffix}"
        if not linker.is_file():
            raise ValueError(f"Pinned Android NDK linker is missing for {abi}")
        env = base_env.copy()
        env[f"CARGO_TARGET_{target.upper().replace('-', '_')}_LINKER"] = str(linker)
        # Encoded flags override inherited RUSTFLAGS consistently, including
        # arguments containing spaces in SDK paths. Do not inherit host flags.
        env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join([
            "-C", "link-arg=-Wl,-z,max-page-size=16384",
            "-C", "link-arg=-Wl,-z,common-page-size=16384",
        ])
        env[f"AR_{target.replace('-', '_')}"] = str(toolchain / f"llvm-ar{executable}")
        subprocess.run(command + ["--target", target], cwd=ROOT, env=env, check=True)
        library = target_dir / target / profile / "libmimi_android_jni.so"
        symbols = subprocess.check_output([str(toolchain / f"llvm-nm{executable}"), "--dynamic", "--defined-only", str(library)], text=True)
        entry_point = "Java_app_yuxino_mimi_android_provider_SharedSubtitleCore_exchangeRaw"
        if not any(line.split() and line.split()[-1] == entry_point for line in symbols.splitlines()):
            raise ValueError(f"Shared core JNI entry point is missing for {abi}")
        destination = output / abi
        destination.mkdir(parents=True, exist_ok=True)
        shutil.copy2(library, destination / library.name)
    subprocess.run([os.environ.get("MIMI_PYTHON", os.sys.executable), str(ROOT / "scripts/verify-shared-core.py"),
                    "--jni-libs", str(output)], cwd=ROOT, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=["host", "android"], required=True)
    parser.add_argument("--profile", choices=["debug", "release"], required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--target-dir", default=str(ROOT / "shared/target"))
    parser.add_argument("--ndk")
    args = parser.parse_args()
    try:
        build(args)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Shared core native build failed: {error}\n")


if __name__ == "__main__":
    main()
