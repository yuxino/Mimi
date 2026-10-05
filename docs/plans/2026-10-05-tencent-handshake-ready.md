# Tencent handshake readiness with `final: 0`

## Confirmed failure

The actual Tencent realtime speech translation handshake was probed using the
existing local service profile. HTTP 101 arrived at **124 ms**; the first JSON
message arrived at **154 ms**, with `code=0`, `final=0` and `has_result=false`.
Only timing and these fields were retained in this record. No credentials,
account/session identifiers, signed URL or raw provider response are included.

The [official WebSocket guide](https://cloud.tencent.com/document/product/1093/127565)
shows a ready response without `final`. The real service also sends an explicit
zero. At source baseline `962c2480`, the Rust decoder classifies that no-result
response as `Ignored`; Android returns no event for the same shape. Both miss
readiness. Desktop then reaches its 5-second setup timeout despite the successful
upgrade and prompt service response. Android shares the decoding defect, but its
generic readiness timeout is 20 seconds; it was not measured by this probe.

This handshake evidence does not establish audio recognition, visible subtitles,
translation quality, billing eligibility or sustained-session stability. The
[earlier Tencent repair](2026-10-05-tencent-realtime-repair.md) remains a separate
record of configuration guidance, structured errors and audio-tail handling.

## Accepted repair boundary

- In both Rust and Android, treat `code=0` with no `result` and either absent
  `final` or integer `final=0` as ready.
- Preserve nonzero provider rejection handling, `final=1` session completion,
  result-bearing transcript/final-pair decoding and unknown final-value handling.
  Keep the existing bounded parsing and sanitized errors.
- Add the explicit-zero ready response to the shared
  [`translation-contracts.json`](../../shared/translation-contracts.json) fixtures
  consumed by both platforms. Retain omitted-final readiness and neighboring
  result/completion/unknown cases so accepting this handshake cannot swallow
  subtitle events or session endings.
- Add a desktop loopback handshake regression that becomes ready before audio
  is sent. Android must consume the same ready fixture through its decoder.
  Credentials, signatures, endpoints, audio formats, setup deadlines and
  automatic recovery policy are outside this change.

## Verification status

- The old signed app timed out after 5,195 ms for Japanese → Chinese through the
  system proxy. The patched canonical signed app, built from `962c2480` plus the
  uncommitted Tencent fix, passed the same check with the same credentials in
  222 ms.
- With system audio only and microphone off, English → Chinese playback of a
  15.091-second synthetic sample in QuickTime produced two confirmed bilingual
  groups in the native overlay through the final sentence. Pause, resume and
  replay added two more groups, for four total. Observed RTT was 16 ms initially
  and 12 ms after resume; these are not end-to-end subtitle latency. Diagnostics
  recorded resume connecting at 299.3 s, listening at 299.7 s and stop at 359.2 s.
  The black background remained, with immersive mode off and the transparency
  setting unchanged at 23. The session was stopped and Japanese → Chinese restored.
- The new Rust handshake test failed against the old decoder; all 21 focused
  Tencent tests passed after the fix. Canonical `scripts/check.sh` passed: Rust
  1,178 / 2 ignored, shared core 72 plus JNI, frontend 1,749 across 121 files,
  formatting, Clippy, typecheck, lint and production build. Android passed all
  135 tests across 21 suites, including two JNI tests. A final test-only change
  uses a oneshot result to expose spawned-server assertion failures; the 21
  focused Rust tests, strict Clippy, formatting and diff checks passed again.

This verifies the observed desktop connection, subtitle and pause/resume run.
Sustained stability and Android hardware behavior remain unverified; the run
does not establish billing eligibility or a general latency benchmark.
