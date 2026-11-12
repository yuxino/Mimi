# Windows audio observations by caller

The former census built endpoint and application-session metadata together.
Default-output following used only endpoint IDs and levels, while application
discovery used only session process IDs. The public census IPC is retired.

Keep two fresh projections over the same safe COM device traversal:

- Endpoint levels: read the raw WASAPI endpoint ID and its peak meter. Do not
  enumerate sessions, read device properties, open processes or inspect session
  state/meters. Return the selection candidates directly, avoiding another ID
  clone and vector conversion.
- Session process IDs: enumerate each active render endpoint's existing sessions
  and query their owners. Do not read endpoint IDs/names/levels or session
  state/levels/process names. The application picker already owns process
  identity, names, deduplication and the 512-item result limit.

Preserve native enumeration order and include session owners even when silent,
inactive or hidden in the tray. Keep role priority, audible thresholds,
selection watermarks, fallback and raw-ID/CPAL mapping unchanged. A failed
endpoint or session retrieval skips that item, and failed level reads remain
zero. Observations are not cached across device/driver changes. COM allocation
ownership stays with the existing endpoint-ID helper and thread-local apartment
owner.

Retain the real Windows endpoint/finite-level regression and make the raw-ID
mapping regression consume the actual endpoint projection. Do not retain
serialization or tests for the removed display-only census fields, and do not
fake a second COM implementation to manufacture platform coverage. Windows CI
must compile and run its existing native audio regressions; this macOS change
does not establish Windows timing gains or physical-device acceptance.
