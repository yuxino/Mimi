# Switch saved desktop service configurations during a session

## Decision

Allow selection of an already saved service configuration from Settings, the
tray panel and the subtitle control panel. Selection uses the existing lifecycle
lock and generation-checked reconnect path. Editing credentials, endpoints,
models, names or proxies, adding and deleting configurations still requires Stop.
A model can be prepared in a separate configuration, then selected while listening.

Merely enabling the old selection command would persist a new profile while the
live connection and paused resume snapshot still used the old one. Automatically
stopping and starting would instead discard session state and alter pause and
recording behavior. A validated reconnect preserves the current session boundary.

## Behavior and boundaries

- Validate the proposed configuration using its normalized listening preferences
  and private credential store before changing the selected profile. Keep Original
  mode where supported; a connection probe's forced translation target is unsuitable.
- Serialize persistence and the owned runtime configuration update with start,
  stop, pause, recovery and other switches. Reject connecting/stopping or busy
  lifecycle transitions; a later stop/pause supersedes the pending reconnect.
- Listening reconnects with the new profile. Paused remains paused and resumes
  with the new profile. Idle/error selection does not start capture. Selecting
  the current profile is a no-op.
- Confirmed subtitles, selected audio inputs, capture scope and recording consent
  remain intact. Recognition is briefly interrupted, and unfinished subtitles
  can be lost. Reconnection failure remains visible in the session error state.
- Recording uses one PCM sample rate per archive. Reject a switch to a different
  rate while recording is enabled, including a paused recording session; tell
  the user to stop first. Same-rate switches retain the recording.
- Use the existing compact selectors and feedback mechanisms. Put the reconnect
  explanation in help tooltips. Prevent repeated actions while a switch is pending.
  Show sanitized selection failures and leave the current selection unchanged when
  preflight or persistence fails. Languages follow the new service's capabilities.

## Verification

Focused backend tests cover proposed configuration resolution, Original mode,
validation and write failure, paused resume state, busy/superseded lifecycle
requests and recording sample-rate protection. Frontend tests cover settings,
both panels, duplicate actions, pending state, stopped/paused/live selection and
sanitized failure feedback. Run the canonical repository check and signed macOS
UI-only smoke. UI fixtures do not establish real provider reconnection latency
or real-device capture continuity. This change is desktop-specific; Android
selection behavior and shared provider/subtitle contracts are unchanged.
