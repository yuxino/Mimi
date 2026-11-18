#!/usr/bin/env python3
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("permissions", Path(__file__).with_name("check-appimage-permissions.py"))
permissions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(permissions)


class PermissionsTest(unittest.TestCase):
    def test_final_launcher_modes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("AppRun", "AppRun.wrapped", "usr/bin/mimi"):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("launcher")
                path.chmod(0o755)
            permissions.check(root)
            wrapped = root / "AppRun.wrapped"
            wrapped.chmod(0o770)
            with self.assertRaisesRegex(ValueError, "AppRun.wrapped"):
                permissions.check(root)
            wrapped.chmod(0o755)
            wrapped.unlink()
            wrapped.symlink_to("usr/bin/mimi")
            permissions.check(root)
            (root / "usr/bin/mimi").chmod(0o750)
            with self.assertRaises(ValueError):
                permissions.check(root)


if __name__ == "__main__":
    unittest.main()
