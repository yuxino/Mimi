import { expect, it } from "vitest";
import { diagnosticAudioState, diagnosticNextStep, supportDiagnosticSummary } from "./supportDiagnosticSummary";

const report = (changes: Record<string, unknown> = {}) => JSON.stringify({
  schema: "mimi.support.v2", session_status: "listening",
  service: { api_round_trip_ms: 0, translation_duration_ms: 125, translation_duration_kind: "follow" },
  last_error: { classification: { category: "rate_limit", phase: "text_translation", code: "TRANSLATION_RATE_LIMITED" } },
  journal: { recent_events: [] }, ...changes,
});

it("keeps zero measurements distinct from missing data and preserves the translation measurement kind", () => {
  expect(supportDiagnosticSummary(report())).toMatchObject({ status: "listening", apiLatencyMs: 0, translationLatencyMs: 125, translationLatencyKind: "follow", lastError: "rate_limit", recentEvents: [] });
  expect(supportDiagnosticSummary(report({ service: { api_round_trip_ms: null, translation_duration_ms: 125, translation_duration_kind: "unrecognized" } }))?.translationLatencyMs).toBeNull();
});

it("ignores unsafe numbers and unknown report/status formats while leaving raw reports available to the caller", () => {
  expect(supportDiagnosticSummary("not-json")).toBeNull();
  expect(supportDiagnosticSummary(report({ schema: "mimi.support.future" }))).toBeNull();
  expect(supportDiagnosticSummary(report({ session_status: "raw provider failure" }))).toBeNull();
  for (const value of [-1, 0.5, "400", Number.MAX_SAFE_INTEGER + 1]) {
    expect(supportDiagnosticSummary(report({ service: { api_round_trip_ms: value, translation_duration_ms: value, translation_duration_kind: "request" } }))?.apiLatencyMs).toBeNull();
    expect(supportDiagnosticSummary(report({ service: { api_round_trip_ms: value, translation_duration_ms: value, translation_duration_kind: "request" } }))?.translationLatencyMs).toBeNull();
  }
});

it("shows only six recent typed events and never carries arbitrary strings into human-readable diagnostics", () => {
  const events = Array.from({ length: 12 }, (_, index) => ({ sequence: index, elapsed_since_app_start_ms: index * 100, kind: "status", status: "listening", ignored_raw_field: "private-path-or-text" }));
  const result = supportDiagnosticSummary(report({ journal: { recent_events: events }, last_error: { classification: { category: "private-error-text", code: "private-key" } } }))!;
  expect(result.recentEvents).toHaveLength(6);
  expect(result.recentEvents.map((event) => event.sequence)).toEqual([6, 7, 8, 9, 10, 11]);
  expect(result.lastError).toBeNull();
  expect(JSON.stringify(result)).not.toContain("private");
});

it("accepts lifecycle, recovery and failure facts but does not claim a retry without a typed scheduled retry", () => {
  const timing = { sequence: 1, elapsed_since_app_start_ms: 0 };
  const events = [
    { ...timing, kind: "lifecycle", action: "resume_requested" },
    { ...timing, kind: "recovery", action: "recovered" },
    { ...timing, kind: "failure", classification: { category: "authentication" } },
    { ...timing, kind: "translation_backoff", recovery: { reason: "rateLimited", retryScheduled: false } },
    { ...timing, kind: "translation_backoff", recovery: { reason: "temporarilyUnavailable", retryScheduled: true } },
    { ...timing, kind: "translation_backoff", recovery: { reason: "rateLimited" } },
  ];
  expect(supportDiagnosticSummary(report({ journal: { recent_events: events } }))?.recentEvents).toEqual([
    { sequence: 1, elapsedMs: 0, kind: "lifecycle", action: "resume_requested" },
    { sequence: 1, elapsedMs: 0, kind: "recovery", action: "recovered" },
    { sequence: 1, elapsedMs: 0, kind: "failure", category: "authentication" },
    { sequence: 1, elapsedMs: 0, kind: "translation_backoff", reason: "rateLimited", retryScheduled: false },
    { sequence: 1, elapsedMs: 0, kind: "translation_backoff", reason: "temporarilyUnavailable", retryScheduled: true },
  ]);
});

