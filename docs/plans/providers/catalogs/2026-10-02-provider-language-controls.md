# Provider-aware language controls

## Status: implemented and verified

This phase follows the subtitle read-position acceptance merged in PR #97.
Language choices now live inside the active service detail, alongside its name,
credentials, and text-translation route. Global proxy settings appear below the
service section with an explicit all-services scope. Large language catalogs
use a searchable picker; small catalogs remain direct buttons. The previous
read-only list of additional languages is removed.

The typed source and target catalogs preserve existing keys while covering the
Audio3 and Lite scopes below. Native snapshots include active profile ID,
provider, text-translation route, target, and resolved language options. The
frontend accepts this snapshot only when its route stamp matches; older
snapshots use the equivalent local resolver.

Explicit source and target settings are independent: choosing Chinese in this
form no longer forces Original. The subsequent
[shared language-menu repair](../../desktop/settings/services/2026-10-02-shared-language-pickers.md) removes the
overlay/tray's five-language filter: all three surfaces use the full active-route
catalog, with searchable bounded menus in the compact surfaces. Source selections
also consistently preserve the current target subject to provider normalization;
Chinese no longer implicitly enables Original in the compact menus.
Settings edits require a stopped session. Transient connection checks disable
controls without incorrectly telling users to stop an already stopped session.

## Verified upstream scope

The official pages were checked on 2026-10-02. This proposal retains Mimi's
current `qwen-audio-3.0-asr-flash-streaming` and `qwen-mt-lite` models; it does not
adopt a different model merely because an upstream example uses one.

[Audio ASR streaming documentation](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-python-sdk)
lists 30 `language_hints` codes and automatic detection when hints are omitted:

```text
zh en ja ko vi th id ms tl hi ar fr de es pt ru it nl sv da fi no el pl cs hu ro bg hr sk
```

[Qwen-MT documentation](https://help.aliyun.com/en/model-studio/machine-translation)
lists 31 Lite languages. The same list is already registered in the core model
metadata and the display-only frontend catalog:

```text
en zh zh_tw ru ja ko es fr pt de it th vi id ms ar hi he ur bn pl nl tr km cs sv hu da fi tl fa
```

The explicit Audio3-to-Lite source intersection contains 24 codes:

```text
zh en ja ko vi th id ms tl hi ar fr de es pt ru it nl sv da fi pl cs hu
```

`no`, `el`, `ro`, `bg`, `hr`, and `sk` are documented Audio3 recognition
languages outside Lite's list. `zh_tw`, `he`, `ur`, `bn`, `tr`, `km`, and `fa`
are additional Lite targets, not additional Audio3 language hints. Keep the
Filipino/Tagalog labels appropriate to each stage. `zh` remains the existing
simplified-Chinese target and `zh_tw` is a distinct traditional-Chinese target.

## Route-aware selection

| Active route | Explicit source controls | Target controls |
| --- | --- | --- |
| Alibaba + Follow Service/Lite | 24-code intersection, plus Automatic | Lite's 31 targets, plus Original |
| Alibaba + Original/no translation | Audio3's 30 codes, plus Automatic | All targets remain available; changing to translation normalizes an unsupported source before any request |
| Alibaba + official DeepL | Existing selections until DeepL resource capabilities and wire mappings are implemented | Do not inherit Lite's target catalog |
| Alibaba + custom translation service | Existing conservatively supported selections | Do not infer capabilities from an arbitrary endpoint |
| Other speech providers | Existing per-provider selections | Existing per-provider selections |

Previously, configuration validated only `provider.capabilities()`. Official
DeepL uses the same effective Alibaba speech provider as Follow Service.
Consequently, expanding Alibaba's global capability vector alone would also
expand the wrong text-translation route. The implementation uses shared profile/route
capability resolution used by settings normalization, configuration validation,
connection probes, and frontend selectors. Provider metadata, selected route,
and actual model must agree. Route changes must normalize unsupported saved
selections without issuing requests with a fabricated fallback language.

Automatic detection is a real ASR request mode, but not a promise that Lite can
translate every recognized Audio3 language. The current Audio3 transcription
event does not carry a detected-language field; its shared event fills a language
only for an explicitly selected source. Automatic events therefore reach HQ
with `language = None`. The existing unsupported-source guard cannot establish
the detected language in this case. Preserve the truthful automatic option and
explain the Lite intersection. Decode a reported language only if the official
response protocol actually provides it; do not guess from recognized text or
manufacture an identity. Unknown language retains the current automatic MT path
and an actual unsupported-service response must remain an explicit failure.

## Implementation boundaries

1. Extend typed source/target models and serde while preserving all existing
   keys. Source models may include all 30 Audio3 codes to support Original;
   route capabilities restrict translated Audio3-to-Lite sessions to the 24
   explicit codes. Expand `from_detected` only for verified codes and retain
   Chinese script distinctions where conversion is necessary.
2. Map new Audio3 hints and Lite source/target names exactly. Preserve all
   existing prompts and filler entries. New ASR languages may reuse the existing
   generic audiovisual context instead of inventing new translated prompts.
   Keep same-language MT bypass conservative: only explicit equivalent language
   and script reports qualify; unknown reports and script conversions use MT.
3. Make other protocol encoders reject unsupported newly representable languages
   through their existing fixed error path. Extending a shared enum must not
   silently expand another provider or encode an unrelated default language.
4. Resolve per-route capabilities consistently for preferences, selected-profile
   probes, active-session language changes, and the frontend. Wire network routes,
   pacing, final ordering, model choice, and credential scope remain unchanged.
5. Render actual selectable languages in service controls with a compact,
   searchable list for large catalogs and direct options for small catalogs.
   The existing upstream availability section may remain supporting evidence,
   but it cannot substitute for the working picker. Localize names and distinguish
   scripts; a display name must never determine a wire code.

## Required verification before claiming completion

- Round-trip every new language key and preserve existing saved preferences.
- Verify all 31 Lite target payloads and the 24 explicit source payloads against
  exact upstream codes/names; test Original's broader 30-code ASR boundary.
- Check automatic hints omission, known unsupported reports, unknown reports,
  and traditional/simplified Chinese conversion without widening MT bypass.
- Check route changes, configuration/probe normalization, DeepL/custom isolation,
  and every unchanged provider's capability boundary.
- Exercise real frontend selections through IPC, including loading saved values,
  compact search, keyboard navigation, and switching profiles/routes.
- Run the canonical check and signed-dev acceptance of representative added
  source/target pairs. Automated payload fixtures do not establish recognition
  or translation quality for all added languages, or native acceptance on every OS.

PR #98 is merged. Canonical checks and representative signed-macOS source/target
tests passed; see the [acceptance report](../../../development/2026-10-02-service-language-acceptance.md)
for exact source revision, measurements, CI results, and remaining limits.
