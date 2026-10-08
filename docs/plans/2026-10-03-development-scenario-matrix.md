# Development Scenario Matrix Implementation Plan

**Goal:** Run repeatable, independent ASR baselines across a tagged public-media scenario matrix and compare each with private Mimi dev evidence.

**Architecture:** Prepare licensed media in parallel, then compile the manual Rust test executable once and run each scene in its own process with bounded concurrency. Copy validated PCM, audition WAV and reference into a private case control directory so child manifests remain self-contained. System-audio playback into signed Mimi dev remains sequential; simultaneous playback would corrupt source attribution.

**Tech Stack:** Python standard library, existing Rust manual Audio3 benchmark, macOS audio tools/frameworks, signed Mimi dev and the offline evidence analyzer.

---

The user has chosen parallel subagent work in this session. The first six cases remain an immutable seed corpus; new movie/animation, language, speed and BGM variants carry separate scene labels and provenance. We do not infer a product defect from an aggregate edit score alone.

### Task 1: Validate and stage scenes

**Files:** Create `scripts/run-development-batch.py`, test `scripts/run-development-batch-test.py`.

Use this input shape, with at most 32 scenes per batch:

```json
{"schemaVersion":1,"scenes":[{"label":"ja-slow-001","pcmPath":"/absolute/clip.pcm","wavPath":"/absolute/clip.wav","referencePath":"/absolute/clip.txt","asrLanguage":"auto","unit":"char","expectedSpeech":true,"tags":["language:ja","human-read"],"provenance":{"license":"CC-BY-4.0","sourceUrl":"https://example.org/source","referenceStatus":"official_transcript_not_independently_audited"}}]}
```

Validate regular non-symlink input files, size limits, mono PCM16/16k WAV and exact PCM equality before any network request. Negative controls use `referencePath:null` and `expectedSpeech:false`; their output is unexpected-transcription counts, not an invented zero-denominator CER. Tagged artificial speed/BGM fixtures retain their parent identities and measured transformation parameters.

Write a test which supplies a mismatched WAV and PCM and asserts preparation fails before invoking an executor. Cover input boundaries, duplicate labels, source preservation and empty-reference negative controls. Run `python3 -B scripts/run-development-batch-test.py`, initially failing on the missing module, then implement the validator and staging. Only `--run` authorizes actual provider work; the default prepares files without credentials, compilation or network.

### Task 2: Execute isolated jobs with bounded concurrency

**Files:** Continue `scripts/run-development-batch.py` and its tests; update `scripts/check.sh`.

Build with `cargo test --features development-debugger --lib --no-run --message-format=json`, select the returned test executable and call only `audio3_benchmark::manual_same_pcm_asr_comparison --ignored --exact --nocapture`. Each child receives its own manifest, private output directory and fixed baseline/bypass options. No credential values are read or injected by Python; the selected ordinary development profile supplies its saved local-file credentials.

Default to three jobs, maximum four. Hold one batch lease to prevent accidental parallel invocations from multiplying concurrency. Bound compiler and job logs, retain failure artifacts, terminate children on cancellation, and atomically update a content-free progress report plus analyzer-compatible baseline index. Unit tests use fake executors to establish the actual concurrency bound, failure isolation, completed-index publication and cancellation behavior without paid requests.

Add the offline test to `scripts/check.sh`. Run focused checks first, then the complete repository check once implementation stabilizes. Commit the working scheduler with its validation results.

### Task 3: Expand and compare real scenarios

**Files:** Private media/provenance, matrix and reports outside Git; maintain `docs/development/debugger-evidence.md` for reusable commands.

Parallel owners prepare different animation/movie dialogue and no-dialogue controls, additional Japanese/Korean/Chinese voices and duration ranges, pitch-preserving speed variants, and labelled RMS-controlled BGM mixtures. Record fixed selection rules, license, hashes, actual source/reference quality and duration; preserve useful downloads and build caches.

Run baseline jobs after validation. For each scene, create a new dev evidence case, wait for actual successful audio sends, play one verified WAV, wait for final tail, stop and seal the case. Score raw ASR, exactly admitted pairs and confirmed history with the same normalization. Review timestamps, actual sent PCM and same-snapshot frontend projection for any suspected loss. Translation semantics and readable glyphs require separate review; text counts are insufficient.

Agents cross-check findings and prioritize a reproducible failure. Make a product repair only when the owning stage and invariant are established, then rerun the same scene and preserve both cases. Silence, music, speech overlap, pause/resume, collapse/expand and reconnect have explicit expected behaviors; incomplete traces remain unknown. Persist each completed corpus and report privately so a reboot does not erase the baseline.

### Task 4: Keep expanded evidence batches bounded and independently reopenable

The first expanded native run reached the default store's 128 MiB reservation
limit after 11 new cases, with about 89 MiB of actual files. Preserve them.
An explicit dev workspace selector chooses a named private sibling catalog;
the original catalog stays the default. Each catalog retains its existing
128 MiB and 64-case bounds, with at most eight extra catalogs. Production
ignores the selector. Validate names and reject symlink/non-directory roots
before writes. Record the workspace in route metadata and display it in the
debugger. No credentials, preferences or opt-in state are copied or changed.

Add focused pure validation and filesystem-boundary tests, then run the full
check and canonical signed dev launcher. Native automation must confirm a new
active case ID before starting capture, and require the previous case's trace
and audio recorders to be sealed before playback. A refused recording remains
a failed preparation; it must never be scored against an old case ID.