it("keeps unsupported language as a safe configuration failure", () => {
  expect(supportDiagnosticSummary(report({ last_error: { classification: { category: "configuration", code: "UNSUPPORTED_LANGUAGE" } } }))?.lastError).toBe("configuration");
});

it.each(["permission", "user_stopped", "start"] as const)("keeps the backend's %s category in the error and event summary", category => {
  const result = supportDiagnosticSummary(report({
    session_status: "error",
    last_error: { classification: { category }, observed_ago_ms: 0 },
    journal: { recent_events: [{ sequence: 1, elapsed_since_app_start_ms: 1, kind: "failure", classification: { category } }] },
  }))!;
  expect(result.lastError).toBe(category);
  expect(result.lastErrorAgeMs).toBe(0);
  expect(result.recentEvents[0]).toMatchObject({ kind: "failure", category });
  expect(diagnosticNextStep(result)).toBe(category === "user_stopped" ? null : "audio");
});

it("only treats newer recovery as recovery after the recorded error", () => {
  const parsed = (errorAge: number, recoveryAge: number) => supportDiagnosticSummary(report({
    last_error: { classification: { category: "transport" }, observed_ago_ms: errorAge },
    recent_recovery: { action: "recovered", observed_ago_ms: recoveryAge },
  }))!;
  expect(parsed(5000, 1000).recoveredAfterError).toBe(true);
  expect(parsed(1000, 5000).recoveredAfterError).toBe(false);
  expect(parsed(1000, 1000).recoveredAfterError).toBe(false);
  expect(diagnosticNextStep({ ...parsed(1000, 5000), status: "error", lastError: "permission" })).toBe("audio");
  expect(diagnosticNextStep(parsed(5000, 1000))).toBeNull();
  expect(supportDiagnosticSummary(report({ recent_recovery: { action: "retrying", observed_ago_ms: 0 } }))?.recoveredAfterError).toBe(false);
  expect(supportDiagnosticSummary(report({
    recent_recovery: { action: "recovered" },
    journal: { recent_events: [
      { sequence: 1, elapsed_since_app_start_ms: 0, kind: "failure", classification: { category: "rate_limit" } },
      { sequence: 2, elapsed_since_app_start_ms: 1, kind: "recovery", action: "recovered" },
    ] },
  }))?.recoveredAfterError).toBe(false);
});

it("does not turn missing, negative or overflowing counts into an observed zero", () => {
  const result = supportDiagnosticSummary(report({
    snapshot: { elapsed_since_app_start_ms: "1000", audio_recent_window_ms: -1 },
    capture: { observation: { pcm_data_recent: true, sound_recent: true, observation_age_ms: -1 } },
    journal: { events_omitted: -1, text_event_counts: { source_draft_events: Number.MAX_SAFE_INTEGER, source_final_events: 1, confirmed_pair_events: -1 } },
  }))!;
  expect(result.elapsedMs).toBeNull();
  expect(result.audioRecentWindowMs).toBeNull();
  expect(result.textEvents).toEqual({ source: null, translation: null, confirmed: null });
  expect(result.omittedEvents).toBeNull();
  expect(diagnosticAudioState(result)).toBe("past");
});

