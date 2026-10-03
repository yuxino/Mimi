#!/usr/bin/env python3
import importlib.util
import struct
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from subprocess import CompletedProcess

spec = importlib.util.spec_from_file_location("gles", Path(__file__).with_name("check-appimage-gles.py"))
gles = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gles)

GLVND_DYNAMIC = """
 0x0000000000000001 (NEEDED)             Shared library: [libGLdispatch.so.0]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x000000000000000e (SONAME)             Library soname: [libGLESv2.so.2]
"""


class GlesTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        self.library = self.root / "usr/lib/libGLESv2.so.2"
        self.library.parent.mkdir(parents=True)
        header = bytearray(64)
        header[:7] = b"\x7fELF\x02\x01\x01"
        struct.pack_into("<HH", header, 16, 3, 62)
        self.library.write_bytes(header)
        self.library.chmod(0o644)
        self.copyright = self.root / "usr/share/doc/mimi/libgles2-copyright"
        self.copyright.parent.mkdir(parents=True)
        self.copyright.write_text("GLVND copyright notice fixture")
        # readelf is Linux tooling; the same artifact-policy tests also run on
        # macOS. Final Linux bundle verification runs the real readelf command.
        self.readelf = patch.object(gles.subprocess, "run", return_value=CompletedProcess(
            [], 0, stdout=GLVND_DYNAMIC, stderr="",
        )).start()
        self.addCleanup(patch.stopall)

    def test_vendor_neutral_library_is_accepted(self):
        gles.check(self.root)
        self.assertEqual(self.readelf.call_args.args[0], ["readelf", "--dynamic", str(self.library)])

    def test_missing_library_is_rejected_even_if_the_host_has_gles(self):
        self.library.unlink()
        with self.assertRaises(FileNotFoundError):
            gles.check(self.root)
        self.readelf.assert_not_called()

    def test_symlink_must_stay_in_the_bundle(self):
        with tempfile.TemporaryDirectory() as host_directory:
            host_library = Path(host_directory) / "libGLESv2.so.2"
            self.library.rename(host_library)
            self.library.symlink_to(host_library)
            with self.assertRaisesRegex(ValueError, "inside the AppImage"):
                gles.check(self.root)

    def test_relative_in_bundle_symlink_is_accepted(self):
        self.library.rename(self.library.with_name("libGLESv2.so.2.1.0"))
        self.library.symlink_to("libGLESv2.so.2.1.0")
        gles.check(self.root)

    def test_wrong_architecture_and_unreadable_library_are_rejected(self):
        self.library.chmod(0o640)
        with self.assertRaisesRegex(ValueError, "readable"):
            gles.check(self.root)
        self.library.chmod(0o644)
        header = bytearray(self.library.read_bytes())
        struct.pack_into("<H", header, 18, 183)  # AArch64
        self.library.write_bytes(header)
        with self.assertRaisesRegex(ValueError, "x86_64"):
            gles.check(self.root)

    def test_vendor_library_or_wrong_soname_is_rejected(self):
        for dynamic in (
            GLVND_DYNAMIC.replace("libGLdispatch.so.0", "libglapi.so.0"),
            GLVND_DYNAMIC.replace("libGLESv2.so.2", "libGLESv1_CM.so.1"),
            GLVND_DYNAMIC + "\n(NEEDED) Shared library: [libdrm.so.2]\n",
        ):
            with self.subTest(dynamic=dynamic):
                self.readelf.return_value.stdout = dynamic
                with self.assertRaisesRegex(ValueError, "GLVND"):
                    gles.check(self.root)

    def test_bundled_host_graphics_stack_is_rejected(self):
        for filename in ("libGLdispatch.so.0", "libEGL_mesa.so.0", "swrast_dri.so"):
            with self.subTest(filename=filename):
                driver = self.library.with_name(filename)
                driver.touch()
                with self.assertRaisesRegex(ValueError, "must not be bundled"):
                    gles.check(self.root)
                driver.unlink()

    def test_missing_copyright_is_rejected(self):
        self.copyright.unlink()
        with self.assertRaisesRegex(ValueError, "copyright"):
            gles.check(self.root)


if __name__ == "__main__":
    unittest.main()
