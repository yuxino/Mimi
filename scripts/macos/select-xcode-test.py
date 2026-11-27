#!/usr/bin/env python3
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("select-xcode.sh")


def make_xcode_bundle(apps_dir: Path, name: str, version: str) -> Path:
    app = apps_dir / name
    dev_dir = app / "Contents" / "Developer"
    dev_dir.mkdir(parents=True)
    with (app / "Contents" / "Info.plist").open("wb") as handle:
        plistlib.dump({"CFBundleShortVersionString": version}, handle)
    return dev_dir


def make_stub_bin(bin_dir: Path, default_dev: str, versions: dict[str, str]) -> None:
    bin_dir.mkdir(parents=True)
    xcode_select = bin_dir / "xcode-select"
    xcode_select.write_text(f"#!/usr/bin/env bash\nprintf '%s\\n' {default_dev!r}\n", encoding="utf-8")
    xcode_select.chmod(0o755)

    lines = ["#!/usr/bin/env bash", 'case "${DEVELOPER_DIR:-}" in']
    for dev_dir, version in versions.items():
        lines.append(f'  {dev_dir!r}) printf "Xcode %s\\nBuild version TEST\\n" {version!r} ;;')
    lines.append("  *) exit 1 ;;")
    lines.append("esac")
    xcodebuild = bin_dir / "xcodebuild"
    xcodebuild.write_text("\n".join(lines) + "\n", encoding="utf-8")
    xcodebuild.chmod(0o755)


class SelectXcodeTests(unittest.TestCase):
    def run_selector(self, apps_dir: Path, bin_dir: Path, github_env: Path):
        env = os.environ.copy()
        env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"
        env["MIMI_XCODE_APPLICATIONS_DIR"] = str(apps_dir)
        env["GITHUB_ENV"] = str(github_env)
        return subprocess.run(
            ["bash", str(SCRIPT)],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_keeps_default_when_major_is_at_least_26(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            apps = root_path / "Applications"
            default_dev = make_xcode_bundle(apps, "Xcode.app", "26.6")
            make_xcode_bundle(apps, "Xcode_26.3.app", "26.3")
            bin_dir = root_path / "bin"
            make_stub_bin(bin_dir, str(default_dev), {str(default_dev): "26.6"})
            github_env = root_path / "github_env"

            result = self.run_selector(apps, bin_dir, github_env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(github_env.read_text(encoding="utf-8").strip(), f"DEVELOPER_DIR={default_dev}")

    def test_selects_highest_stable_xcode_26_plus_when_default_is_older(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            apps = root_path / "Applications"
            old_dev = make_xcode_bundle(apps, "Xcode_16.4.app", "16.4")
            dev_26_3 = make_xcode_bundle(apps, "Xcode_26.3.app", "26.3")
            dev_26_10 = make_xcode_bundle(apps, "Xcode_26.10.app", "26.10")
            dev_beta = make_xcode_bundle(apps, "Xcode_26.11_beta.app", "26.11")
            bin_dir = root_path / "bin"
            make_stub_bin(
                bin_dir,
                str(old_dev),
                {
                    str(old_dev): "16.4",
                    str(dev_26_3): "26.3",
                    str(dev_26_10): "26.10",
                    str(dev_beta): "26.11",
                },
            )
            github_env = root_path / "github_env"

            result = self.run_selector(apps, bin_dir, github_env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(github_env.read_text(encoding="utf-8").strip(), f"DEVELOPER_DIR={dev_26_10}")

    def test_falls_back_to_beta_when_only_beta_26_plus_is_installed(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            apps = root_path / "Applications"
            dev_beta = make_xcode_bundle(apps, "Xcode_26.5_beta.app", "26.5")
            bin_dir = root_path / "bin"
            make_stub_bin(bin_dir, "/Library/Developer/CommandLineTools", {str(dev_beta): "26.5"})
            github_env = root_path / "github_env"

            result = self.run_selector(apps, bin_dir, github_env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(github_env.read_text(encoding="utf-8").strip(), f"DEVELOPER_DIR={dev_beta}")

    def test_fails_clearly_when_no_xcode_26_plus_exists(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            apps = root_path / "Applications"
            old_dev = make_xcode_bundle(apps, "Xcode_16.4.app", "16.4")
            bin_dir = root_path / "bin"
            make_stub_bin(bin_dir, str(old_dev), {str(old_dev): "16.4"})
            github_env = root_path / "github_env"

            result = self.run_selector(apps, bin_dir, github_env)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("No Xcode >= 26 installation found", result.stderr)
            self.assertFalse(github_env.exists())


if __name__ == "__main__":
    unittest.main()
