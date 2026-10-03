#!/usr/bin/env python3
"""Check the GLES entry library without loading code from the AppImage."""
import os
import re
import stat
import struct
import subprocess
import sys
from pathlib import Path


def check_library(library):
    library = Path(library)
    mode = library.stat().st_mode
    if not stat.S_ISREG(mode) or mode & 0o444 != 0o444:
        raise ValueError("libGLESv2.so.2 must be a regular file readable by every user")
    with library.open("rb") as source:
        header = source.read(20)
    if (len(header) != 20 or header[:7] != b"\x7fELF\x02\x01\x01"
            or struct.unpack_from("<HH", header, 16) != (3, 62)):
        raise ValueError("libGLESv2.so.2 must be an x86_64 ELF shared library")
    dynamic = subprocess.run(
        ["readelf", "--dynamic", str(library)], check=True,
        capture_output=True, text=True, env={**os.environ, "LC_ALL": "C"},
    ).stdout
    sonames = re.findall(r"\(SONAME\).*\[([^\]]+)\]", dynamic)
    needed = set(re.findall(r"\(NEEDED\).*\[([^\]]+)\]", dynamic))
    # The GLVND GLES wrapper dispatches through the host library. Do not bundle
    # a driver implementation just because it has the same public SONAME.
    allowed = {"libGLdispatch.so.0", "libc.so.6", "libdl.so.2", "libpthread.so.0"}
    if sonames != ["libGLESv2.so.2"] or "libGLdispatch.so.0" not in needed or needed - allowed:
        raise ValueError("libGLESv2.so.2 must be the GLVND entry library, without vendor-driver dependencies")


def check(appdir):
    root = Path(appdir).resolve(strict=True)
    library = root / "usr/lib/libGLESv2.so.2"
    if not library.resolve(strict=True).is_relative_to(root):
        raise ValueError("libGLESv2.so.2 must resolve inside the AppImage")
    check_library(library)
    copyright_file = root / "usr/share/doc/mimi/libgles2-copyright"
    if not copyright_file.is_file() or copyright_file.stat().st_size == 0:
        raise ValueError("The bundled libgles2 copyright notice is missing")
    # These are deliberately supplied by the host, as in the AppImage
    # excludelist. A successful launch on the build host cannot prove that an
    # accidentally bundled graphics driver will work on another machine.
    driver_patterns = (
        "libGLdispatch.so*", "libEGL.so*", "libGLX.so*", "libGL.so*",
        "libEGL_*.so*", "libGLX_*.so*", "libglapi.so*", "libgbm.so*",
        "libdrm*.so*", "libnvidia*.so*", "*_dri.so",
    )
    for pattern in driver_patterns:
        if next(root.rglob(pattern), None) is not None:
            raise ValueError(f"Host graphics libraries must not be bundled: {pattern}")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--library":
        check_library(sys.argv[2])
    elif len(sys.argv) == 2:
        check(sys.argv[1])
    else:
        raise SystemExit("usage: check-appimage-gles.py <AppDir> | --library <libGLESv2.so.2>")
