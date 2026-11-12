#!/usr/bin/env python3
"""Verify every shared-core ABI and 16 KB native/APK alignment, offline."""
import argparse
import hashlib
import json
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
# These Kotlin implementations were retired. Match DEX class descriptors
# (including Kotlin file facades/inner types),
# not short display labels. A stale/generated source must not repack them.
LEGACY_RUNTIME_DESCRIPTOR = re.compile(
    rb"Lapp/yuxino/mimi/android/provider/(?:"
    rb"DashScopeEngine|OpenAIRealtimeEngine|StreamingServiceEngine|CloudProtocols|"
    rb"VolcanoProtocol|GrokProtocol|TranslationPipeline|OpenAITranslationClient|"
    rb"DeepLTranslationClient|DeepLXTranslationClient|ProviderLanguageCatalogs|"
    rb"DeepLLanguages|ProviderFinishGate|TranscriptBuffer|SharedLivePairStream|"
    rb"SharedTranscriptStream|TranslationClient|BoundedTranslationHttpClient|"
    rb"ServiceEvent|WireFrame|ServiceProtocol|TencentProtocol|TencentProviderException|"
    rb"BaiduProtocol|TurnText|GeminiProtocol|AzureProtocol|VolcanoWire|"
    rb"FinalTranslationPolicy|TranslationDeadlines)(?:Kt)?(?:\$[^;\x00]*)?;"
)
MAPPING_HEADER = re.compile(r"#\s*(compiler|compiler_version|min_api|pg_map_id|pg_map_hash):\s*(.*)")
MAPPING_CLASS = re.compile(r"([^\s:()/;,]+) -> ([^\s:()/;,]+):")
MAPPING_MEMBER = re.compile(
    r"\s+(?P<minified>\d+(?::\d+)?:)?[^\s:()]+ (?P<name>[^\s:()]+)"
    r"(?P<arguments>\([^()\r\n]*\))?(?P<original>:\d+(?::\d+)?)? -> [^\s:()]+"
)


def verify_license_notices(content):
    """Reject stale inventories after dependency changes or stale APK assets."""
    # Git may check text out with CRLF on Windows. Hash/compare the reviewed
    # LF representation, without ignoring any dependency or notice content.
    content = content.replace(b"\r\n", b"\n")
    expected = NOTICE_SOURCE.read_bytes().replace(b"\r\n", b"\n")
    if content != expected:
        raise ValueError("Shared core dependency license notices do not match the reviewed inventory")
    lock_bytes = LOCKFILE.read_bytes().replace(b"\r\n", b"\n")
    digest = hashlib.sha256(lock_bytes).hexdigest()
    if f"Cargo-lock SHA256: {digest}".encode() not in content:
        raise ValueError("Shared core dependency license inventory is stale for Cargo.lock")
    text = content.decode("utf-8")
    packages = re.findall(r"^Package: (\S+) (\S+)$", text, re.MULTILINE)
    count = re.search(r"^Inventory package count: (\d+)$", text, re.MULTILINE)
    if not count or len(packages) != int(count[1]) or len(set(packages)) != len(packages):
        raise ValueError("Shared core dependency license inventory has missing or duplicate entries")
    lock_text = lock_bytes.decode("utf-8")
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


