# Build, runtime, and repository efficiency

## Status

Accepted.

## Context

Mimi opens several independent WebViews, moves small system-audio buffers
through provider-specific framing code, and validates both desktop platforms in
CI. Correctness and privacy boundaries already have strong coverage, so
optimization work must preserve those boundaries and demonstrate a concrete
reduction in loaded bytes, allocations, CI wall time, or tracked files.

## Decision

- Resolve the native window label from the shared entry point, but load only
  that window's React surface. Shared state initialization stays eager; window
  components and their surface-specific CSS are same-origin dynamic chunks.
- Frame buffered provider audio directly from the complete prefix of the
  pending buffer. Build owned wire messages before draining that prefix so
  encoder failure keeps the existing consume-on-attempt behavior without an
  extra whole-batch allocation and copy.
- Downmix and resample borrowed slices. Reuse pending-buffer capacity after
  successful encoding instead of replacing the buffer on every callback.
- Run ordinary branch and pull-request bundle jobs alongside tests. Every job
  remains required for overall success; tag publication retains its existing
  explicit test and preflight dependencies.
- Keep `npm run build` as the standalone typecheck-and-build contract, but do
  not invoke a separate TypeScript build immediately before the same command in
  the canonical check or CI.
- Keep short-lived CI bundles long enough for debugging but do not retain them
  as release history. Published Release assets remain the durable distribution
  surface.
- Keep only assets used by the supported macOS and Windows bundles, plus the
  editable source and conservative fallback icons. Generated Android, iOS, and
  Microsoft Store assets do not belong in this desktop-only repository.

## Non-goals

- Do not change provider wire formats, prompts, queue bounds, final-event
  ordering, credential handling, or diagnostics content.
- Do not trade release size against unmeasured build or runtime changes by
  adjusting LTO, optimization level, panic behavior, or dependencies.
- Do not delete behavioral tests merely because adjacent cases look similar.
- Do not rewrite Git history or count ignored Cargo/npm caches as shipped
  repository or bundle size.

## Verification contract

- Compare the shared-bundle baseline with the recursive JS/CSS dependency set
  loaded by each dynamic window entry on both configured WebView targets.
- Keep focused frame/tail tests for every provider and add capacity reuse
  coverage for the same-rate streaming path.
- Run the canonical repository check, the minimum supported Rust check, and a
  locked Tauri build after the final edit.
- Launch the stably signed `/Applications/mimi-dev.app` in UI-only mode and
  inspect every window surface that now loads through a dynamic chunk.
- Require the next main CI run to complete Rust, frontend, MSRV, and both
  platform bundle jobs; compare its job start times and wall time with the
  serial baseline.
