# Readable initial translated previews

Keep intermediate captions without adding a switch or waiting for final speech.
The public-video native frames show one-word fragments replacing a complete
translated sentence, followed by further whole-draft translations. Bottom
placement removes a separate geometry jump but cannot fix premature MT input.

Honor the high-quality client's existing configured opening length threshold
before its first speculative request from the stability timer. Its non-resetting
maximum timer still permits a short draft, with an opening wait of at least two
seconds (longer configured maximums stay unchanged). Thus a missing final or a
short isolated utterance cannot starve display. Both draft timers use the whole
eligible draft; never cut a punctuation prefix or invent a durable final.
An already waiting/in-flight preview or consumed candidate permits shorter ASR
corrections, so a changed recognition can still repair the display. An actual
source boundary retires eligibility through the existing pacer's candidate reset;
it must not refund HTTP spacing or rate-limit suppression. Authoritative finals,
same-language passthrough, original-only recognition and Stop completion retain
their existing paths.

This rule belongs in the shared Rust runtime and reaches desktop and Android.
Cover both timers, short finals, corrections, ownership reset, punctuation,
Unicode and complete-tail preservation in direct Rust and actual JNI fixtures.
Rerun the same long public-video segment in all three modes and inspect native
frames. A reduced fragment count does not establish semantic accuracy or prevent
every revision of an incomplete sentence; record those limits explicitly.