def dex_r8_markers(content):
    """Read marker strings only, with bounds checks; this is not a bytecode verifier."""
    if (len(content) < 112 or content[:4] != b"dex\n" or content[7] != 0
            or content[4:7] not in (b"035", b"037", b"038", b"039", b"040")
            or struct.unpack_from("<III", content, 32) != (len(content), 112, 0x12345678)):
        raise ValueError("Invalid or unsupported DEX header")
    count, table = struct.unpack_from("<II", content, 56)
    if (count == 0 and table != 0) or (count and (table < 112 or table + count * 4 > len(content))):
        raise ValueError("Invalid DEX string table")

    def unique_keys(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("Malformed R8 DEX marker: duplicate key")
            result[key] = value
        return result

    markers = []
    for index in range(count):
        offset = struct.unpack_from("<I", content, table + index * 4)[0]
        if not table + count * 4 <= offset < len(content):
            raise ValueError("Invalid DEX string offset")
        length = 0
        for shift in range(0, 35, 7):
            if offset >= len(content):
                raise ValueError("Truncated DEX string length")
            byte = content[offset]
            offset += 1
            length |= (byte & 0x7f) << shift
            if byte < 0x80:
                break
        else:
            raise ValueError("Invalid DEX string length")
        if length > 0xffffffff:
            raise ValueError("Invalid DEX string length")
        end = content.find(b"\0", offset)
        if end < 0:
            raise ValueError("Unterminated DEX string")
        if not content.startswith(b"~~R8", offset):
            continue
        try:
            text = content[offset:end].decode("utf-8")
            if len(text.encode("utf-16-le")) // 2 != length:
                raise ValueError("Invalid marker length")
            marker = json.loads(text[4:], object_pairs_hook=unique_keys)
        except (UnicodeError, ValueError) as error:
            raise ValueError("Malformed R8 DEX marker") from error
        if (not isinstance(marker, dict) or marker.get("backend") != "dex"
                or not isinstance(marker.get("version"), str) or not marker["version"]
                or type(marker.get("min-api")) is not int or marker["min-api"] < 1
                or not isinstance(marker.get("pg-map-id"), str)
                or not re.fullmatch(r"[0-9a-f]{1,64}", marker["pg-map-id"])):
            raise ValueError("Malformed R8 DEX marker metadata")
        markers.append(marker)
    return markers


def verify_r8_mapping(path, marker):
    """Check R8's content hash and original class/member owners without retaining the map."""
    metadata = {}
    digest = hashlib.sha256()
    body = False
    class_seen = False
    legacy_owner = False
    with path.open("rb") as source:
        for raw_line in source:
            if body:
                digest.update(raw_line)
            try:
                line = raw_line.decode("utf-8").rstrip("\r\n")
            except UnicodeError as error:
                raise ValueError("Invalid R8 mapping encoding") from error
            header = MAPPING_HEADER.fullmatch(line.strip())
            if header:
                key, value = header.groups()
                if body or key in metadata:
                    raise ValueError("Duplicate or misplaced R8 mapping metadata")
                metadata[key] = value
                if key == "pg_map_hash":
                    if not raw_line.endswith(b"\n"):
                        raise ValueError("Invalid R8 mapping hash marker")
                    body = True
                continue
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            if not body:
                raise ValueError("R8 mapping is missing its hash metadata")
            class_mapping = MAPPING_CLASS.fullmatch(line)
            if class_mapping:
                class_seen = True
                owner = class_mapping[1]
            else:
                member = MAPPING_MEMBER.fullmatch(line)
                if (not class_seen or not member
                        or (member["arguments"] is None and (member["minified"] or member["original"]))):
                    raise ValueError("Invalid R8 mapping class or member record")
                owner, separator, name = member["name"].rpartition(".")
                if not separator:
                    continue
                if not owner or not name:
                    raise ValueError("Invalid R8 mapping member owner")
            descriptor = b"L" + owner.replace(".", "/").encode("utf-8") + b";"
            legacy_owner |= LEGACY_RUNTIME_DESCRIPTOR.fullmatch(descriptor) is not None
    expected_hash = metadata.get("pg_map_hash", "")
    if (not body or not class_seen or metadata.get("compiler") != "R8"
            or not metadata.get("compiler_version")
            or not re.fullmatch(r"[1-9][0-9]*", metadata.get("min_api", ""))
            or not re.fullmatch(r"SHA-256 [0-9a-f]{64}", expected_hash)
            or not re.fullmatch(r"[0-9a-f]{1,64}", metadata.get("pg_map_id", ""))):
        raise ValueError("Missing or invalid R8 mapping metadata")
    actual_hash = digest.hexdigest()
    if expected_hash != "SHA-256 " + actual_hash:
        raise ValueError("R8 mapping content hash does not match")
    # Mimi uses R8's default content-derived ID, not a custom MapIdProvider.
    if not actual_hash.startswith(metadata["pg_map_id"]):
        raise ValueError("R8 mapping ID does not match its content hash")
    if (metadata["pg_map_id"] != marker["pg-map-id"]
            or metadata["compiler_version"] != marker["version"]
            or int(metadata["min_api"]) != marker["min-api"]):
        raise ValueError("R8 mapping does not match APK marker")
    if legacy_owner:
        raise ValueError("APK contains offline legacy runtime in R8 mapping")


def verify_apk(path, mapping=None):
    expected = {f"lib/{abi}/{LIBRARY}" for abi in ABIS}
    with zipfile.ZipFile(path) as archive, path.open("rb") as raw:
        if NOTICES not in archive.namelist() or not archive.read(NOTICES).strip():
            raise ValueError("APK is missing the shared core dependency license notices")
        verify_license_notices(archive.read(NOTICES))
        markers = []
        for item in archive.infolist():
            if re.fullmatch(r"classes(?:\d+)?\.dex", item.filename):
                content = archive.read(item)
                if LEGACY_RUNTIME_DESCRIPTOR.search(content):
                    raise ValueError(f"APK contains offline legacy runtime in {item.filename}")
                markers.extend(dex_r8_markers(content))
        if markers:
            if mapping is None:
                raise ValueError("R8 APK requires its mapping file")
            if any(marker != markers[0] for marker in markers[1:]):
                raise ValueError("APK has conflicting R8 markers")
            verify_r8_mapping(mapping, markers[0])
        elif mapping is not None:
            raise ValueError("R8 mapping supplied for APK without R8 marker")
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
    parser.add_argument("--mapping", type=Path, help="Required R8 mapping for an optimized APK")
    args = parser.parse_args()
    if args.mapping and not args.apk:
        parser.error("--mapping requires --apk")
    try:
        if args.jni_libs:
            verify_directory(args.jni_libs)
        else:
            verify_apk(args.apk, args.mapping)
    except (ValueError, OSError, zipfile.BadZipFile) as error:
        parser.exit(1, f"Shared core artifact verification failed: {error}\n")
    print("Verified shared core: four ABIs, ELF LOAD alignment and applicable APK ZIP alignment/license notices/runtime isolation.")


if __name__ == "__main__":
    main()
