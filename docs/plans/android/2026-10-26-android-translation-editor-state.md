# Android editor state and saved-session navigation

The translation editor must preserve the user's current selection through
configuration changes and use it immediately when Save or Check is pressed.
The API 35 native regressions distinguish an explicit Custom choice on a known
Kimi endpoint from an inferred preset, and exercise route selection plus the
action in the same UI task before Spinner delivers its listener callback.

## State ownership

- The native Spinner's current route is authoritative. Reconcile it through one
  transition before listener handling, draft snapshots, validation and checks.
  Preserve the previous route's typed fields before loading the next route;
  invalidate obsolete check work and results through the existing generation.
- Explicit Custom is user intent, not an unknown endpoint. Retain this flag in
  the existing in-memory editor draft, independently of endpoint inference.
  Changing a model or recreating an activity must not silently re-enable a
  vendor's setup actions after the user chose Custom.
- Draft secrets remain in the existing RAM-only ViewModel. Do not serialize
  keys or draft content into the Android saved-state Bundle, persistent
  preferences, logs or a second credential store. Cancel discards the draft;
  Save keeps the existing provider/profile-scoped credential behavior.

These are native editing rules. Provider requests, model IDs, authentication,
translation scheduling and subtitle policy remain in the shared runtime.

## Saved-session reading and export

Track a displayed page by session ID and page number. Only a changed page
returns its reading viewport to the top after native layout. Refreshing the
same page, publishing progress or showing export feedback must preserve the
reader's position. Guard deferred layout work against a newer page or a closed
Activity.

A successful document-picker result is an accepted export request even if a
list/page refresh is still busy. Retain at most one pending request containing
only archive ID, format and destination URI. Run it on the existing single
worker after the current operation, and reject a duplicate without overwriting
accepted work. Closing the reader must enqueue accepted work before worker
shutdown; it still writes the destination and cleans its private temporary file
without retaining or notifying the closed Activity. Shared Rust continues to
own archive content and export formatting.

## Shared connection feedback

DeepL and DeepLX rejection 429 maps to the existing concurrency-limited reason;
456 maps to quota-exhausted. Keep authentication, transport, timeout and other
rejections distinct. This classification belongs in the shared runtime and is
verified through shared cases, direct Rust and actual JNI probes. Android only
adapts the fixed diagnostic reason to an existing localized message; request
fields and translation prompts do not change.

## Regression boundaries

The focused native cases use synthetic unsaved fields. Save is stopped by a
required empty name and Check by a missing field, before storage or network.
They also assert that switching back retains the previous route's draft and
that no draft secret appears in saved state. Existing recreation, profile,
compatible-service and connection fixtures remain part of acceptance.

Saved-session regressions use synthetic archives and real system-input paging.
A bounded worker barrier makes refresh and the actual document result overlap;
both staying in the reader and closing it must write the requested file and
clean temporary output. No fixture starts capture or a provider session.

The debug application separately uses an untranslatable `debug_app_name`
referenced by its debug-only manifest. Every locale must identify the installed
development application and its launcher as `mimi Dev`; release names and
identities are unchanged.
