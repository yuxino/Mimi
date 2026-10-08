#!/usr/bin/env python3
"""Compile/run the production placement logic without an Android SDK or device."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
PACKAGE = Path("app/yuxino/mimi/android/capture")
SOURCES = [
    ROOT / "android/app/src/main/java" / PACKAGE / "OverlayPlacement.java",
    ROOT / "android/app/src/test/java" / PACKAGE / "OverlayPlacementRegression.java",
]

if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="mimi-overlay-placement-") as output:
        # Some cloud JREs include jdk.compiler but omit the javac executable.
        subprocess.run(["java", "com.sun.tools.javac.Main", "-Xlint:all", "-Werror",
                        "-d", output, *map(str, SOURCES)], check=True)
        subprocess.run(["java", "-cp", output,
                        "app.yuxino.mimi.android.capture.OverlayPlacementRegression"], check=True)
