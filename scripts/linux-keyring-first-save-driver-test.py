#!/usr/bin/env python3
"""Exercise the native-prompt driver with deterministic window teardown races."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
DRIVER = ROOT / "scripts/linux-keyring-first-save-smoke.sh"
STUB = r'''#!/usr/bin/env python3
import os
from pathlib import Path
import sys
import time

root = Path(os.environ["MIMI_TEST_PRIVATE_KEYRING_DIRECTORY"])
name = Path(sys.argv[0]).name
scenario = os.environ["PROMPT_SCENARIO"]
def wait_for(path):
    deadline = time.monotonic() + 3
    while not path.exists():
        if time.monotonic() >= deadline:
            print("prompt driver did not acknowledge closure", file=sys.stderr)
            sys.exit(1)
        time.sleep(.002)

if name == "uname":
    print("Linux")
elif name == "gdbus":
    print("(true,)")
elif name == "sleep":
    time.sleep(.002)
elif name in ("openbox", "gnome-keyring-daemon"):
    time.sleep(60)
elif name == "timeout":
    os.execvp(sys.argv[2], sys.argv[2:])
elif name == "cargo":
    (root / "visible").write_text("11")
    (root / "first-save-phase").write_text("cancel")
    wait_for(root / "escape")
    if scenario == "exit-during-close":
        sys.exit(23)
    if scenario == "stuck-window":
        time.sleep(60)
        sys.exit(24)
    if scenario == "delayed-close":
        # D-Bus has already completed, but GTK has not destroyed its window.
        time.sleep(.15)
    (root / "visible").unlink()
    wait_for(root / "first-save-retry-ready")
    (root / "visible").write_text("22")
    (root / "first-save-phase").write_text("accept")
    wait_for(root / "accepted")
    (root / "visible").unlink()
elif name == "xdotool":
    action = sys.argv[1]
    visible = (root / "visible")
    if action == "search":
        if not visible.exists():
            sys.exit(1)
        print(visible.read_text())
    elif action == "key" and sys.argv[-1] == "Escape":
        (root / "escape").touch()
    elif action == "type" or (action == "key" and sys.argv[-1] == "Return"):
        if not visible.exists() or visible.read_text() != "22":
            print("input reached the cancelled prompt", file=sys.stderr)
            sys.exit(97)
        if action == "type":
            with (root / "typed").open("a") as stream:
                stream.write("password field\n")
        else:
            (root / "accepted").touch()
'''


class FirstSaveDriverTests(unittest.TestCase):
    def run_scenario(self, scenario):
        with tempfile.TemporaryDirectory(prefix="mimi-prompt-driver-") as directory:
            root = Path(directory)
            binaries = root / "bin"
            binaries.mkdir()
            for name in ("uname", "gdbus", "sleep", "openbox", "gnome-keyring-daemon",
                         "timeout", "cargo", "xdotool"):
                executable = binaries / name
                executable.write_text(STUB)
                executable.chmod(0o700)
            env = dict(os.environ, PATH=f"{binaries}{os.pathsep}{os.environ['PATH']}",
                       MIMI_LINUX_FIRST_SAVE_SESSION="1",
                       MIMI_TEST_PRIVATE_KEYRING_DIRECTORY=str(root),
                       PROMPT_SCENARIO=scenario)
            result = subprocess.run(["bash", str(DRIVER)], cwd=ROOT, env=env,
                                    text=True, capture_output=True, timeout=30)
            typed = (root / "typed").read_text() if (root / "typed").exists() else ""
            return result, typed

    def test_accepts_only_the_second_prompt(self):
        for scenario in ("immediate-close", "delayed-close"):
            with self.subTest(scenario=scenario):
                result, typed = self.run_scenario(scenario)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(typed, "password field\npassword field\n")

    def test_refuses_to_type_if_cancelled_window_stays_visible(self):
        result, typed = self.run_scenario("stuck-window")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cancelled keyring prompt did not close.", result.stderr)
        self.assertEqual(typed, "")

    def test_reports_test_exit_while_dismissing(self):
        result, typed = self.run_scenario("exit-during-close")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exited while dismissing", result.stderr)
        self.assertEqual(typed, "")


if __name__ == "__main__":
    unittest.main()
