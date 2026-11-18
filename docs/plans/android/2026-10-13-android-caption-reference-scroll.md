# Android reference-line continuity

## Observed boundary

A fixed native ASR + MT display sequence reproduced an unchanged original moving backwards: when the translation grew, the original viewport dropped a line; when the translation shortened, that older line reappeared. The translation bottom edge and repeated identical publications were already stable. This is a native view-budget issue, not evidence of a provider or shared subtitle-state failure.

## Smallest change

While following live bilingual captions, retain the smallest source-line budget seen for the same original text and viewport size. A translation revision can require fewer source lines, but cannot reveal an older source line again merely because it frees space. Keep the current measured glyph budgeting, compact bottom alignment and translation budget.

Reset on changed original text, width/height, appearance or mode changes, deliberate history reading and return to live. Store only the current bounded original and viewport metadata; add no history, network behavior or provider-specific policy. Single-language modes retain their existing layout.

## Verification and limits

Four JVM regressions cover refinement, changed text, resizing and explicit reset. The native `asr-mt-differential-v1` probe compares the same ten frames before and after, checks actual draws for full glyph visibility and fixed translation bottom, and rejects backwards movement of the first visible line when the original is unchanged. Additional native cases exercise text, height, font, display mode, history and return-to-live resets.

This does not pin the original baseline: its vertical position still follows translation height. It does not prevent legitimate translation revisions, and synthetic native evidence does not establish cloud timing, real audio or physical-device reading comfort.
