#!/usr/bin/env python3
import hashlib
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("build-linux-packages.sh")
EXPECTED_SHA256 = "f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f"


class BuildLinuxPackagesTests(unittest.TestCase):
    def run_script(self, root: Path, *, cached_content=None, download_content=b"valid-apprun",
                   download_sha=EXPECTED_SHA256):
        cache_home = root / "cache"
        bin_dir = root / "bin"
        bin_dir.mkdir(parents=True, exist_ok=True)
        npm_log = root / "npm.log"
        curl_log = root / "curl.log"

        launcher = cache_home / "tauri" / "AppRun-x86_64"
        if cached_content is not None:
            launcher.parent.mkdir(parents=True, exist_ok=True)
            launcher.write_bytes(cached_content)
            launcher.chmod(0o600)

        payload_file = root / "payload.bin"
        payload_file.write_bytes(download_content)

        # Stub uname to report Linux x86_64 so this test runs on macOS and Linux alike.
        uname = bin_dir / "uname"
        uname.write_text(
            "#!/usr/bin/env bash\n"
            'if [[ "${1:-}" == "-s" ]]; then echo Linux\n'
            'elif [[ "${1:-}" == "-m" ]]; then echo x86_64\n'
            'else /usr/bin/uname "$@"; fi\n',
            encoding="utf-8",
        )
        uname.chmod(0o755)

        # Stub sha256sum so the fixture payload maps to the pinned digest without storing a real binary.
        valid_digest = hashlib.sha256(download_content).hexdigest()
        sha256sum = bin_dir / "sha256sum"
        sha256sum.write_text(
            "#!/usr/bin/env python3\n"
            "import hashlib, sys\n"
            f"target_real = {valid_digest!r}\n"
            f"mapped = {download_sha!r}\n"
            "path = sys.argv[1]\n"
            "with open(path, 'rb') as fh:\n"
            "    digest = hashlib.sha256(fh.read()).hexdigest()\n"
            "if digest == target_real:\n"
            "    digest = mapped\n"
            "print(f'{digest}  {path}')\n",
            encoding="utf-8",
        )
        sha256sum.chmod(0o755)

        curl = bin_dir / "curl"
        curl.write_text(
            "#!/usr/bin/env bash\n"
            "set -euo pipefail\n"
            f'echo "$*" >> {str(curl_log)!r}\n'
            'out=""\n'
            'while (( $# > 0 )); do\n'
            '  if [[ "$1" == "--output" ]]; then out="$2"; shift 2; else shift; fi\n'
            'done\n'
            f'cp {str(payload_file)!r} "$out"\n',
            encoding="utf-8",
        )
        curl.chmod(0o755)

        npm = bin_dir / "npm"
        npm.write_text(
            "#!/usr/bin/env bash\n"
            f'echo "$*" >> {str(npm_log)!r}\n',
            encoding="utf-8",
        )
        npm.chmod(0o755)

        env = os.environ.copy()
        env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"
        env["XDG_CACHE_HOME"] = str(cache_home)

        result = subprocess.run(
            ["bash", str(SCRIPT), "--config", "src-tauri/config/ci.json"],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        return result, launcher, curl_log, npm_log

    def test_valid_fresh_download_is_verified_and_chmod_755(self):
        with tempfile.TemporaryDirectory() as root:
            result, launcher, curl_log, npm_log = self.run_script(Path(root))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(launcher.is_file())
            self.assertEqual(stat.S_IMODE(launcher.stat().st_mode), 0o755)
            self.assertTrue(curl_log.is_file())
            self.assertIn("run tauri -- build --config src-tauri/config/ci.json -- --locked",
                          npm_log.read_text(encoding="utf-8"))

    def test_valid_cache_skips_download_and_sets_755(self):
        with tempfile.TemporaryDirectory() as root:
            result, launcher, curl_log, npm_log = self.run_script(
                Path(root), cached_content=b"valid-apprun"
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(curl_log.exists())
            self.assertEqual(stat.S_IMODE(launcher.stat().st_mode), 0o755)
            self.assertTrue(npm_log.is_file())

    def test_corrupt_cache_redownloads_once_and_recovers(self):
        with tempfile.TemporaryDirectory() as root:
            result, launcher, curl_log, npm_log = self.run_script(
                Path(root), cached_content=b"corrupted-cache"
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("checksum mismatch", result.stderr)
            self.assertEqual(len(curl_log.read_text(encoding="utf-8").splitlines()), 1)
            self.assertEqual(launcher.read_bytes(), b"valid-apprun")
            self.assertEqual(stat.S_IMODE(launcher.stat().st_mode), 0o755)
            self.assertTrue(npm_log.is_file())

    def test_corrupt_download_fails_closed_without_running_build(self):
        with tempfile.TemporaryDirectory() as root:
            result, launcher, curl_log, npm_log = self.run_script(
                Path(root),
                cached_content=b"corrupted-cache",
                download_content=b"still-bad",
                download_sha="0" * 64,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("SHA-256 mismatch", result.stderr)
            self.assertFalse(launcher.exists())
            self.assertFalse(npm_log.exists())
            self.assertEqual(len(curl_log.read_text(encoding="utf-8").splitlines()), 1)


if __name__ == "__main__":
    unittest.main()
