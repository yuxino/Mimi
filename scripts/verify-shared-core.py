#!/usr/bin/env python3
"""Verify every shared-core ABI and 16 KB native/APK alignment, offline."""
import argparse
import hashlib
from pathlib import Path
import re
import struct
import zipfile

ABIS = {"arm64-v8a": (2, 183), "armeabi-v7a": (1, 40), "x86_64": (2, 62), "x86": (1, 3)}
LIBRARY = "libmimi_android_jni.so"
PAGE_SIZE = 16384
NOTICES = "assets/shared-core.txt"
ROOT = Path(__file__).resolve().parent.parent
LOCKFILE = ROOT / "shared/mimi-android-jni/Cargo.lock"
NOTICE_SOURCE = ROOT / "android/native-licenses/shared-core.txt"


def verify_license_notices(content):
    """Reject stale inventories after dependency changes or stale APK assets."""
    expected = NOTICE_SOURCE.read_bytes()
    if content != expected:
        raise ValueError("Shared core dependency license notices do not match the reviewed inventory")
    digest = hashlib.sha256(LOCKFILE.read_bytes()).hexdigest()
    if f"Cargo-lock SHA256: {digest}".encode() not in content:
        raise ValueError("Shared core dependency license inventory is stale for Cargo.lock")
    text = content.decode("utf-8")
    packages = re.findall(r"^Package: (\S+) (\S+)$", text, re.MULTILINE)
    count = re.search(r"^Inventory package count: (\d+)$", text, re.MULTILINE)
    if not count or len(packages) != int(count[1]) or len(set(packages)) != len(packages):
        raise ValueError("Shared core dependency license inventory has missing or duplicate entries")
    lock_text = LOCKFILE.read_text()
    locked = set()
    for block in lock_text.split("[[package]]")[1:]:
        if '\nsource = "registry+' not in block:
            continue
        name = re.search(r'^name = "([^"]+)"$', block, re.MULTILINE)
        version = re.search(r'^version = "([^"]+)"$', block, re.MULTILINE)
        if name and version:
            locked.add((name[1], version[1]))
    if not set(packages).issubset(locked):
        raise ValueError("Shared core license inventory contains a package absent from Cargo.lock")


def verify_elf(content, abi):
    if len(content) < 64 or content[:4] != b"\x7fELF" or content[5] != 1:
        raise ValueError(f"Invalid little-endian ELF library for {abi}")
    elf_class, machine = ABIS[abi]
    if content[4] != elf_class or struct.unpack_from("<H", content, 18)[0] != machine:
        raise ValueError(f"Native library architecture does not match {abi}")
    if struct.unpack_from("<H", content, 16)[0] != 3:  # ET_DYN
        raise ValueError(f"Native library is not an ELF shared object for {abi}")
    if elf_class == 2:
        offset = struct.unpack_from("<Q", content, 32)[0]
        size, count = struct.unpack_from("<HH", content, 54)
        minimum_size = 56
    else:
        offset = struct.unpack_from("<I", content, 28)[0]
        size, count = struct.unpack_from("<HH", content, 42)
        minimum_size = 32
    if size < minimum_size or count == 0 or offset + size * count > len(content):
        raise ValueError(f"Invalid ELF program headers for {abi}")
    load_segments = 0
    for index in range(count):
        entry = offset + size * index
        if struct.unpack_from("<I", content, entry)[0] == 1:  # PT_LOAD
            alignment = struct.unpack_from("<Q" if elf_class == 2 else "<I", content,
                                           entry + (48 if elf_class == 2 else 28))[0]
            if alignment < PAGE_SIZE or alignment & (alignment - 1):
                raise ValueError(f"Native library LOAD segment is not 16 KB aligned for {abi}")
            values = struct.unpack_from("<QQ" if elf_class == 2 else "<II", content,
                                        entry + (8 if elf_class == 2 else 4))
            if values[0] % alignment != values[1] % alignment:
                raise ValueError(f"Native library LOAD offsets are not congruent for {abi}")
            load_segments += 1
    if load_segments == 0:
        raise ValueError(f"Native library has no LOAD segments for {abi}")


def verify_directory(directory):
    verify_license_notices(NOTICE_SOURCE.read_bytes())
    actual = {path.parent.name for path in directory.glob(f"*/{LIBRARY}")}
    if actual != set(ABIS):
        raise ValueError("Generated shared core libraries do not contain exactly the four supported ABIs")
    for abi in ABIS:
        verify_elf((directory / abi / LIBRARY).read_bytes(), abi)


def verify_apk(path):
    expected = {f"lib/{abi}/{LIBRARY}" for abi in ABIS}
    with zipfile.ZipFile(path) as archive, path.open("rb") as raw:
        if NOTICES not in archive.namelist() or not archive.read(NOTICES).strip():
            raise ValueError("APK is missing the shared core dependency license notices")
        verify_license_notices(archive.read(NOTICES))
        entries = [item for item in archive.infolist() if item.filename.endswith("/" + LIBRARY)]
        if len(entries) != len(expected) or {item.filename for item in entries} != expected:
            raise ValueError("APK must contain exactly one shared core library for each supported ABI")
        for item in entries:
            abi = item.filename.split("/")[1]
            if item.compress_type != zipfile.ZIP_STORED:
                raise ValueError(f"APK shared core library must be uncompressed for {abi}")
            raw.seek(item.header_offset)
            header = raw.read(30)
            if len(header) != 30 or header[:4] != b"PK\x03\x04":
                raise ValueError("Invalid APK local ZIP header")
            name_length, extra_length = struct.unpack_from("<HH", header, 26)
            if (item.header_offset + 30 + name_length + extra_length) % PAGE_SIZE:
                raise ValueError(f"APK shared core library is not ZIP-aligned to 16 KB for {abi}")
            verify_elf(archive.read(item), abi)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    choices = parser.add_mutually_exclusive_group(required=True)
    choices.add_argument("--jni-libs", type=Path)
    choices.add_argument("--apk", type=Path)
    args = parser.parse_args()
    try:
        if args.jni_libs:
            verify_directory(args.jni_libs)
        else:
            verify_apk(args.apk)
    except (ValueError, OSError, zipfile.BadZipFile) as error:
        parser.exit(1, f"Shared core artifact verification failed: {error}\n")
    print("Verified shared core: four ABIs, ELF LOAD alignment and applicable APK ZIP alignment/license notices.")


if __name__ == "__main__":
    main()
