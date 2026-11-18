#!/usr/bin/env python3
"""Regression coverage for the AppImage host-graphics boundary."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "appimage_graphics", Path(__file__).with_name("check-appimage-gles.py")
)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class HostGraphicsTests(unittest.TestCase):
    def test_normal_runtime_libraries_are_allowed(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            libraries = root / "usr/lib"
            libraries.mkdir(parents=True)
            for name in ("libwebkit2gtk-4.1.so.0", "libpulse.so.0", "libmimi.so"):
                (libraries / name).touch()
            module.check(root)

    def test_graphics_libraries_are_rejected_in_nested_directories(self):
        for name in (
            "libGL.so.1", "libEGL.so.1", "libGLX.so.0", "libGLdispatch.so.0",
            "libOpenGL.so.0", "libGLESv1_CM.so.1", "libGLESv2.so.2",
            "libglapi.so.0", "libgbm.so.1", "libdrm.so.2",
            "libnvidia-eglcore.so.1", "iris_dri.so",
        ):
            with self.subTest(library=name), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                libraries = root / "usr/lib/x86_64-linux-gnu/dri"
                libraries.mkdir(parents=True)
                (libraries / name).touch()
                with self.assertRaisesRegex(ValueError, "must not be bundled"):
                    module.check(root)

    def test_graphics_symlinks_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "libGLESv2.so.2").symlink_to("missing-host-library")
            with self.assertRaisesRegex(ValueError, "must not be bundled"):
                module.check(root)

    def test_legacy_bundled_copyright_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            notice = root / "usr/share/doc/mimi/libgles2-copyright"
            notice.parent.mkdir(parents=True)
            notice.touch()
            with self.assertRaisesRegex(ValueError, "copyright notice"):
                module.check(root)


if __name__ == "__main__":
    unittest.main()
