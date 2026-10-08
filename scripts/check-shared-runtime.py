#!/usr/bin/env python3
"""Fail on known ways of reconnecting a separate desktop/Android live runtime.

This is an architecture check, not a compiler or a proof of device parity.
Native capture, trust/proxy adaptation, lifecycle and rendering remain native.
"""
import argparse
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parent.parent
PROVIDER = "android/app/src/main/java/app/yuxino/mimi/android/provider/"
LEGACY_FILES = {
    "DashScopeEngine.kt", "OpenAIRealtimeEngine.kt", "StreamingServiceEngine.kt",
    "CloudProtocols.kt", "VolcanoProtocol.kt", "GrokProtocol.kt",
    "TranslationPipeline.kt", "OpenAITranslationClient.kt",
    "DeepLTranslationClient.kt", "DeepLXTranslationClient.kt",
    "ProviderLanguageCatalogs.kt", "DeepLLanguages.kt", "ProviderFinishGate.kt",
    "TranscriptBuffer.kt", "SharedLivePairStream.kt", "SharedTranscriptStream.kt",
}
LEGACY_SYMBOLS = {
    "DashScopeEngine", "OpenAIRealtimeEngine", "StreamingServiceEngine",
    "TranslationPipeline", "OpenAITranslationClient", "DeepLTranslationClient",
    "DeepLXTranslationClient", "BoundedTranslationHttpClient", "TranslationClient",
    "ServiceProtocol", "ServiceEvent", "WireFrame", "VolcanoProtocol",
    "GrokProtocol", "TencentProtocol", "BaiduProtocol", "GeminiProtocol",
    "AzureProtocol", "TranscriptBuffer", "SharedLivePairStream",
    "SharedTranscriptStream", "ProviderFinishGate", "createTranslationClient",
}
# Strings are retained for config checks and hidden for executable-token checks.
LEXEMES = re.compile(
    r'"""[\s\S]*?"""|"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\''
    r'|//[^\n]*|/\*[\s\S]*?\*/'
)


def code(text, hide_strings=False):
    def replace(match):
        token = match.group()
        if token.startswith(("//", "/*")) or hide_strings:
            return "\n" * token.count("\n") + " "
        return token
    return LEXEMES.sub(replace, text)


