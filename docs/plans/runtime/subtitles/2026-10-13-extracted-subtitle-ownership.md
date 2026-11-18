# Extracted subtitle ownership repairs

Extract the correctness fixes from #261 into the Alibaba preview branch, on top
of current main. Keep the current independent-MT preview schedule, committer
policy and version; do not carry the six-second complete-sentence rescue policy,
1.5.25 packaging or native capture fixtures.

- Preserve the numeric ASR layout owner when its HTTP translation confirms, so
  original-only confirmation and timestamp metadata keep the same sentence.
- Preserve Volcano START/END ownership: a late final for A becomes durable without
  clearing or replacing the newer B draft. Source order crosses the existing
  shared event/reducer and JNI path, including coalesced queues and Stop.
- Apply proto3 implicit zero defaults to omitted START/END scalar timestamps,
  while RESPONSE fragments remain unowned until a real START proves identity.
- Keep bounded ownership metadata, retired-boundary guards, Clear/reconnect
  isolation and the compatible conservative path for missing START.

Sources: raw-owner portion of `ed345c26`, ownership changes in `f9201391`,
zero-default changes in `1ae919cf`, and its equivalent JNI hex-iterator fix
`6c7193cd`. Adapt only necessary context to the latest Alibaba protocol and
current core regressions.

Verification uses direct Rust wire/client/controller/reducer tests and actual
Android JVM JNI timing/projection contracts. Existing #261 native recordings
describe their older revisions; this extraction does not claim fresh native
service acceptance or authorize capture, paid requests or publication.

Extraction verification: the canonical check passed on the current-main
combination, including shared Rust, actual host JNI, desktop Rust, frontend and
production-output checks. Android Debug JVM JNI verification passed 35 tests
across NativeRuntimeJniTest, SharedTranslationContractTest and
RuntimeSubtitleProjectionTest, including all 23 binary timing fixtures. No
provider request, audio capture, app installation or release occurred in this
verification.
