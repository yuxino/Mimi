# MT stability research probes

Offline experiments for [the design](../../docs/plans/2026-10-13-mt-stable-units-research.md).
These files are not imported by Mimi, do not access credentials, start capture,
call providers, download models, or modify application preferences. Python is
for this experiment only; product policy must use shared Rust, not a sidecar.

## Reproduce the VAD host comparison

The recorded run used Python 3.9 on macOS arm64. Use a separate temporary venv
and `requirements-vad.txt`; other Python versions need a separate compatibility
check, not a silent replacement of recorded versions.

Download the reviewed model explicitly before running:

- Source: [Silero model at pinned revision](https://github.com/snakers4/silero-vad/blob/1e261b036686cd0017d500ee96acd1c4ba572a9d/src/silero_vad/data/silero_vad_16k_op15.onnx)
- SHA256: `7ed98ddbad84ccac4cd0aeb3099049280713df825c610a8ed34543318f1b2c49`
- Size: 1,289,603 bytes. Licenses and attribution: [NOTICE](NOTICE.md).

Reuse an existing `run-development-batch.py` matrix. Each `scene` needs a unique
non-sensitive `label`, `wavPath`, and `provenance.license` / `sourceUrl`. WAVs
must be mono PCM16, 16 kHz, at most 120 seconds, with at most 32 scenes. Paths may
be absolute or relative to the matrix. All inputs are validated before inference.
No reference text is read. Reuse existing private inputs; do not commit media.

```bash
python -B scripts/research/mt-vad-benchmark.py \
  --matrix /absolute/private/matrix.json \
  --model /absolute/private/silero_vad_16k_op15.onnx \
  --output /absolute/private/vad-400ms.json
```

Each output must be new and is created mode 0600. It contains numeric results,
sample hashes, public source/license metadata and supplied scene labels, never
WAV paths or transcript text. Inspect labels and provenance before publication.
Each source has fresh recurrent and hysteresis state. A last partial input frame
is zero-padded to 512 samples; there is no synthetic silence post-roll or EOF
final. Run 250/400/600 ms separately with `--silence-ms` for boundary sensitivity.
Do not compare timing runs executed concurrently.

The two hosts process identical 576-sample input windows (64 context + 512 new
samples). Direct ONNX probability uses the same hysteresis as sherpa's low-level
`VadModel.is_speech`. This bypasses sherpa's audio-retaining `VoiceActivityDetector`.
All frame decisions are checked for equality between hosts and across repeated
resets; equal decisions demonstrate this compatibility, not VAD accuracy.

Timing encloses the host call, including Python binding and hysteresis overhead;
it excludes reading WAV and assembling windows. The first whole-input pass warms
the model; three passes are measured. `initialization_ms` includes Python import
and model construction, so it is not a controlled app cold-start comparison.

## Source-candidate experiment

`mt-unit-policy.py` is an executable state-machine prototype exercised by the
tests. It proposes punctuation-stable units after 600 ms, allows a replaceable
preview at 2 seconds, and preserves a 1.5-second request budget. A confirmed
pause can release a stable partial preview sooner, but never freezes it. These
are experimental constants, not measured optimal settings.

Successful current responses may provisionally pin a source prefix. Later
requests then contain only the unpinned suffix. Corrections revoke the pin;
finals always replace it. Empty/failing/stale results, resets, independent input
sources, Unicode bounds, abbreviations, decimals and sentence-tail negation have
explicit cases. The prototype contains no MT model, translation-quality scorer,
target-text merger, audio-to-ASR timestamp alignment, or product adapter.
Its `paused` input must come from a correctly aligned signal before product use.
The dedicated `mt-stability-research` CI job runs these standard-library tests
without models, audio, third-party Python dependencies or provider credentials.

```bash
python3 -B scripts/research/mt-research-test.py
python -B scripts/research/render-mt-vad-results.py \
  --results docs/development/evidence/2026-10-13-mt-stability \
  --output /absolute/new/vad-comparison.png
```

The figure is a plot of measured offline results, not an application screenshot.
Keep accepted evidence; delete the temporary venv, downloaded model and scratch
files after this research run. No application dependencies or shared caches need
to be removed.
