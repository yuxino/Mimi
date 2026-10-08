#!/usr/bin/env python3
"""Mutate real architecture inputs to prove known forks fail the check."""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location("shared_runtime_guard", Path(__file__).with_name("check-shared-runtime.py"))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class SharedRuntimeBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for directory in ("src-tauri/src/clients", "src-tauri/src/core", "shared/mimi-runtime/src", "android/app/src/main/java"):
            shutil.copytree(guard.ROOT / directory, self.root / directory)
        for filename in ("src-tauri/src/audio/send_pipeline.rs", "src-tauri/Cargo.toml",
                         "shared/mimi-runtime/Cargo.toml", "shared/mimi-android-jni/Cargo.toml",
                         "android/app/build.gradle.kts", ".github/workflows/android.yml",
                         ".github/workflows/ci.yml", "scripts/build-shared-core.py",
                         "scripts/check-shared-core.sh", "scripts/check-shared-runtime.py"):
            target = self.root / filename
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(guard.ROOT / filename, target)

    def edit(self, filename, transform):
        path = self.root / filename
        path.write_text(transform(path.read_text(encoding="utf-8")), encoding="utf-8")

    def add(self, filename, text):
        path = self.root / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def rejected(self, message):
        self.assertTrue(any(message in error for error in guard.check(self.root)), message)

    def test_current_repository_passes(self):
        self.assertEqual(guard.check(), [])

    def test_utf8_inputs_and_mutations_work_with_a_legacy_windows_locale(self):
        original_open = Path.open

        def legacy_locale_open(path, mode="r", buffering=-1, encoding=None,
                               errors=None, newline=None):
            if "b" not in mode and encoding is None:
                encoding = "cp1252"
            return original_open(path, mode, buffering, encoding, errors, newline)

        with mock.patch.object(Path, "open", legacy_locale_open):
            self.edit("src-tauri/src/clients/high_quality_client.rs",
                      lambda text: text + "\n// 日本語の字幕・中文\n")
            self.add("android/app/src/main/java/Utf8Fixture.kt",
                     '// 日本語の字幕・中文\nclass Utf8Fixture { }\n')
            self.assertEqual(guard.check(self.root), [])
            self.edit("src-tauri/src/clients/high_quality_client.rs",
                      lambda text: text + "\nfn local_deadline() -> u64 { 60 }\n")
            self.rejected("desktop facade must only re-export shared code")

    def test_desktop_client_cannot_gain_local_business_logic(self):
        self.edit("src-tauri/src/clients/high_quality_client.rs", lambda text: text + "\nfn local_deadline() -> u64 { 60 }\n")
        self.rejected("desktop facade must only re-export shared code")

    def test_actual_native_build_entry_rejects_fork_before_compiling(self):
        self.edit("src-tauri/src/clients/high_quality_client.rs", lambda text: text + "\nfn local_deadline() -> u64 { 60 }\n")
        environment = dict(os.environ, PATH="")  # Rust cannot run even if the guard regresses.
        result = subprocess.run([sys.executable, str(self.root / "scripts/build-shared-core.py"),
                                 "--platform", "host", "--profile", "debug",
                                 "--output", str(self.root / "output")],
                                env=environment, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("desktop facade must only re-export shared code", result.stderr)
        self.assertFalse((self.root / "output").exists())

    def test_desktop_controller_cannot_be_reimplemented(self):
        self.edit("src-tauri/src/core/session.rs", lambda _: "pub struct TranslationSessionController;\n")
        self.rejected("desktop facade must only re-export shared code: core/session.rs")

    def test_desktop_facade_cannot_redirect_to_another_client(self):
        self.edit("src-tauri/src/clients/audio3_client.rs", lambda text: text.replace("audio3_client", "live_translate_client"))
        self.rejected("desktop facade must only re-export shared code")

    def test_new_desktop_client_needs_shared_implementation(self):
        self.add("src-tauri/src/clients/new_provider.rs", "pub fn translate() {}\n")
        self.rejected("desktop module has no shared implementation")

    def test_module_list_cannot_hide_an_implementation_or_path_override(self):
        self.edit("src-tauri/src/clients/mod.rs", lambda text: text + '\n#[path = "../other.rs"]\npub mod audio3_client;\n')
        # Path overrides cannot redirect an otherwise valid facade to local code.
        self.rejected("desktop module list")

    def test_core_module_list_cannot_redirect_the_shared_controller(self):
        self.edit("src-tauri/src/core/mod.rs", lambda text: text.replace("pub mod session;", '#[path = "local_session.rs"]\npub mod session;'))
        self.rejected("desktop core module list")

    def test_legacy_engine_fallback_is_rejected_even_with_import_alias(self):
        self.edit("android/app/src/main/java/app/yuxino/mimi/android/capture/MimiService.kt",
                  lambda text: text + "\nimport app.yuxino.mimi.android.provider.DashScopeEngine as Fallback\n")
        self.rejected("Android production references offline legacy runtime")

    def test_old_text_factory_cannot_return_to_production(self):
        self.edit(guard.PROVIDER + "TranslationConfiguration.kt",
                  lambda text: text + "\nfun fallback(config: TranslationConfiguration) = createTranslationClient(config)\n")
        self.rejected("Android production references offline legacy runtime")

    def test_new_kotlin_engine_in_a_flavor_is_rejected(self):
        self.add("android/app/src/demo/java/Fork.kt", "class Fork : ProviderEngine { }\n")
        self.rejected("Android production defines another provider engine")

    def test_another_engine_cannot_hide_in_the_shared_adapter_file(self):
        self.edit(guard.PROVIDER + "SharedRuntimeEngine.kt", lambda text: text + "\nclass OtherEngine : ProviderEngine { }\n")
        self.rejected("Android production defines another provider engine")

    def test_new_java_transport_cannot_bypass_rust(self):
        self.add("android/app/src/release/java/Fork.java", "class Fork { void connect() { client.newWebSocket(request, listener); } }\n")
        self.rejected("Android production creates provider transport outside Rust")

    def test_java_engine_cannot_bypass_kotlin_entry(self):
        self.add("android/app/src/release/java/Fork.java", "class Fork implements ProviderEngine { }\n")
        self.rejected("Android production defines another provider engine")

    def test_native_adapter_can_accept_the_capture_interface(self):
        self.add("android/app/src/main/java/CaptureAdapter.kt", "class CaptureAdapter(private val engine: ProviderEngine) { }\n")
        self.assertEqual(guard.check(self.root), [])

    def test_comment_string_and_offline_test_mentions_do_not_create_forks(self):
        self.edit(guard.PROVIDER + "SharedRuntimeEngine.kt",
                  lambda text: text + '\n// DashScopeEngine is historical.\nprivate val label = "newWebSocket( DashScopeEngine"\n')
        self.add("android/app/src/test/java/Fixture.kt", "class Fixture : ProviderEngine { val old = DashScopeEngine(listener) }\n")
        self.assertEqual(guard.check(self.root), [])

    def test_shared_changes_are_allowed_without_desktop_copy(self):
        self.edit("shared/mimi-runtime/src/clients/high_quality_client.rs", lambda text: text + "\nfn changed_shared_policy() {}\n")
        self.assertEqual(guard.check(self.root), [])

    def test_android_incremental_inputs_cannot_omit_runtime_source(self):
        self.edit("android/app/build.gradle.kts", lambda text: text.replace('fileTree(repositoryRoot.resolve("shared/mimi-runtime"))', 'fileTree(repositoryRoot.resolve("unused"))'))
        self.rejected("Android incremental native inputs")

    def test_android_incremental_inputs_cannot_omit_build_scripts(self):
        self.edit("android/app/build.gradle.kts", lambda text: text.replace('"build.rs", ', ""))
        self.rejected("Android incremental native inputs")

    def test_builds_cannot_switch_to_an_external_runtime_version(self):
        self.edit("shared/mimi-android-jni/Cargo.toml", lambda text: text.replace('path = "../mimi-runtime"', 'version = "0.1"'))
        self.rejected("both builds must use repository shared source")

    def test_android_ci_cannot_skip_pc_or_shared_changes(self):
        for pattern in ("src-tauri/**", "src/**", "shared/**"):
            with self.subTest(pattern=pattern):
                path = self.root / ".github/workflows/android.yml"
                original = path.read_text(encoding="utf-8")
                path.write_text(original.replace(f"      - '{pattern}'\n", ""), encoding="utf-8")
                self.rejected("Android CI must follow desktop/shared changes")
                path.write_text(original, encoding="utf-8")

    def test_build_and_ci_gates_cannot_be_removed(self):
        for filename, transform, message in (
            ("android/app/build.gradle.kts", lambda text: text.replace('it.name == "preBuild"', 'it.name == "unused"'), "Android compilation must depend"),
            ("android/app/build.gradle.kts", lambda text: text.replace('tasks.matching { it.name == "preBuild" }.configureEach {\n    dependsOn(verifySharedRuntimeArchitecture)', 'tasks.matching { it.name == "preBuild" }.configureEach {\n    // removed dependency'), "Android compilation must depend"),
            ("scripts/build-shared-core.py", lambda text: text.replace('"check-shared-runtime.py"', '"unused.py"'), "direct native-library build"),
            ("scripts/check-shared-core.sh", lambda text: text.replace("scripts/check-shared-runtime-test.py", "unused.py"), "canonical shared check"),
            (".github/workflows/ci.yml", lambda text: text.replace("scripts/check-shared-runtime.py", "unused.py"), "desktop CI"),
        ):
            with self.subTest(filename=filename):
                path = self.root / filename
                original = path.read_text(encoding="utf-8")
                path.write_text(transform(original), encoding="utf-8")
                self.rejected(message)
                path.write_text(original, encoding="utf-8")


if __name__ == "__main__":
    unittest.main()
