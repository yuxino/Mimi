#!/usr/bin/env python3
"""Regression tests for native ABI, toolchain pinning and packaging boundaries."""
import hashlib
import importlib.util
import json
from pathlib import Path
from unittest import mock
import struct
import sys
import tempfile
import unittest
import zipfile


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


build = load("shared_core_build", "build-shared-core.py")
verify = load("shared_core_verify", "verify-shared-core.py")


def elf(abi, alignment=16384):
    elf_class, machine = verify.ABIS[abi]
    content = bytearray(128)
    content[:6] = b"\x7fELF" + bytes([elf_class, 1])
    struct.pack_into("<H", content, 16, 3)
    struct.pack_into("<H", content, 18, machine)
    if elf_class == 2:
        struct.pack_into("<Q", content, 32, 64)
        struct.pack_into("<HH", content, 54, 56, 1)
        struct.pack_into("<Q", content, 64 + 48, alignment)
    else:
        struct.pack_into("<I", content, 28, 64)
        struct.pack_into("<HH", content, 42, 32, 1)
        struct.pack_into("<I", content, 64 + 28, alignment)
    struct.pack_into("<I", content, 64, 1)
    return bytes(content)


def dex_strings(*strings):
    """A bounded string-table fixture; no Android compiler or executable code."""
    content = bytearray(112 + 4 * len(strings))
    data_offset = len(content)
    for index, value in enumerate(strings):
        struct.pack_into("<I", content, 112 + index * 4, len(content))
        length = len(value.decode("utf-8").encode("utf-16-le")) // 2
        while length >= 0x80:
            content.append((length & 0x7f) | 0x80)
            length >>= 7
        content.append(length)
        content.extend(value + b"\0")
    content[:8] = b"dex\n035\0"
    struct.pack_into("<III", content, 32, len(content), 112, 0x12345678)
    struct.pack_into("<II", content, 56, len(strings), 112 if strings else 0)
    struct.pack_into("<II", content, 104, len(content) - data_offset, data_offset if strings else 0)
    return bytes(content)


def r8_mapping(path, body=(b"app.yuxino.mimi.android.provider.SharedRuntimeEngine -> a:\n"
                           b"    1:1:void run():2:2 -> a\n")):
    digest = hashlib.sha256(body).hexdigest()
    marker = {"backend": "dex", "version": "synthetic-r8", "min-api": 29, "pg-map-id": digest[:7]}
    header = (f"# compiler: R8\n# compiler_version: synthetic-r8\n# min_api: 29\n"
              f"# pg_map_id: {digest[:7]}\n# pg_map_hash: SHA-256 {digest}\n").encode()
    path.write_bytes(header + body)
    return marker


def r8_marker(marker):
    return b"~~R8" + json.dumps(marker, separators=(",", ":")).encode()


def apk(path, abis=None, aligned=True, compressed=False, duplicate=False,
        notices=verify.NOTICE_SOURCE.read_bytes(), dex=None):
    with zipfile.ZipFile(path, "w") as archive:
        if notices is not None:
            archive.writestr(verify.NOTICES, notices)
        for name, content in (dex or {}).items():
            archive.writestr(name, content)
        for abi in abis or verify.ABIS:
            name = f"lib/{abi}/{verify.LIBRARY}"
            item = zipfile.ZipInfo(name)
            item.compress_type = zipfile.ZIP_DEFLATED if compressed else zipfile.ZIP_STORED
            if aligned:
                # A valid private ZIP extra field pads the local data offset.
                size = (-(archive.fp.tell() + 30 + len(name.encode()))) % 16384
                if size < 4:
                    size += 16384
                item.extra = struct.pack("<HH", 0xCAFE, size - 4) + bytes(size - 4)
            archive.writestr(item, elf(abi))
        if duplicate:
            import warnings
            with warnings.catch_warnings():
                warnings.simplefilter("ignore", UserWarning)
                archive.writestr(name, elf(abi))


