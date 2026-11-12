#!/usr/bin/env python3
"""Generate shared desktop/Android service copy and presets; --check rejects drift."""
from __future__ import annotations

import argparse
import ipaddress
import json
import re
import sys
from pathlib import Path
from urllib.parse import urlsplit
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "shared/provider-copy.json"
TS_OUTPUT = ROOT / "src/lib/generated/providerCopy.ts"
KOTLIN_OUTPUT = ROOT / "android/app/src/main/java/app/yuxino/mimi/android/GeneratedTextTranslationPresets.kt"
SERVICE_SETUP_OUTPUT = ROOT / "android/app/src/main/java/app/yuxino/mimi/android/GeneratedServiceSetups.kt"
RESOURCE_ROOT = ROOT / "android/app/src/main/res"


def compatible_presets(data: dict) -> list[dict[str, str]]:
    presets = data["compatiblePresets"]
    if not isinstance(presets, list) or not presets:
        raise ValueError("Compatible presets must be a non-empty list")
    ids = set()
    for preset in presets:
        if not isinstance(preset, dict) or set(preset) != {
                "id", "label", "endpoint", "model", "apiKeyUrl", "guideUrl", "setupKind", "setupHelpKey"}:
            raise ValueError("Invalid compatible preset fields")
        if any(not isinstance(value, str) or any(not char.isprintable() for char in value)
               for value in preset.values()):
            raise ValueError("Invalid compatible preset text")
        preset_id = preset["id"]
        if not re.fullmatch(r"[a-z][a-zA-Z0-9]*", preset_id) or preset_id in ids:
            raise ValueError(f"Invalid or duplicate compatible preset ID: {preset_id}")
        ids.add(preset_id)
        if not preset["label"].strip() or len(preset["label"]) > 80:
            raise ValueError(f"Invalid compatible preset label: {preset_id}")
        if len(preset["model"].encode("utf-8")) > 256 or preset["model"] != preset["model"].strip():
            raise ValueError(f"Invalid compatible preset model: {preset_id}")
        endpoint = preset["endpoint"]
        try:
            url = urlsplit(endpoint)
            host = url.hostname
            local = host == "localhost"
            if host and not local:
                try:
                    local = ipaddress.ip_address(host).is_loopback
                except ValueError:
                    pass
            if (not host or url.username is not None or url.password is not None or url.query or url.fragment
                    or len(endpoint) > 2048 or endpoint != endpoint.strip()
                    or not (url.scheme == "https" or url.scheme == "http" and local)):
                raise ValueError("Unsafe endpoint")
            _ = url.port
        except ValueError as error:
            raise ValueError(f"Invalid compatible preset endpoint: {preset_id}") from error
        if preset["setupKind"] not in {"cloud", "local", "keyless"}:
            raise ValueError(f"Invalid compatible preset setup kind: {preset_id}")
        if bool(preset["apiKeyUrl"]) != (preset["setupKind"] == "cloud"):
            raise ValueError(f"Invalid compatible preset API key link: {preset_id}")
        for field in ("apiKeyUrl", "guideUrl"):
            value = preset[field]
            if field == "apiKeyUrl" and not value:
                continue
            validate_setup_url(value, preset_id)
        key = preset["setupHelpKey"]
        help_entry = data.get("entries", {}).get(key)
        if (not re.fullmatch(r"[a-zA-Z][a-zA-Z0-9]*", key) or not isinstance(help_entry, dict)
                or set(help_entry.get("scope", [])) != {"desktop", "android"}
                or not help_entry.get("androidResource")):
            raise ValueError(f"Invalid compatible preset setup help: {preset_id}")
    return presets


def validate_setup_url(value: str, preset_id: str) -> None:
    """Only fixed public HTTPS pages may become explicit browser destinations."""
    try:
        url = urlsplit(value)
        host = url.hostname
        if (url.scheme != "https" or not host or host == "localhost" or host.endswith(".localhost")
                or url.username is not None or url.password is not None
                or len(value) > 2048 or value != value.strip() or url.port not in {None, 443}
                or any(not char.isprintable() for char in value)):
            raise ValueError("Unsafe setup page")
        try:
            address = ipaddress.ip_address(host)
        except ValueError as error:
            if (not re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?\.[A-Za-z]{2,63}", host)
                    or host.endswith((".local", ".internal", ".invalid", ".test", ".localdomain"))):
                raise ValueError("Invalid setup hostname") from error
        else:
            if not address.is_global:
                raise ValueError("Non-public setup page")
    except ValueError as error:
        raise ValueError(f"Invalid compatible preset setup URL: {preset_id}") from error


