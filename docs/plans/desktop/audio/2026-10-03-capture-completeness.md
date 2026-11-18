# Preserve complete macOS audio buffers and channel layout

The user accepts any justified approach that improves original-language
recognition, including capture correctness, conversion and recognition, rather
than requiring denoising alone. Keep translation acceptance independent.

## Confirmed capture defects

Before this change, the macOS callback read only `lengthAtOffset` bytes returned by
`CMBlockBufferGetDataPointer`. Apple's contract permits this to be smaller than
the complete buffer when it spans multiple memory blocks. The remaining bytes
are silently excluded from decoding. Planar channel data or an individual PCM
sample can also span that boundary. This is a reproducible API-level loss risk;
it is not proof that the user's reported session had this buffer layout.

Use the full `CMBlockBufferGetDataLength`. Retain the zero-copy path only when
the returned pointer covers the complete range. Otherwise copy that full range
with `CMBlockBufferCopyDataBytes` before decoding. Keep copies callback-local,
bounded to 4 MiB, and report allocation/read/size failures through the existing
content-free capture-processing failure path. Do not retain audio after the
callback or change sample rate, channel mixing, credentials or ASR requests.

The decoder also tests bit 6 as the non-interleaved flag. Core Audio defines
non-interleaved as bit 5; bit 6 is nonmixable. Therefore valid planar multichannel
PCM is decoded as interleaved, and a nonmixable interleaved format is decoded as
planar. Use the imported `kAudioFormatFlagIsNonInterleaved` constant. Preserve the
same per-frame channel average, but locate each channel's matching frame correctly.
The requested mono configuration is unaffected by this layout distinction;
the defect matters when a delivered format contains multiple channels.

## Verification

Construct actual CoreMedia buffers from separately owned blocks. Assert the
original pointer exposes only the first prefix, then verify the new reader
returns all bytes in order, including a PCM sample spanning blocks and the final
sample. Verify contiguous data takes the borrowed path, empty buffers remain
empty, invalid pointers are rejected before FFI, and the size limit is checked
before materializing storage. These native buffer fixtures require no Screen
Recording, microphone, provider connection or actual user audio.

Add frame-order regressions for planar float32, planar signed 16/32-bit PCM, and
interleaved PCM carrying the distinct nonmixable flag. Verify these fail against
the original bit test. A stereo fixture whose corresponding channels average to
0.5 must produce four 0.5 frames, rather than averages of adjacent samples within
each channel plane.

The pre-existing interleaved stereo fixture inherited a mono format's `0x29`
flags, which actually include non-interleaved. That mismatch was hidden by the
same incorrect decoder bit test. Give this fixture explicit interleaved flags;
do not change its expected channel-average values to accommodate the bug.

Run the targeted tests and strict/full repository checks, then build the signed
canonical development app and inspect diagnostics. A user's actual failing
excerpt is still needed for original-text accuracy acceptance. No lower reference
error rate or general noise/music improvement is claimed from fixing byte loss.

Primary API references:
[pointer range](https://developer.apple.com/documentation/coremedia/cmblockbuffergetdatapointer(_:atoffset:lengthatoffsetout:totallengthout:datapointerout:)),
[full-range copy](https://developer.apple.com/documentation/coremedia/cmblockbuffercopydatabytes(_:atoffset:datalength:destination:)),
[channel layout](https://developer.apple.com/documentation/coreaudiotypes/kaudioformatflagisnoninterleaved).

## Completed checks

The native buffer fixtures and decoder regressions passed all 10 focused tests.
Restoring the incorrect layout-bit test causes the three new layout tests to
fail. The canonical repository check passed: 970 Rust tests, 2 ignored manual
tests, and 946 frontend tests across 87 files, including strict compilation,
formatting, lint and frontend production build. Strict Clippy also passed with
the optional local-development credential feature.

The signed canonical development app was built and opened. During acceptance,
a later UI-only package from another worktree replaced that shared path and
removed the restored subtitle controls from the running UI. This is a package
mismatch, independent of these capture changes; rebuild the intended worktree
before interpreting native-session results.

The intended worktree was subsequently rebuilt and opened in normal live mode.
Accessibility inspection confirmed both real-time subtitle and immersive-mode
switches on the subtitle settings page; immersive mode remains disabled while
the session is idle. Native playback acceptance remains outstanding: the user
was operating the restored settings UI, so no automated playback or live-session
start was performed during that interaction. The fixture tests establish byte
and layout correctness, not end-to-end recognition completeness.
