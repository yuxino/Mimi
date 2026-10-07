# Alibaba text translation model selection

Add a model picker to the desktop Alibaba text-translation stage when its route
follows the built-in service. Offer Qwen-MT Lite, Flash and Plus, with Lite as
the backward-compatible default. Persist the choice as non-secret profile
metadata, including the local-development preset; the preset's credentials and
service type remain read-only. Independent text destinations keep their existing
model configuration.

Require subtitles to be stopped before saving a model change. Save quietly,
report failure through the shared settings toast, and invalidate the text check
after success. Live sessions and explicit text connection checks must resolve
the same saved model. Include the model in the request-budget scope so switching
models cannot reuse accounting for a different destination.

Keep Audio 3.0 recognition, preview pacing, final ordering and the existing Lite
language range unchanged for comparable model trials. Flash and Plus's wider
language catalogs are not exposed by this change. Lite and Flash stream results;
Plus uses non-streaming translation for both previews and finals, including
connection checks. Do not route Qwen-MT through the generic chat prompt.

Verify legacy defaults, per-profile persistence, development-preset persistence,
provider validation, session/probe model propagation, stream selection, stopped
and active UI states, failed saves and signed native visibility. Live translation
quality remains a user trial; opening the app is not provider acceptance.

This is a desktop configuration feature. Android retains its Lite default and
has no model picker; existing shared provider wire contracts remain unchanged.

For the requested model comparison, show concise parenthetical model positioning
and return style in all seven UI languages. Positioning follows Alibaba's model
selection guide; a quality-oriented label does not assert that Plus wins every
sample. Keep timings out of the permanent option text because they depend on
network, length and the particular run.

Add an explicitly invoked, ignored, macOS-development-feature-only comparison
beside existing private live checks. Reuse the production Qwen client and its
unchanged model-specific hints/terms. Accept at most six English/Japanese synthetic
inputs, two rounds and three models (36 serial requests), rotate model order and
reuse connections per model/source. Record first lexical callback, completion,
nonempty callback count and sanitized status; write source/results only to opted-in
private 0700/0600 files outside Git. This is text-only model evidence, with no
capture, subtitle recording or claim about end-to-end latency.
