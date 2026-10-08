#!/usr/bin/env python3
"""Exercise release build ordering and failures without signing or packaging."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent


class ReleasePreparationTests(unittest.TestCase):
    def prepare(self, failure=""):
        with tempfile.TemporaryDirectory(prefix="mimi release test ") as temporary:
            root = Path(temporary)
            scripts = root / "scripts"
            scripts.mkdir()
            (root / "src-tauri").mkdir()
            (root / "src-tauri/Info.plist").write_text(
                '<?xml version="1.0"?><plist version="1.0"><dict/></plist>'
            )
            for name in ("prepare-macos-release.sh", "extract-macos-updater.py"):
                shutil.copy2(SCRIPTS / name, scripts / name)
            (scripts / "macos-release-identity.txt").write_text("A" * 40)
            for name in ("verify-macos-release.sh", "verify-macos-release-source.sh"):
                path = scripts / name
                path.write_text("#!/usr/bin/env bash\nexit 0\n")
                path.chmod(0o755)
            binary = root / "bin"
            binary.mkdir()
            for name, content in {
                "uname": '[[ "${1:-}" == -m ]] && echo arm64 || echo Darwin',
                "git": '[[ "$1" == rev-parse ]] && echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; exit 0',
            }.items():
                path = binary / name
                path.write_text("#!/usr/bin/env bash\n" + content + "\n")
                path.chmod(0o755)
            npm = binary / "npm"
            npm.write_text('''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
root = pathlib.Path.cwd()
args = sys.argv[1:]
if args == ["-p", 'require("./package.json").version']:
    print("1.2.3")
    sys.exit(0)
with (root / "calls.jsonl").open("a") as output:
    output.write(json.dumps(args) + "\\n")
if args == ["run", "build"]:
    assert os.environ["TAURI_ENV_PLATFORM"] == "darwin"
    if os.environ["TEST_FAILURE"] == "frontend":
        sys.exit(23)
    (root / "dist").mkdir()
    sys.exit(0)
assert args[:4] == ["run", "tauri", "--", "build"]
assert (root / "dist").is_dir()
config = json.loads(pathlib.Path(args[args.index("--config") + 1]).read_text())
assert config["build"]["beforeBuildCommand"] == ""
assert config["bundle"]["createUpdaterArtifacts"] is False
if os.environ["TEST_FAILURE"] == "native":
    sys.exit(24)
intel = "--target" in args
bundle = pathlib.Path(os.environ["CARGO_TARGET_DIR"])
if intel:
    bundle /= "x86_64-apple-darwin"
bundle /= "release/bundle"
app = bundle / "macos/mimi.app/Contents"
app.mkdir(parents=True)
shutil.copy2(config["bundle"]["macOS"]["infoPlist"], app / "Info.plist")
(bundle / "dmg").mkdir()
(bundle / "dmg" / ("mimi_1.2.3_x64.dmg" if intel else "mimi_1.2.3_aarch64.dmg")).touch()
''')
            npm.chmod(0o755)
            # Version lookup uses Node rather than npm.
            shutil.copy2(npm, binary / "node")
            env = dict(os.environ, PATH=str(binary) + os.pathsep + os.environ["PATH"],
                       CARGO_TARGET_DIR=str(root / "output"), TEST_FAILURE=failure,
                       TMPDIR=str(root))
            result = subprocess.run(["bash", str(scripts / "prepare-macos-release.sh")],
                                    cwd=root, env=env, capture_output=True, text=True, timeout=30)
            calls = [json.loads(line) for line in (root / "calls.jsonl").read_text().splitlines()]
            archives = list((root / "output").rglob("*.tar.gz"))
            return result, calls, len(archives)

    def test_builds_one_frontend_before_both_signed_architectures(self):
        result, calls, archives = self.prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[0], ["run", "build"])
        self.assertEqual(len(calls), 3)
        self.assertNotIn("--target", calls[1])
        self.assertIn("x86_64-apple-darwin", calls[2])
        self.assertEqual(archives, 2)

    def test_frontend_failure_prevents_any_native_build(self):
        result, calls, archives = self.prepare("frontend")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [["run", "build"]])
        self.assertEqual(archives, 0)

    def test_first_native_failure_prevents_second_architecture(self):
        result, calls, archives = self.prepare("native")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 2)
        self.assertEqual(archives, 0)


if __name__ == "__main__":
    unittest.main()
