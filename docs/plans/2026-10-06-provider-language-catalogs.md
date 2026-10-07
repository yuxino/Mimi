# Provider language choices match the integrated model

The language registry is an encoding vocabulary, not a provider capability list.
Keep Audio3's 30 hints and Qwen-MT Lite's 31 targets independent from that registry.
Custom deployments may declare a subset; Service default omits a hint and does not
promise automatic recognition. Do not infer model support from a generic API field.

Use the existing searchable language controls in settings, tray and overlay control.
All surfaces resolve the same active provider, translation route and source-dependent
pair rules. Target summaries follow the saved setting. Do not add a separate visual
style or a second list maintained only for the floating window.

Target switching is available for integrated translators as well as independent
translation routes. The native command validates the selected target against the
current source-dependent catalog and requires that exact target to survive
normalization; it does not silently save a fallback target. Same-language choices
remain available on Original-capable independent routes and documented mixed
translation modes. A target change keeps paused sessions paused, reconnects a
listening session, and obeys the existing busy and newer-stop/pause guards. Tray
and floating controls have the narrowly scoped target-switch permission.

Apple Speech settings use one selector for supported languages, showing the
chosen language's resource status and Download and use or Set recognition language
action together. Selecting alone does not change saved preferences. Quick-switch
menus continue to expose ready compatible sources, and all surfaces display the
saved source and target after successful application. Settings source/target fields
retain the shared row alignment.

## Verified catalog corrections

- Gemini's exact Live Translate model exposes 78 targets, including separate
  Portuguese regions. Source remains automatic; the generic Live API hint does not
  establish support in this translation model.
- OpenAI's dedicated translation model retains its 13 targets. Azure deployment of
  the same model uses those 13 targets, rather than Mimi's former three-target guard.
- xAI exposes its 20 documented speech-to-speech codes, including Arabic, Portuguese
  and Spanish regions, plus automatic source detection. Never silently substitute a
  region for an ambiguous generic source hint.
- Baidu's real-time speech translation has 45 mapped language codes in both directions.
  Tencent has nine sources and a source-dependent target matrix. `zh_en` means explicit
  Chinese/English mixing; `zh_en` to `zh_en` translates each utterance to the other
  language, so it must not be treated as a no-op or universal automatic detection.
- Volcano AST2.0 S2T exposes 20 ordinary languages plus Cantonese/Shanghainese
  inputs. One side must be Chinese or English. Its separate `zhen` reversal mode
  uses the canonical `zh_en` with a provider-specific label; `yue`/`wuu` map to
  `yue-CN`/`sh-CN`. Mixed reversal must not be normalized into passthrough.
- DeepL and DeepLX have separate, documented text catalogs and wire mappings.
  Recognition options intersect the chosen ASR's actual catalog; expanding translation
  does not expand Audio3 recognition. Automatic text detection is preserved by omitting
  DeepL's source field or sending DeepLX's `auto`.
- Android's built-in Alibaba live translation model differs from desktop Audio3.
  Its 60-language translation route and 27 explicit ASR hints are resolved separately;
  independent text translation uses the ASR route's intersection.
- Apple remains a runtime catalog. Explicit regional variants require an exact native
  match; a generic language resource does not certify all regional variants.

The evidence table and official links live in [language setup](../speech-language-setup.md).
Cross-platform synthetic wire fixtures verify option-to-request mappings and negative
cases. Catalog tests verify each advertised code is serializable and accepted by the
owning adapter. Native Apple preparation/text checks and browser visual fixtures are
recorded separately from cloud-account or full audio-session validation.
