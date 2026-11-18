# Per-profile recognition and text translation proxies

**Goal:** Save separate recognition and text translation routes with each service profile.

**Architecture:** Optional profile proxy fields inherit the legacy global proxy when absent. Saving one field materializes only that preference. Catalog validation normalizes addresses and rejects authenticated proxy URLs before disk writes. Profile mutations use the existing stopped-session lifecycle guard and catalog transaction; no credential reads or migrations are needed for persistence.

## Implementation

- Add optional speech/text proxy metadata, IPC draft fields, and resolved immutable session configurations. Preserve old preferences for migration and existing configurations.
- Route recognition WebSockets through the speech proxy, independent HTTP translation through the text proxy. Integrated realtime providers use their single speech route. Both readiness checks match their actual stage.
- Move network settings into profile details with two compact labeled selectors for independent pipelines and one integrated selector otherwise. Keep explicit save, errors, lock during active/paused sessions, and help tooltips. Reset drafts on profile switches; discard stale check results only for changed routes.
- Update Chinese, English, Japanese copy and document the desktop-only feature.

## Verification

- Cover absent-field inheritance, normalized independent preferences, restart/profile switching, invalid-input and persistence rollback, immutable session snapshots, stage-specific checks, and actual distinct network routes with local synthetic endpoints.
- Run the canonical repository check. Review the final diff separately for credential access, lifecycle races, misleading integrated controls, and stale results.
- Rebuild and inspect the canonical signed dev app. Preserve real account settings and capture preferences; report live verification limits.

## Acceptance follow-ups

- Add explicit paste actions to credential/address/model inputs while keeping native keyboard paste. Read the clipboard only on a click and keep the value in the unsaved draft; ignore a late read after switching profiles/providers. Use the official Tauri clipboard plugin for native desktop text reads, scoped to the settings window only. This small local dependency avoids WebView clipboard permission/gesture failures without exposing clipboard writes or other windows. Browser previews retain the standard Clipboard API and a safe keyboard fallback.
- Expose eye controls for saved recognition and independent translation keys through the existing scoped reveal command. Keep reveal separate from replacement drafts and clear plaintext on blur, hide, deletion, or provider change.
- Add a skip-translation switch for services that support recognition-only mode, backed by the existing persisted `original` target. Restoring translation uses the previous target in the current editor, or a supported target after restart. The backend already suppresses MT in this mode; integrated providers without recognition-only support do not offer the switch.
- Add compact language guidance for known single-language versus mixed/unknown audio, based on provider documentation; do not promise uniform accuracy gains.

## Retained transport policy

`direct` explicitly disables automatic HTTP and WebSocket proxies. `system`
reads the standard environment settings and supported static system HTTP/HTTPS
settings through the shared system matcher. It does not execute PAC scripts;
macOS SOCKS-only system settings are outside that matcher's system integration.
`custom` supports credential-free `http://`, `socks5://`, and `socks5h://`
endpoints. HTTP proxies tunnel WebSockets with CONNECT. SOCKS5 performs local
DNS; SOCKS5h sends the destination hostname to the proxy. HTTPS proxy URLs,
proxy authentication, arbitrary paths, queries, fragments, port zero, and
oversized endpoints are rejected. Unused custom addresses are removed when
switching to system or direct.

Proxy endpoints are preferences, never additional provider credentials. No
process-global environment is mutated. Existing destination TLS verification
and provider authorization remain in place; HTTP redirects are disabled so
an endpoint-specific captured route cannot silently migrate to a different
destination. Unsupported/authenticated matched proxy routes fail closed.
Error labels and Debug representations omit proxy hosts, authentication, and
provider payloads. Support diagnostics must not include proxy addresses.


Recognition and text stages capture their own immutable routes. Reconnects construct new clients and may read updated OS system routing. The old global preference remains only as a compatibility fallback for absent profile fields.

## Local verification

- Canonical check passed after the floating controls and restored layout: 953 Rust tests passed (one existing ignored test). The final frontend regression passed 946 tests across 87 files, strict clippy, lint, typechecked build, signing recovery checks and diff checks.
- Synthetic transport regression confirmed recognition CONNECT and translation HTTP reach different local proxies. Catalog tests cover legacy fallback, per-profile restart/selection, invalid URLs and write failure rollback.
- Rebuilt and launched the canonical signed macOS dev app. The real settings detail showed separate recognition/text routes, the saved recognition eye action, the skip switch and compact source-language guidance. The existing signed requirement remained identical.
- The user resumed a real listening session during native inspection. Clipboard/reveal isolation and state transitions have automated coverage; the new native paste action and every provider/proxy combination were not exercised with real credentials. No release app was replaced and no publication is claimed.

## Floating control acceptance follow-up

Expose the same skip-translation preference in the floating control panel for recognition-only capable profiles. Use a dedicated validated target-switch command rather than the stopped-session settings save. Reuse the guarded source-switch reconnect flow, update the immutable active configuration before releasing its guard, preserve paused state, and let newer pause/stop intents supersede reconnect. Normalize Audio3-only source hints when returning to a smaller recognition catalog. Remember the last translation target while the control window remains open, including panel dismiss/reopen; after restart use a supported fallback. Only the overlay control permission exposes this new command. A custom speech session that began in Original mode resolves and validates missing text credentials before saving a translated target; credential failure keeps its prior preference and active configuration. Existing MT suppression for Original remains unchanged. No provider wire changes or new Android capability are introduced.

## Text visibility and source colors

The user requested removal of the text-opacity switch on October 4. Remove its floating-panel row, three-language text, persisted preference and IPC/save plumbing. Product timelines use their existing default text styling. Legacy preference files with the old property are ignored without resetting other settings. The debugger replay retains its internal fixed-opacity rendering option for readable inspection; this is not a product preference.

Give existing system and microphone subtitle lanes separate persisted colors using the existing palette/custom RGB controls. Preserve the system color; default microphone to the existing yellow preset. Source labels and both bilingual lanes use their input's color, including committed history and streaming drafts. This changes rendering only; capture, queueing, subtitle/history content and credentials are unchanged. Restore the original two-row Live Subtitles and Immersive Mode group at the top of the subtitle settings, recovered from the parent of `a6aa41d`. Live Subtitles directly starts/stops the session and paused sessions have their previous explicit resume action. Preserve the existing lifecycle coordinator and credential-aware disabled states. The service settings remain passive. The user asked to restore the old subtitle settings appearance; independent profile/language/network changes remain in their current sections.

Desktop-only presentation controls are intentional; no Android behavior is implied. Regression coverage verifies legacy defaults, persistence without provider configuration changes, source/rendering isolation and live floating controls.

The restored subtitle-session controller remains mounted while other settings categories are visible, so category navigation cannot clear a pending start/stop action or admit duplicate clicks. Service settings stay passive. Legacy resume/race tests were restored alongside the original controls.
