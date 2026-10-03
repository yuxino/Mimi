#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest
spec = importlib.util.spec_from_file_location("ci_scope", Path(__file__).with_name("ci-scope.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ScopeTests(unittest.TestCase):
    def test_copy_changes_do_not_build_native_packages(self):
        result = module.plan(["src/lib/i18n.ts", "docs/qa.md"])
        self.assertFalse(result["native"])
        self.assertFalse(result["bundle"])
        self.assertFalse(result["arm_bundle"])

    def test_windows_capture_still_tests_both_architectures(self):
        result = module.plan(["src-tauri/src/audio/windows.rs"])
        self.assertEqual(result["rust_os"], ["windows-2025"])
        self.assertTrue(result["windows_arm"])
        self.assertFalse(result["arm_bundle"])

    def test_shared_native_contract_tests_every_platform_without_packages(self):
        for path in ["src-tauri/src/settings_store.rs", "src-tauri/Cargo.lock", ".github/workflows/ci.yml", "scripts/ci-scope.py",
                     "shared/translation-contracts.json",
                     "shared/mimi-core/src/subtitle_reducer.rs",
                     "shared/mimi-android-jni/src/lib.rs",
                     "scripts/build-shared-core.py",
                     "android/app/src/main/java/app/yuxino/mimi/android/provider/DeepLTranslationClient.kt"]:
            result = module.plan([path])
            self.assertEqual(result["rust_os"], module.ALL_OS)
            self.assertFalse(result["bundle"])

    def test_manual_linux_package_does_not_build_windows_or_mac(self):
        result = module.plan([], package="linux")
        self.assertEqual(result["bundle_os"], ["ubuntu-22.04"])
        self.assertEqual(result["rust_os"], ["ubuntu-22.04"])
        self.assertFalse(result["windows_arm"])

    def test_full_validation_is_available_without_automatic_packages(self):
        result = module.plan([], full=True)
        self.assertEqual(result["rust_os"], module.ALL_OS)
        self.assertFalse(result["bundle"])

    def test_release_always_checks_all_platforms_and_arm_launch(self):
        result = module.plan([], release=True)
        self.assertEqual(result["rust_os"], module.ALL_OS)
        self.assertTrue(result["arm_bundle"])
        self.assertFalse(result["bundle"])  # Release-only jobs own signed packages.

    def test_bad_diff_expands_checks_and_invalid_package_fails_closed(self):
        self.assertEqual(module.plan(None)["rust_os"], module.ALL_OS)
        with self.assertRaises(ValueError):
            module.plan([], package="unexpected")


if __name__ == "__main__":
    unittest.main()