def kotlin_string(value: str) -> str:
    return json.dumps(value, ensure_ascii=False).replace("$", "\\$")


def service_setups(data: dict) -> list[dict[str, str]]:
    setups = data["serviceSetups"]
    if not isinstance(setups, list) or not setups:
        raise ValueError("Service setups must be a non-empty list")
    ids = set()
    for setup in setups:
        if not isinstance(setup, dict) or set(setup) != {"id", "label", "apiKeyUrl", "guideUrl"}:
            raise ValueError("Invalid service setup fields")
        if any(not isinstance(value, str) or any(not char.isprintable() for char in value)
               for value in setup.values()):
            raise ValueError("Invalid service setup text")
        setup_id = setup["id"]
        if not re.fullmatch(r"[a-z][a-zA-Z0-9]*", setup_id) or setup_id in ids:
            raise ValueError(f"Invalid or duplicate service setup ID: {setup_id}")
        ids.add(setup_id)
        if not setup["label"].strip() or len(setup["label"]) > 80:
            raise ValueError(f"Invalid service setup label: {setup_id}")
        for field in ("apiKeyUrl", "guideUrl"):
            try:
                validate_setup_url(setup[field], setup_id)
            except ValueError as error:
                raise ValueError(f"Invalid service setup URL: {setup_id}") from error
    return setups


