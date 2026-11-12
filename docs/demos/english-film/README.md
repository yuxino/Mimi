# Alibaba live-translation regression fixture

`response.json` is retained because `the_real_alibaba_capture_pairs_every_utterance_by_identity` in `shared/mimi-runtime` replays it to check event identity and source/translation pairing. Mimi does not load it in production. It is public test data, not application telemetry.

This response was captured from a public film excerpt for the retired September 2026 demonstration. The promotional video and its documentation are no longer part of the README. Do not delete or rewrite the original response and receive offsets when cleaning up demonstration assets.

Film: **Sintel**, © Blender Foundation, [sintel.org](https://www.sintel.org), [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/). [Source film](https://download.blender.org/demo/movies/Sintel.2010.1080p.mkv). The fixture contains recognized English dialogue and its Chinese translations from a 31-second excerpt beginning at 02:07. No endorsement is implied.

The source movie hash is retained in `response.json`. Preparation source: `b048092cbe76d5d4fb5836400c7e377a1e3e273e`; source preparation run 34038261225, artifact 9990866707; response capture run 34040075430. The retired recorder and presentation harness remain only in Git history and are not current application acceptance tests.
