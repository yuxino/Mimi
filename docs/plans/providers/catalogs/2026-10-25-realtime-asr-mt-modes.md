# Realtime recognition and independent translation modes

**Goal:** Let OpenAI, Gemini and Azure OpenAI profiles choose simultaneous
translation or recognition followed by an independently selected text translator.
New profiles default to simultaneous translation, as requested. Missing metadata
preserves the same route for existing profiles.

**Architecture:** Shared Rust owns the mode, recognition protocols, capability
intersection, independent credentials, probes and bounded translation pipeline.
Desktop and Android persist and edit the same contract. Existing Alibaba and
xAI mode contracts remain compatible; their defaults are outside this change.

**Tech stack:** Rust shared runtime and JNI, Tauri, React/TypeScript, native
Android Kotlin UI.

## Behavior and implementation

1. Add `speechTranslationMode` (`simultaneous` / `asrMt`) for the three providers.
   Validate unsupported provider/mode combinations. Preserve a saved text
   destination, its metadata, credentials and proxy while simultaneous mode is
   selected; none of those may cause a text request in that mode.
2. Reuse the independent recognition and prioritized MT pipeline. OpenAI uses
   dedicated realtime transcription; Azure uses its transcription deployment
   and resource authentication; Gemini uses dedicated Live transcription.
   Do not emulate recognition by asking a conversational model to transcribe.
3. ASR + MT uses explicit independent text configuration, defaulting its route
   to OpenAI-compatible when absent. Never silently reuse a recognition key or
   create a credential from another profile. Original-only makes no MT request.
4. Expose a compact, shared mode selector and the existing independent text
   editor in desktop and Android. Switching requires a stopped session, keeps
   stored configuration, intersects supported languages, and reports failures
   through existing feedback. Connection checks remain independent.
5. Update shared fixtures, Rust and actual JNI regressions, native adapter
   persistence tests, frontend mode/restore/error tests and provider copy.

## Verification

- Focused checks cover missing/new mode defaults, switching and restoration,
  isolated keys/proxies, original-only, language intersection, request setup,
  incremental/final transcription, stale generations and shutdown.
- Inspect the signed `/Applications/mimi-dev.app` preview using credential-free
  fixtures before complete acceptance. Keep real-provider latency/quality
  separate from synthetic protocol and native UI evidence.
- Run the canonical `./scripts/check.sh`, shared architecture boundary and
  actual JNI suite. Review the completed diff independently before delivery.

## Protocol references

- [OpenAI realtime transcription](https://developers.openai.com/api/docs/guides/realtime-transcription)
- [Gemini Live transcription](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe)
- [Azure realtime audio](https://learn.microsoft.com/en-us/azure/ai-services/openai/how-to/audio-real-time)