class NativeBuildTests(unittest.TestCase):
    def test_windows_crlf_checkout_preserves_reviewed_inventory_and_rejects_changes(self):
        notices = verify.NOTICE_SOURCE.read_bytes().replace(b"\r\n", b"\n")
        lock = verify.LOCKFILE.read_bytes().replace(b"\r\n", b"\n")
        with tempfile.TemporaryDirectory() as root:
            notice_path = Path(root) / "shared-core.txt"
            lock_path = Path(root) / "Cargo.lock"
            notice_path.write_bytes(notices.replace(b"\n", b"\r\n"))
            lock_path.write_bytes(lock.replace(b"\n", b"\r\n"))
            with mock.patch.object(verify, "NOTICE_SOURCE", notice_path), mock.patch.object(verify, "LOCKFILE", lock_path):
                verify.verify_license_notices(notices)
                verify.verify_license_notices(notice_path.read_bytes())
                with self.assertRaisesRegex(ValueError, "reviewed inventory"):
                    verify.verify_license_notices(notices + b"Changed notice\n")
                lock_path.write_bytes(lock_path.read_bytes() + b"# changed dependency graph\r\n")
                with self.assertRaisesRegex(ValueError, "stale for Cargo.lock"):
                    verify.verify_license_notices(notices)

    def test_reviewed_toolchain_and_four_abis_are_pinned(self):
        config, abis = build.native_config()
        self.assertEqual(config["minSdk"], "29")
        self.assertEqual(len(abis), 4)
        self.assertEqual(set(abis), set(verify.ABIS))
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "config.properties"
            path.write_text("ndkVersion=26.0.0\nminSdk=29\nabis=arm64-v8a\n")
            with self.assertRaisesRegex(ValueError, "reviewed NDK"):
                build.native_config(path)

    def test_missing_or_wrong_ndk_fails_before_building(self):
        with tempfile.TemporaryDirectory() as root:
            ndk = Path(root)
            with self.assertRaisesRegex(ValueError, "missing"):
                build.ndk_toolchain(ndk, "27.2.12479018")
            (ndk / "source.properties").write_text("Pkg.Revision = 26.3.11579264\n")
            with self.assertRaisesRegex(ValueError, "pinned version"):
                build.ndk_toolchain(ndk, "27.2.12479018")

    def test_all_abi_elfs_require_matching_machine_and_16kb_loads(self):
        for abi in verify.ABIS:
            verify.verify_elf(elf(abi), abi)
            with self.assertRaisesRegex(ValueError, "16 KB aligned"):
                verify.verify_elf(elf(abi, 4096), abi)
        with self.assertRaisesRegex(ValueError, "architecture"):
            verify.verify_elf(elf("x86_64"), "arm64-v8a")
        with self.assertRaisesRegex(ValueError, "ELF"):
            verify.verify_elf(b"not an ELF", "arm64-v8a")
        content = bytearray(elf("arm64-v8a"))
        struct.pack_into("<H", content, 16, 2)
        with self.assertRaisesRegex(ValueError, "shared object"):
            verify.verify_elf(content, "arm64-v8a")
        content = bytearray(elf("arm64-v8a"))
        struct.pack_into("<Q", content, 64 + 8, 1)
        with self.assertRaisesRegex(ValueError, "congruent"):
            verify.verify_elf(content, "arm64-v8a")

    def test_truncated_headers_or_no_load_segments_fail(self):
        content = bytearray(elf("arm64-v8a"))
        struct.pack_into("<HH", content, 54, 56, 10)
        with self.assertRaisesRegex(ValueError, "program headers"):
            verify.verify_elf(content, "arm64-v8a")
        content = bytearray(elf("arm64-v8a"))
        struct.pack_into("<I", content, 64, 0)
        with self.assertRaisesRegex(ValueError, "no LOAD"):
            verify.verify_elf(content, "arm64-v8a")

    def test_generated_library_set_cannot_silently_omit_an_abi(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            for abi in verify.ABIS:
                (directory / abi).mkdir()
                (directory / abi / verify.LIBRARY).write_bytes(elf(abi))
            verify.verify_directory(directory)
            (directory / "x86" / verify.LIBRARY).unlink()
            with self.assertRaisesRegex(ValueError, "four supported ABIs"):
                verify.verify_directory(directory)

    def test_apk_accepts_all_four_uncompressed_16kb_libraries(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "app.apk"
            apk(path)
            verify.verify_apk(path)

    def test_apk_rejects_legacy_runtime_in_primary_or_secondary_dex(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "app.apk"
            for name, descriptor in [
                ("classes.dex", b"Lapp/yuxino/mimi/android/provider/DashScopeEngine;"),
                ("classes2.dex", b"Lapp/yuxino/mimi/android/provider/OpenAITranslationClient$check$1;"),
                ("classes3.dex", b"Lapp/yuxino/mimi/android/provider/ProviderLanguageCatalogsKt;"),
                ("classes4.dex", b"Lapp/yuxino/mimi/android/provider/BoundedTranslationHttpClient;"),
                ("classes5.dex", b"Lapp/yuxino/mimi/android/provider/TencentProtocol;"),
            ]:
                with self.subTest(dex=name):
                    apk(path, dex={name: dex_strings(descriptor)})
                    with self.assertRaisesRegex(ValueError, "offline legacy runtime"):
                        verify.verify_apk(path)

    def test_apk_allows_shared_runtime_adapter_and_probe_types(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "app.apk"
            apk(path, dex={"classes.dex": dex_strings(b'~~D8{"backend":"dex","version":"synthetic-d8"}', *(
                b"Lapp/yuxino/mimi/android/provider/" + name + b";"
                for name in (b"SharedRuntimeEngine", b"NativeTextProbeClient", b"TranslationResult$Success")
            ))})
            verify.verify_apk(path)

    def test_r8_mapping_matches_markers_in_any_dex_without_requiring_one_per_dex(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            marker = r8_marker(r8_mapping(mapping))
            for marker_dex in ["classes.dex", "classes2.dex"]:
                with self.subTest(marker_dex=marker_dex):
                    entries = {"classes.dex": dex_strings(b"La;"), "classes2.dex": dex_strings(b"Lb;")}
                    entries[marker_dex] = dex_strings(marker, b"La;")
                    apk(path, dex=entries)
                    verify.verify_apk(path, mapping)
            apk(path, dex={"classes.dex": dex_strings(marker), "classes2.dex": dex_strings(marker)})
            verify.verify_apk(path, mapping)

    def test_r8_requires_mapping_and_rejects_an_unmarked_apk_with_mapping(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            marker = r8_mapping(mapping)
            apk(path, dex={"classes.dex": dex_strings(r8_marker(marker))})
            with self.assertRaisesRegex(ValueError, "requires its mapping"):
                verify.verify_apk(path)
            with self.assertRaises(FileNotFoundError):
                verify.verify_apk(path, Path(root) / "missing.txt")
            mapping.write_bytes(b"")
            with self.assertRaisesRegex(ValueError, "mapping metadata"):
                verify.verify_apk(path, mapping)
            empty_marker = r8_mapping(mapping, b"")
            apk(path, dex={"classes.dex": dex_strings(r8_marker(empty_marker))})
            with self.assertRaisesRegex(ValueError, "mapping metadata"):
                verify.verify_apk(path, mapping)
            apk(path, dex={"classes.dex": dex_strings(b'~~D8{"version":"synthetic-d8"}')})
            with self.assertRaisesRegex(ValueError, "without R8 marker"):
                verify.verify_apk(path, mapping)

    def test_r8_rejects_a_valid_stale_mapping_and_conflicting_previous_markers(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            current = r8_mapping(mapping)
            stale_path = Path(root) / "stale.txt"
            previous = r8_mapping(stale_path, b"example.Previous -> b:\n")
            apk(path, dex={"classes.dex": dex_strings(r8_marker(current))})
            with self.assertRaisesRegex(ValueError, "does not match APK marker"):
                verify.verify_apk(path, stale_path)
            apk(path, dex={"classes.dex": dex_strings(r8_marker(current)),
                           "classes2.dex": dex_strings(r8_marker(previous))})
            with self.assertRaisesRegex(ValueError, "conflicting R8 markers"):
                verify.verify_apk(path, mapping)

    def test_r8_hash_covers_raw_body_comments_line_endings_and_the_last_byte(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            body = b'# {"id":"synthetic-metadata"}\nexample.Current -> a:\n'
            marker = r8_mapping(mapping, body)
            original = mapping.read_bytes()
            header = original[:-len(body)]
            apk(path, dex={"classes.dex": dex_strings(r8_marker(marker))})
            verify.verify_apk(path, mapping)
            mapping.write_bytes(header.replace(b"\n", b"\r\n") + body)
            verify.verify_apk(path, mapping)
            for changed in [body.replace(b"synthetic", b"different"), body.replace(b"\n", b"\r\n"), body[:-1]]:
                with self.subTest(body=changed):
                    mapping.write_bytes(header + changed)
                    with self.assertRaisesRegex(ValueError, "content hash does not match"):
                        verify.verify_apk(path, mapping)

    def test_r8_rejects_invalid_missing_duplicate_and_unbound_mapping_metadata(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            marker = r8_mapping(mapping)
            original = mapping.read_bytes()
            apk(path, dex={"classes.dex": dex_strings(r8_marker(marker))})
            id_line = f'# pg_map_id: {marker["pg-map-id"]}\n'.encode()
            hash_line = next(line for line in original.splitlines(keepends=True) if line.startswith(b"# pg_map_hash:"))
            for changed in [original.replace(b"# compiler: R8\n", b""),
                            original.replace(b"# compiler: R8", b"# compiler: D8"),
                            original.replace(hash_line, b""),
                            original.replace(b"SHA-256", b"SHA-1"),
                            original.replace(id_line, id_line * 2),
                            original.replace(b"# min_api: 29", b"# min_api: invalid"),
                            original + id_line]:
                with self.subTest(metadata=changed[:90]):
                    mapping.write_bytes(changed)
                    with self.assertRaisesRegex(ValueError, "mapping.*metadata"):
                        verify.verify_apk(path, mapping)
            wrong_id = "0000000" if marker["pg-map-id"] != "0000000" else "1111111"
            mapping.write_bytes(original.replace(id_line, f"# pg_map_id: {wrong_id}\n".encode()))
            with self.assertRaisesRegex(ValueError, "ID does not match its content hash"):
                verify.verify_apk(path, mapping)

    def test_r8_compiler_and_min_api_must_match_the_artifact(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            marker = r8_mapping(mapping)
            for update in [{"version": "another-r8"}, {"min-api": 30}]:
                with self.subTest(marker=update):
                    apk(path, dex={"classes.dex": dex_strings(r8_marker({**marker, **update}))})
                    with self.assertRaisesRegex(ValueError, "does not match APK marker"):
                        verify.verify_apk(path, mapping)

    def test_r8_rejects_malformed_marker_json_fields_and_duplicate_keys(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            marker = r8_mapping(mapping)
            invalid = [b"~~R8{broken}", b"~~R8[]", r8_marker(marker)[:-1] + b',"pg-map-id":"aaaaaaa"}',
                       r8_marker({key: value for key, value in marker.items() if key != "pg-map-id"})]
            invalid.extend(r8_marker({**marker, **update}) for update in
                           [{"min-api": True}, {"backend": "cf"}, {"version": ""}, {"pg-map-id": "not-a-hash"}])
            for value in invalid:
                with self.subTest(marker=value):
                    apk(path, dex={"classes.dex": dex_strings(value)})
                    with self.assertRaisesRegex(ValueError, "Malformed R8 DEX marker"):
                        verify.verify_apk(path, mapping)

    def test_r8_rejects_invalid_dex_string_table_offsets_and_terminators(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            marker = r8_mapping(mapping)
            original = dex_strings(r8_marker(marker))
            bad_table = bytearray(original)
            struct.pack_into("<I", bad_table, 56, 0xffffffff)
            bad_offset = bytearray(original)
            struct.pack_into("<I", bad_offset, 112, len(original))
            for value in [bytes(bad_table), bytes(bad_offset), original[:-1] + b"x"]:
                with self.subTest(dex_size=len(value)):
                    apk(path, dex={"classes.dex": value})
                    with self.assertRaisesRegex(ValueError, "DEX string"):
                        verify.verify_apk(path, mapping)

    def test_r8_rejects_obfuscated_original_classes_facades_and_inner_classes(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            for name in ["DashScopeEngine", "CloudProtocolsKt", "OpenAITranslationClient$check$1"]:
                with self.subTest(original=name):
                    body = f"app.yuxino.mimi.android.provider.{name} -> a:\n".encode()
                    marker = r8_mapping(mapping, body)
                    apk(path, dex={"classes.dex": dex_strings(r8_marker(marker)), "classes2.dex": dex_strings(b"La;")})
                    with self.assertRaisesRegex(ValueError, "offline legacy runtime in R8 mapping"):
                        verify.verify_apk(path, mapping)

    def test_r8_rejects_inline_only_methods_and_relocated_field_owners(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            for member in ["1:1:void app.yuxino.mimi.android.provider.DashScopeEngine.run():12:12 -> a",
                           "1:void app.yuxino.mimi.android.provider.DashScopeEngine.<init>(java.lang.String[]):12 -> a",
                           "int app.yuxino.mimi.android.provider.CloudProtocolsKt.cached -> b"]:
                with self.subTest(member=member):
                    marker = r8_mapping(mapping, ("example.Current -> a:\n    " + member + "\n").encode())
                    apk(path, dex={"classes.dex": dex_strings(r8_marker(marker), b"La;")})
                    with self.assertRaisesRegex(ValueError, "offline legacy runtime in R8 mapping"):
                        verify.verify_apk(path, mapping)

    def test_r8_allows_similar_class_names_and_rejects_unknown_mapping_records(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            body = (b"app.yuxino.mimi.android.provider.DashScopeEngineAdapter -> a:\n"
                    b"    void app.yuxino.mimi.android.provider.TranslationClientFactory.run() -> a\n"
                    b"    int other.package.DashScopeEngine.cached -> b\n")
            marker = r8_mapping(mapping, body)
            apk(path, dex={"classes.dex": dex_strings(r8_marker(marker), b"La;")})
            verify.verify_apk(path, mapping)
            marker = r8_mapping(mapping, body + b"    unrecognized member record\n")
            apk(path, dex={"classes.dex": dex_strings(r8_marker(marker))})
            with self.assertRaisesRegex(ValueError, "mapping class or member"):
                verify.verify_apk(path, mapping)

    def test_r8_mapping_never_bypasses_the_direct_dex_or_native_guards(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            marker = r8_mapping(mapping)
            entries = {"classes.dex": dex_strings(r8_marker(marker))}
            apk(path, dex={**entries, "classes2.dex": dex_strings(b"Lapp/yuxino/mimi/android/provider/DashScopeEngine;")})
            with self.assertRaisesRegex(ValueError, "offline legacy runtime in classes2.dex"):
                verify.verify_apk(path, mapping)
            for options, error in [({"abis": ["arm64-v8a"]}, "each supported ABI"),
                                   ({"compressed": True}, "uncompressed"),
                                   ({"aligned": False}, "ZIP-aligned"),
                                   ({"notices": None}, "license notices")]:
                with self.subTest(options=options):
                    apk(path, dex=entries, **options)
                    with self.assertRaisesRegex(ValueError, error):
                        verify.verify_apk(path, mapping)

    def test_r8_cli_requires_and_passes_the_mapping_argument(self):
        with tempfile.TemporaryDirectory() as root:
            path, mapping = Path(root) / "app.apk", Path(root) / "mapping.txt"
            marker = r8_mapping(mapping)
            apk(path, dex={"classes.dex": dex_strings(r8_marker(marker))})
            arguments = ["verify-shared-core.py", "--apk", str(path)]
            with mock.patch.object(sys, "argv", arguments), mock.patch("sys.stderr"):
                with self.assertRaises(SystemExit) as result:
                    verify.main()
                self.assertEqual(result.exception.code, 1)
            with mock.patch.object(sys, "argv", arguments + ["--mapping", str(mapping)]), mock.patch("sys.stdout"):
                verify.main()

    def test_mapping_argument_requires_an_apk(self):
        with mock.patch.object(sys, "argv", ["verify-shared-core.py", "--jni-libs", "jni", "--mapping", "map.txt"]), mock.patch("sys.stderr"):
            with self.assertRaises(SystemExit) as result:
                verify.main()
            self.assertEqual(result.exception.code, 2)

    def test_apk_rejects_missing_duplicate_compressed_and_misaligned_libraries(self):
        cases = [({"abis": ["arm64-v8a"]}, "each supported ABI"),
                 ({"duplicate": True}, "each supported ABI"),
                 ({"compressed": True}, "uncompressed"),
                 ({"aligned": False}, "ZIP-aligned")]
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "app.apk"
            for options, expected in cases:
                apk(path, **options)
                with self.assertRaisesRegex(ValueError, expected):
                    verify.verify_apk(path)

    def test_apk_requires_packaged_native_dependency_notices(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "app.apk"
            for notices in [None, b" \n"]:
                apk(path, notices=notices)
                with self.assertRaisesRegex(ValueError, "license notices"):
                    verify.verify_apk(path)


if __name__ == "__main__":
    unittest.main()
