# Real model language capabilities

The user requested language choices based on actual service support, and then
authorized testing a faster Alibaba text model before the broader catalog change.
The live/probe text model is selected by the shared `REALTIME_MT_MODEL` constant:
Qwen-MT Lite. It supports incremental streaming and 31 language codes, while
Flash/Plus support 92. Lite does not accept `domains`; its encoder omits that
field without rewriting existing prompt assets, terms or translation memory.

Primary sources:

- [Qwen-MT model selection, domain limitations and language tables](https://help.aliyun.com/en/model-studio/machine-translation)
- [Audio 3.0 streaming language hints and automatic detection](https://help.aliyun.com/en/model-studio/qwen-audio-asr-streaming-python-sdk)
- [DeepL resource-specific source, target and detection metadata](https://developers.deepl.com/docs/languages/using-the-languages-api)

The existing `qwen-audio-3.0-asr-flash-streaming` ASR accepts 30 source languages.
Omitting `language_hints` uses actual model detection. The ASR and MT lists must
remain separate: Norwegian, Romanian, Greek, Bulgarian, Croatian and Slovak are
supported ASR inputs but are outside Lite's MT table. The Lite pipeline therefore
has 24 documented ASR/MT source languages, plus automatic detection;
automatic detection cannot guarantee translation of all 30 ASR languages.

The Qwen model registry records the exact 31/92 upstream code sets. Protocol
requests reject unsupported explicit source/target codes. A reported-source
helper distinguishes known unsupported languages from absent detection, preserves
Chinese scripts, and maps documented Filipino and Norwegian aliases. The client
checks that helper before MT requests, so a known unsupported detected language
is rejected locally. The subsequent [language-control implementation](2026-10-02-provider-language-controls.md)
extends the typed enums and actual route-aware settings choices.

The route-aware selector in `providerCapabilities.ts` uses Audio 3.0's 30 inputs,
Lite's 31 targets and their 24-language translated-source intersection. A parity
fixture in `providerCapabilities.test.ts` checks the Rust realtime default and
the Lite language table. DeepL and custom-service routes retain conservative
choices until their endpoint's available languages are known. Automatic detection
never promises translation outside the selected text model's range.

The larger catalog update must preserve existing serialized `zh`, `en`, `ja`,
`ko` and `auto` values, update actual wire mappings, and validate capabilities by
the saved speech/text route. Official DeepL text targets must use its translation
resource list rather than its style-rule, voice or SDK union list. Custom-service
languages stay conservative until that endpoint advertises known capabilities.
Unverified providers must not inherit Alibaba or DeepL's expanded lists.

Focused fixtures cover exact unique registry sizes, regional report aliases,
ASR-only language exclusions, model-specific domain omission, stream flags and
the existing terms/memory payloads. Installed native timing and canonical checks
are distinct from those protocol fixtures.