def check(root=ROOT):
    root = Path(root)
    errors = []

    def read(relative):
        path = root / relative
        if not path.is_file():
            errors.append(f"missing architecture input: {relative}")
            return ""
        return path.read_text(encoding="utf-8")

    def require(condition, message):
        if not condition:
            errors.append(message)

    # A desktop facade cannot grow a local function or point to another module.
    runtime = root / "shared/mimi-runtime/src"
    desktop = root / "src-tauri/src"
    candidates = set()
    for directory in ("clients", "core/protocols"):
        for path in (desktop / directory).rglob("*.rs"):
            relative = path.relative_to(desktop)
            shared = runtime / relative
            if path.name == "mod.rs":
                body = code(path.read_text(encoding="utf-8"))
                declarations = re.findall(r"(?:pub\s+)?mod\s+(\w+)\s*;", body)
                rest = re.sub(r"#\[(?:cfg|allow)\([^\]]*\)\]|(?:pub\s+)?mod\s+\w+\s*;", "", body)
                require(not rest.strip(), f"desktop module list contains implementation: {relative}")
                for name in declarations:
                    require((shared.parent / f"{name}.rs").is_file()
                            or (shared.parent / name / "mod.rs").is_file(),
                            f"desktop module has no shared implementation: {relative}::{name}")
            else:
                require(shared.is_file(), f"desktop module has no shared implementation: {relative}")
                candidates.add(relative)
    for path in (desktop / "core").rglob("*.rs"):
        relative = path.relative_to(desktop)
        if path.name != "mod.rs" and (runtime / relative).is_file():
            candidates.add(relative)
    candidates.add(Path("audio/send_pipeline.rs"))
    require(bool(candidates - {Path("audio/send_pipeline.rs")}), "desktop shared facades are missing")
    core_modules = code(read("src-tauri/src/core/mod.rs"))
    core_rest = re.sub(r"#\[(?:cfg|allow)\([^\]]*\)\]|(?:pub\s+)?mod\s+\w+\s*;", "", core_modules)
    require(not core_rest.strip(), "desktop core module list cannot redirect shared facades or contain implementation")
    for relative in sorted(candidates):
        body = code(read(f"src-tauri/src/{relative.as_posix()}"))
        body = re.sub(r"#\[allow\(unused_imports\)\]", "", body)
        module = "::".join(relative.with_suffix("").parts)
        expected = f"pub use mimi_runtime::{module}::*;"
        require(re.sub(r"\s+", "", body) == re.sub(r"\s+", "", expected),
                f"desktop facade must only re-export shared code: {relative}")

    # Legacy Kotlin is retained for offline fixtures. Production must not call it.
    android = root / "android/app/src"
    sources = []
    for directory in android.iterdir() if android.is_dir() else []:
        if not directory.is_dir() or directory.name.startswith(("test", "androidTest")):
            continue
        for suffix in ("*.kt", "*.java"):
            sources.extend(directory.rglob(suffix))
    require(bool(sources), "Android production sources are missing")
    engine_users = []
    for path in sorted(sources):
        relative = path.relative_to(root).as_posix()
        if relative in {PROVIDER + name for name in LEGACY_FILES}:
            continue
        text = code(path.read_text(encoding="utf-8"), hide_strings=True)
        if relative == PROVIDER + "ServiceCatalog.kt":
            # These two legacy constants are metadata only, not executable clients.
            text = re.sub(r"OpenAIRealtimeEngine\s*\.\s*(?:ENDPOINT|MODEL)\b", "", text)
        tokens = set(re.findall(r"\b\w+\b", text))
        leaked = sorted(tokens & LEGACY_SYMBOLS)
        require(not leaked, f"Android production references offline legacy runtime: {relative}: {', '.join(leaked)}")
        require(not re.search(r"\b(?:newWebSocket|newCall|openConnection|sendAsync)\s*\(", text),
                f"Android production creates provider transport outside Rust: {relative}")
        implementations = []
        for header in re.findall(r"(?<![\w.])(?:class\s+\w+|object(?=\s|:))[^{};]*(?=[{;])", text):
            # Constructor parameters may use ProviderEngine as a capture adapter.
            # Remove balanced parentheses so only inheritance is checked.
            while True:
                reduced = re.sub(r"\([^()]*\)", "", header)
                if reduced == header:
                    break
                header = reduced
            if re.search(r"(?::|\bimplements\b)[^{};]*\bProviderEngine\b", header):
                implementations.append(header)
        require(all(relative == PROVIDER + "SharedRuntimeEngine.kt"
                    and re.match(r"class\s+SharedRuntimeEngine\b", header)
                    for header in implementations),
                f"Android production defines another provider engine: {relative}")
        if re.search(r"\bSharedRuntimeEngine\s*\(", text) and relative != PROVIDER + "SharedRuntimeEngine.kt":
            engine_users.append(relative)
    require(engine_users == ["android/app/src/main/java/app/yuxino/mimi/android/capture/MimiService.kt"],
            "Android production session must enter SharedRuntimeEngine from MimiService")

    for manifest, dependencies in {
        "src-tauri/Cargo.toml": {"mimi-core": "../shared/mimi-core", "mimi-runtime": "../shared/mimi-runtime"},
        "shared/mimi-android-jni/Cargo.toml": {"mimi-core": "../mimi-core", "mimi-runtime": "../mimi-runtime"},
        "shared/mimi-runtime/Cargo.toml": {"mimi-core": "../mimi-core"},
    }.items():
        text = read(manifest)
        section = re.search(r"(?ms)^\[dependencies\]\s*\n(.*?)(?=^\[|\Z)", text)
        for name, source in dependencies.items():
            entry = re.search(rf"(?m)^{re.escape(name)}\s*=\s*\{{([^}}]*)\}}", section[1] if section else "")
            require(bool(entry and re.search(rf'\bpath\s*=\s*"{re.escape(source)}"', entry[1])),
                    f"both builds must use repository shared source: {manifest}: {name}")

    gradle = code(read("android/app/build.gradle.kts"))
    for crate in ("mimi-core", "mimi-runtime", "mimi-android-jni"):
        tree = re.search(rf'fileTree\(repositoryRoot.resolve\("shared/{crate}"\)\)\s*\{{([^}}]*)\}}', gradle)
        require(bool(tree and all(f'"{item}"' in tree[1] for item in ("Cargo.toml", "Cargo.lock", "src/**", "build.rs"))),
                f"Android incremental native inputs must track shared source and build script: {crate}")
    require('tasks.register<Exec>("verifySharedRuntimeArchitecture")' in gradle
            and bool(re.search(r'tasks.matching\s*\{\s*it.name == "preBuild"\s*\}.configureEach\s*\{\s*dependsOn\(verifySharedRuntimeArchitecture\)', gradle)),
            "Android compilation must depend on shared runtime architecture verification")
    require('inputs.files(sharedCoreNativeInputs)' in gradle and 'dependsOn(buildSharedCoreHost)' in gradle,
            "Android native builds and JVM tests must track actual shared source")
    build = read("scripts/build-shared-core.py")
    require(bool(re.search(r'subprocess.run\(\[sys.executable,\s*str\(ROOT / "scripts" / "check-shared-runtime.py"\)\]', build)),
            "direct native-library build must verify shared runtime architecture")
    shared_check = read("scripts/check-shared-core.sh")
    require(all(re.search(rf'(?m)^"\$python_command" -B scripts/{filename}\s*$', shared_check)
                for filename in ("check-shared-runtime.py", "check-shared-runtime-test.py")),
            "canonical shared check must run architecture verification and its rejection tests")
    ci = read(".github/workflows/ci.yml")
    require(all(re.search(rf'(?m)^\s+python3 -B scripts/{filename}\s*$', ci)
                for filename in ("check-shared-runtime.py", "check-shared-runtime-test.py")),
            "desktop CI must verify shared runtime architecture even for frontend-only changes")
    android_ci = read(".github/workflows/android.yml")
    for event in ("pull_request", "push"):
        section = re.search(rf"(?ms)^  {event}:\n(.*?)(?=^  \w+:|^\w|\Z)", android_ci)
        for pattern in ("android/**", "shared/**", "src-tauri/**", "src/**", "scripts/**", ".github/workflows/**"):
            require(bool(section and re.search(rf"(?m)^      - '{re.escape(pattern)}'\s*$", section[1])),
                    f"Android CI must follow desktop/shared changes: {event}: {pattern}")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    errors = check(args.root)
    if errors:
        for error in errors:
            print(f"shared runtime boundary: {error}", file=sys.stderr)
        return 1
    print("Shared runtime architecture verified (source ownership, production entry, build inputs and CI scope).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
