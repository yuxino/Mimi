#!/usr/bin/env python3
"""Reject host graphics libraries from the AppImage.

WebKit loads GLES at runtime, outside linuxdeploy's ordinary linked-library
scan. A graphics library copied from the build host would shadow the host's
dispatcher and drivers, so the AppImage follows the AppImage excludelist and
leaves the entire EGL/GLVND stack, including libGLESv2.so.2, to the host.
"""
import sys
from pathlib import Path

GRAPHICS_LIBRARIES = (
    "libGL*.so*",
    "libEGL*.so*",
    "libOpenGL*.so*",
    "libglapi.so*",
    "libgbm*.so*",
    "libdrm*.so*",
    "libnvidia*.so*",
    "*_dri.so",
)


def check(appdir):
    root = Path(appdir).resolve(strict=True)
    for pattern in GRAPHICS_LIBRARIES:
        bundled = next(root.rglob(pattern), None)
        if bundled is not None:
            raise ValueError(
                f"Host graphics libraries must not be bundled: {bundled.relative_to(root)}"
            )
    copyright_file = root / "usr/share/doc/mimi/libgles2-copyright"
    if copyright_file.exists():
        raise ValueError("The libgles2 copyright notice must not be bundled")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: check-appimage-gles.py <AppDir>")
    check(sys.argv[1])
