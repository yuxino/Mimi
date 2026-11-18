# Try Alibaba LiveTranslate simultaneous interpretation

The current development build starts new Alibaba configurations in
**Simultaneous translation** under
**Settings → Models & Services → Alibaba Cloud → Recognition and
translation**. Existing profiles keep **Recognize, then translate** until explicitly
changed; a workspace ID is required before LiveTranslate can be saved.

1. Use **Get workspace ID** and **Get API key** to open the official Beijing
   console and copy an existing workspace ID and its matching key.
2. Choose LiveTranslate in Mimi, fill the workspace ID, reuse or enter the
   matching key, save, and select the configuration.
3. Choose automatic recognition and a supported text translation target, play a
   short non-sensitive sample, and start subtitles. Stop afterward.

The configuration guide is linked beside the mode selector. Saving alone starts
no audio session. Switching back restores the independent MT configuration.

The API key stays in Mimi's existing private credential store. The workspace ID
is non-secret metadata. LiveTranslate 3.8 uses a workspace-specific host in
[the official setup guide](https://help.aliyun.com/zh/model-studio/qwen3-5-livetranslate-flash-realtime).
Missing IDs are rejected rather than guessed. This iteration does not offer
Singapore routing or an Android selection UI. See the user-facing setup steps:
[English](../provider-setup.md#alibaba-livetranslate) ·
[简体中文](../provider-setup.zh-CN.md#alibaba-livetranslate).

For a first user test, select system audio, play a short non-sensitive sample,
and stop subtitles afterward. Keep microphone capture, audio recording and
subtitle retention off unless deliberately testing them. A service check proves
setup acceptance only; it does not prove usable translated subtitles.

## Same-PCM manual probe

This probe is test-only on macOS. It never launches Mimi or changes its selected
configuration. The saved development profile must already be initialized and
use Alibaba with the default text route. Its loader is strictly read-only.
Obtain current approval for any possible API charges before executing.

Prepare one public or synthetic mono PCM16/16 kHz sample and a manifest using
the existing [ASR fixture schema](../plans/development/verification/2026-10-02-audio3-repeatable-benchmark.md).
Do not upload a private meeting recording.

```sh
MIMI_TRANSLATION_COMPARISON_APPROVED=yes \
MIMI_TRANSLATION_COMPARISON_ARM=liveTranslate \
MIMI_TRANSLATION_COMPARISON_WORKSPACE='<existing-Beijing-workspace-id>' \
MIMI_ASR_BENCH_MANIFEST=/absolute/public-fixtures/manifest.json \
MIMI_ASR_BENCH_PRIVATE_OUTPUT_DIR=/private/tmp/mimi-debug-benchmark/realtime-test \
cargo test --locked --manifest-path src-tauri/Cargo.toml \
  --features development-debugger --lib \
  audio3_benchmark::translation_comparison::manual_three_arm_translation_comparison \
  -- --ignored --exact --nocapture
```

Select `baseline` or `liveTranslate` individually. `all` explicitly runs
both serially and requires corresponding authorization. The ignored test's
historical `manual_three_arm_translation_comparison` name remains compatible
with existing invocations; it now accepts only these two arms. Missing arm means
baseline only. One clip is capped at 20 seconds with a fixed two-second silent
tail; there is no driver retry. Secrets must never be supplied through environment
variables. Cache/path variables in this recipe contain no credentials.

Results contain test subtitles in private 0600 JSON files, with the input hash.
Keep them outside Git. Analyze offline with:

```sh
python3 -B scripts/diagnostics/analyze-translation-comparison.py \
  --input /absolute/private/translation-comparison-ID/results.json \
  --output /absolute/private/new-comparison-analysis.json
```

Time zero is the first scheduled PCM frame after service readiness. Report first
accepted translated text, the first appearance of the final unchanged text
(retrospective stability), confirmation time, and replacement rewrites. Network
badge duration is not translation latency. Production preview channels may
coalesce updates, so rewrite counts are observed runtime changes, not exhaustive
wire counts. Audio-clock speech ends are not exposed here; sentence-end waiting
remains unavailable. A baseline source final arrives with the translated pair,
not at the recognizer's raw final time. Audio3 gets the manifest language hint;
LiveTranslate 3.8 requires automatic source detection. Review omissions,
incorrect translations, and readability separately against the fixed sample.

Both routes can charge. Public new-user allowances do not prove an account's
remaining quota. Check [prices](https://help.aliyun.com/zh/model-studio/model-pricing)
and [quota expiry](https://help.aliyun.com/zh/model-studio/new-free-quota) for the
actual account. Choosing **Original** changes what Mimi displays, but LiveTranslate
may still generate translation output and charge for it.

## Existing evidence

The [integration run ledger](integration-runs.md) records one LiveTranslate-only
run using 10.277 seconds of synthetic English audio plus the fixed silent tail.
From the first PCM frame, the first accepted translation appeared at 3,170 ms;
the complete final text first appeared at 10,990 ms and was confirmed at
12,993 ms. The approved budget and estimated audio charge are recorded there;
the actual account bill was not verified. The ASR + MT baseline was not run, so
this is not an A/B quality result. Offline checks, provider response, visible
desktop behavior, and comparative quality remain separate claims.
