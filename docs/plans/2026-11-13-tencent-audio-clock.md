# Tencent audio clock and slow writes

Relative 200-ms deadlines accumulate timer overshoot and can make Tencent's
sender fall behind real-time capture. Anchor each deadline to the preceding
scheduled time, while discarding missed periods after a slow write.

Update the next deadline only after the WebSocket write succeeds, using the
later of `scheduled + 200 ms` and the write completion time. This matches the
existing Baidu client. Recording it before the write leaves an extra expired
deadline after socket backpressure: the stalled frame can then be followed by
two immediate frames. Completion-based pacing permits only the next frame to
be immediately due, then resumes the 200-ms clock. It does not promise a hard
200-ms minimum between every pair of network arrivals.

Retain the existing async lock order (`audio_send_lock`, frame deadline, sink)
through the write. Reconnect resets the deadline after replacing the sink, so
an older send cannot publish its deadline after that reset. Cancellation drops
the guards; failed writes do not advance the clock. Frame sizes, queue limits,
finish behavior and generation/readiness checks stay unchanged.

Keep the simulated timer-overshoot checks and normal loopback pacing test.
Also fill a local WebSocket's write buffer with a synthetic fixture message,
pause its reader for 500 ms, and send three PCM frames through production
`send_audio`. Once reading resumes, the third frame must not immediately follow
the second. This exercises socket backpressure without provider traffic or a
production test hook; it does not establish a Tencent service rejection.