it("distinguishes recent sound, silent data, absent data and past observations without diagnosing a cause", () => {
  const state = (pcm: unknown, sound: unknown, age: unknown, status = "listening") => supportDiagnosticSummary(report({
    session_status: status,
    snapshot: { elapsed_since_app_start_ms: 10000, audio_recent_window_ms: 2000 },
    capture: { observation: { pcm_data_recent: pcm, sound_recent: sound, observation_age_ms: age } },
  }))!;
  expect(diagnosticAudioState(state(true, true, 0))).toBe("sound");
  expect(diagnosticAudioState(state(true, false, 2000))).toBe("quiet");
  expect(diagnosticAudioState(state(false, false, 0))).toBe("noAudio");
  expect(diagnosticAudioState(state(true, true, 2001))).toBe("past");
  expect(diagnosticAudioState(state(true, true, 0, "paused"))).toBe("past");
  expect(diagnosticAudioState(state(true, true, null))).toBe("past");
  expect(diagnosticAudioState(state("true", false, 0))).toBe("unknown");
  expect(diagnosticNextStep(state(false, false, 0))).toBe("audio");
  expect(diagnosticNextStep(state(true, false, 0))).toBeNull();
  expect(diagnosticNextStep(state(false, false, 2001))).toBeNull();
});

it("keeps cumulative event counts separate from current translation and missing counts", () => {
  const result = supportDiagnosticSummary(report({
    service: { translation_pending: false, translation_preview_pending: true, translation_timed_out: false,
      translation_recovery: { reason: "rateLimited", retryScheduled: true, retryAfterMs: 9000 } },
    journal: { text_event_counts: { source_draft_events: 18, source_final_events: 0, translation_draft_events: 10, translation_final_events: 2, confirmed_pair_events: 3 }, events_omitted: 80 },
  }))!;
  expect(result.textEvents).toEqual({ source: 18, translation: 12, confirmed: 3 });
  expect(result.omittedEvents).toBe(80);
  expect(result.translationRecovery).toEqual({ reason: "rateLimited", retryScheduled: true });
  expect(diagnosticNextStep(result)).toBeNull();
  expect(supportDiagnosticSummary(report({ journal: { text_event_counts: { source_draft_events: 18, source_final_events: "0" } } }))?.textEvents).toEqual({ source: null, translation: null, confirmed: null });
});

it("retains at most 24 safe recent events and exposes the latest six after validation", () => {
  const events = Array.from({ length: 30 }, (_, sequence) => ({ kind: "lifecycle", action: "start_requested", sequence, elapsed_since_app_start_ms: sequence }));
  const result = supportDiagnosticSummary(report({ journal: { recent_events: [...events, { kind: "unknown", raw: "private" }] } }))!;
  expect(result.allEvents).toHaveLength(23);
  expect(result.recentEvents.map(event => event.sequence)).toEqual([24, 25, 26, 27, 28, 29]);
  expect(JSON.stringify(result)).not.toContain("private");
});

it("never recommends a service retry when the report says it is already scheduled", () => {
  const result = (scheduled: boolean) => supportDiagnosticSummary(report({ service: {
    translation_recovery: { reason: "temporarilyUnavailable", retryScheduled: scheduled },
  } }))!;
  expect(diagnosticNextStep(result(true))).toBeNull();
  expect(diagnosticNextStep(result(false))).toBe("services");
});

it("preserves recognition failures after listening without inventing a startup failure or native cause", () => {
  const classification = { category: "recognition", phase: "asr_recognition", code: "APPLE_SPEECH_RECOGNITION_FAILED", description: "private-native-detail" };
  const result = supportDiagnosticSummary(report({
    session_status: "error",
    last_error: { classification },
    journal: { recent_events: [
      { sequence: 1, elapsed_since_app_start_ms: 100, kind: "status", status: "listening" },
      { sequence: 2, elapsed_since_app_start_ms: 707_700, kind: "failure", classification },
      { sequence: 3, elapsed_since_app_start_ms: 707_701, kind: "status", status: "error" },
    ] },
  }));
  expect(result?.status).toBe("error");
  expect(result?.lastError).toBe("recognition");
  expect(result?.recentEvents).toEqual([
    { sequence: 1, elapsedMs: 100, kind: "status", status: "listening" },
    { sequence: 2, elapsedMs: 707_700, kind: "failure", category: "recognition" },
    { sequence: 3, elapsedMs: 707_701, kind: "status", status: "error" },
  ]);
  expect(JSON.stringify(result)).not.toContain("private-native-detail");
  expect(JSON.stringify(result)).not.toContain("asr_setup");
});
