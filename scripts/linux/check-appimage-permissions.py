#!/usr/bin/env python3
"""Check the extracted final AppImage, independent of the build user's UID."""
import stat
import sys
from pathlib import Path


def check(appdir):
    for name in ("AppRun", "AppRun.wrapped", "usr/bin/mimi"):
        path = Path(appdir) / name
        mode = path.stat().st_mode
        if not stat.S_ISREG(mode) or mode & 0o555 != 0o555:
            raise ValueError(f"{name} must be readable and executable by every user; mode={mode & 0o777:o}")


if __name__ == "__main__":
    check(sys.argv[1])
