#!/usr/bin/env python3
"""Regression tests for native ABI, toolchain pinning and packaging boundaries."""
import importlib.util
from pathlib import Path
from unittest import mock
import struct
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


def apk(path, abis=None, aligned=True, compressed=False, duplicate=False,
        notices=verify.NOTICE_SOURCE.read_bytes()):
    with zipfile.ZipFile(path, "w") as archive:
        if notices is not None:
            archive.writestr(verify.NOTICES, notices)
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

    def test_dependency_notices_match_the_reviewed_lockfile_inventory(self):
        notices = verify.NOTICE_SOURCE.read_bytes()
        verify.verify_license_notices(notices)
        with self.assertRaisesRegex(ValueError, "reviewed inventory"):
            verify.verify_license_notices(b"License notices")
        with tempfile.TemporaryDirectory() as root:
            old_lock = verify.LOCKFILE
            try:
                path = Path(root) / "Cargo.lock"
                path.write_bytes(old_lock.read_bytes() + b"\n# dependency graph changed\n")
                verify.LOCKFILE = path
                with self.assertRaisesRegex(ValueError, "stale for Cargo.lock"):
                    verify.verify_license_notices(notices)
            finally:
                verify.LOCKFILE = old_lock

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