def generated_files() -> dict[Path, str]:
    data = json.loads(SOURCE.read_text(encoding="utf-8"))
    if data["schemaVersion"] != 1:
        raise ValueError("Unsupported provider copy schema")
    locales = data["locales"]
    entries = data["entries"]
    presets = compatible_presets(data)
    setups = service_setups(data)
    desktop = (ROOT / "src/lib/i18n.ts").read_text(encoding="utf-8")
    language_match = re.search(r"UI_LANGUAGES = \[([^]]+)]", desktop)
    if not language_match or set(re.findall(r'"([^\"]+)"', language_match[1])) != set(locales):
        raise ValueError("Shared copy must cover every desktop UI language")
    expected_resources = set()
    for key, entry in entries.items():
        if not re.fullmatch(r"[a-zA-Z][a-zA-Z0-9]*", key):
            raise ValueError(f"Invalid copy key: {key}")
        if set(entry["translations"]) != set(locales):
            raise ValueError(f"Missing or extra translations: {key}")
        if not entry["scope"] or set(entry["scope"]) - {"desktop", "android"}:
            raise ValueError(f"Invalid platform scope: {key}")
        resource = entry["androidResource"]
        if resource:
            if "android" not in entry["scope"] or not re.fullmatch(r"[a-z][a-z0-9_]*", resource):
                raise ValueError(f"Invalid Android binding: {key}")
            if resource in expected_resources:
                raise ValueError(f"Duplicate Android binding: {resource}")
            expected_resources.add(resource)
        mode_field = entry.get("desktopModeField")
        if mode_field:
            owner = ROOT / "src/windows/settings/AlibabaRealtimeSettings.tsx"
            if f"{mode_field}: common.{key}" not in owner.read_text(encoding="utf-8"):
                raise ValueError(f"Missing desktop mode binding: {key}")
        if entry["desktopSetting"]:
            for locale in locales:
                owner = ROOT / f"src/lib/locales/{locale}.ts"
                binding = f'{entry["desktopSetting"]}: PROVIDER_COPY["{locale}"].{key}'
                if binding not in owner.read_text(encoding="utf-8"):
                    raise ValueError(f"Missing desktop binding: {locale}/{key}")
    contract = json.loads((ROOT / "shared/translation-contracts.json").read_text(encoding="utf-8"))
    language_count = len(contract["alibabaRealtimeLanguages"]["languages"])
    rendered = {}
    for locale in locales:
        rendered[locale] = {}
        for key, entry in entries.items():
            value = entry["translations"][locale]
            if not isinstance(value, str) or not value.strip():
                raise ValueError(f"Empty translation: {locale}/{key}")
            rendered[locale][key] = value.replace("{liveLanguageCount}", str(language_count))
    desktop_copy = {
        locale: {key: value for key, value in copy.items() if "desktop" in entries[key]["scope"]}
        for locale, copy in rendered.items()
    }
    outputs = {
        TS_OUTPUT: "// Generated by scripts/sync-provider-copy.py. Edit shared/provider-copy.json.\n"
        + "export const PROVIDER_COPY = " + json.dumps(desktop_copy, ensure_ascii=False, indent=2) + " as const;\n"
        + "\nexport const OPENAI_COMPATIBLE_PRESETS = " + json.dumps(presets, ensure_ascii=False, indent=2) + " as const;\n"
        + "\nexport const SERVICE_SETUPS = " + json.dumps(setups, ensure_ascii=False, indent=2) + " as const;\n",
        KOTLIN_OUTPUT: "// Generated by scripts/sync-provider-copy.py. Edit shared/provider-copy.json.\n"
        + "package app.yuxino.mimi.android\n\n"
        + "internal data class TextTranslationPreset(\n"
        + "    val id: String, val label: String, val endpoint: String, val model: String,\n"
        + "    val apiKeyUrl: String, val guideUrl: String, val setupKind: String, val setupHelpResource: Int,\n)\n\n"
        + "internal object TextTranslationPresets {\n    val entries: List<TextTranslationPreset> = listOf(\n"
        + "".join("        TextTranslationPreset(" + ", ".join(kotlin_string(preset[key]) for key in
                  ("id", "label", "endpoint", "model", "apiKeyUrl", "guideUrl", "setupKind"))
                  + ", R.string." + entries[preset["setupHelpKey"]]["androidResource"] + "),\n"
                  for preset in presets)
        + "    )\n}\n",
        SERVICE_SETUP_OUTPUT: "// Generated by scripts/sync-provider-copy.py. Edit shared/provider-copy.json.\n"
        + "package app.yuxino.mimi.android\n\n"
        + "internal data class ServiceSetup(\n"
        + "    val id: String, val label: String, val apiKeyUrl: String, val guideUrl: String,\n)\n\n"
        + "internal object ServiceSetups {\n    val entries: List<ServiceSetup> = listOf(\n"
        + "".join("        ServiceSetup(" + ", ".join(kotlin_string(setup[key]) for key in
                  ("id", "label", "apiKeyUrl", "guideUrl")) + "),\n" for setup in setups)
        + "    )\n}\n"
    }
    for locale, directory in locales.items():
        root = RESOURCE_ROOT / directory
        for file in root.glob("*.xml"):
            if file.name == "provider_strings.xml":
                continue
            for resource in expected_resources:
                if re.search(r'<string\b[^>]*\bname="' + resource + '"', file.read_text(encoding="utf-8")):
                    raise ValueError(f"Duplicate shared resource in {file.relative_to(ROOT)}: {resource}")
        lines = ['<?xml version="1.0" encoding="utf-8"?>',
                 '<!-- Generated by scripts/sync-provider-copy.py. Edit shared/provider-copy.json. -->', '<resources>']
        for key, entry in entries.items():
            resource = entry["androidResource"]
            if resource:
                value = rendered[locale][key].replace("\\", "\\\\").replace("\n", "\\n").replace("'", "\\'").replace('"', '\\"')
                lines.append(f'    <string name="{resource}">{escape(value)}</string>')
        lines.append('</resources>')
        outputs[root / "provider_strings.xml"] = "\n".join(lines) + "\n"
    return outputs


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        outputs = generated_files()
    except (KeyError, ValueError) as error:
        print(f"Provider copy: {error}", file=sys.stderr)
        return 1
    stale = []
    for path, content in outputs.items():
        if path.exists() and path.read_text(encoding="utf-8") == content:
            continue
        stale.append(str(path.relative_to(ROOT)))
        if not args.check:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
    if args.check and stale:
        print("Provider copy is out of date; run python3 -B scripts/sync-provider-copy.py:\n" + "\n".join(stale), file=sys.stderr)
        return 1
    print(f"Shared provider copy {'verified' if args.check else 'generated'} ({len(outputs)} files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
