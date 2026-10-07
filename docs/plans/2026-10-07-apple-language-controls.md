# Apple language controls across settings and the floating panel

Settings previously disabled every language field with the credential editor
during active or paused subtitles, while the floating and tray panels already
allowed session language changes. Its blanket instruction to stop subtitles
therefore contradicted the other surfaces.

- Keep configuration and credential edits locked while a session owns them.
  Enable the active profile's language controls under the same connecting,
  stopping and in-flight action guards as the floating panel.
- Use the existing source/target session switch commands during active or paused
  sessions. They validate native resources before saving, publish the selection
  to all windows, reconnect live sessions and keep paused sessions paused.
  Do not bypass this lifecycle by writing listening preferences directly.
- Subscribe language controls only to whether an active or paused session owns
  language changes. Replacing subtitle drafts or latency samples must not render
  settings again. Preserve the high-frequency snapshot/credential-draft regression
  for both active and paused sessions, and verify command routing after a mounted
  control enters and leaves those states.
- Apple Speech can preview uninstalled languages, but downloads remain explicit
  and require a stopped session. Only show that restriction beside an actual
  resource preparation action; installed languages must not carry it.
- Apple Translation keeps its installed-resource preflight. A rejected language
  pair leaves the previous selection intact and reports the existing localized
  error, without silently downloading or modifying credentials.
- Re-enabling translation from Original must resolve the selected text route
  before validating the proposed live configuration. Do not use a connection
  probe of the old target: Apple probes preserve Original and validation removes
  its text-translation marker, incorrectly reporting missing credentials when
  the proposed target translates. Resolve Apple's keyless marker directly and
  validate before saving, using the same boundary for other independent routes.
- Display localized language names in resource status text, using the same name
  table as the picker. Locale identifiers remain native protocol data.

Verification should cover active and paused source/target changes, lifecycle
transitions, missing-resource rejection, name/status presentation, and disabled
configuration editing. Keep automated/component checks separate from signed
native and actual audio evidence.
