import { openAIRealtimeErrorCode } from "../../lib/locales/openaiRealtimeErrors";

const STATUSES = ["idle", "connecting", "stopping", "listening", "paused", "error"] as const;
const FAILURE_CATEGORIES = ["unknown", "recognition", "configuration", "authentication", "credential_storage", "device_unavailable", "timeout", "request_rejected", "service_error", "rate_limit", "backlog", "transport", "stopped", "processing", "size_limit", "permission", "user_stopped", "start"] as const;
const LIFECYCLE_ACTIONS = ["start_requested", "start_busy", "start_already_active", "start_superseded", "stop_requested", "pause_requested", "resume_requested"] as const;
const RECOVERY_ACTIONS = ["retrying", "recovered", "retries_exhausted", "user_stopped"] as const;
export type DiagnosticStatus = (typeof STATUSES)[number];
export type DiagnosticFailureCategory = (typeof FAILURE_CATEGORIES)[number];
export type DiagnosticLifecycleAction = (typeof LIFECYCLE_ACTIONS)[number];
export type DiagnosticRecoveryAction = (typeof RECOVERY_ACTIONS)[number];

type EventFacts =
  | { kind: "status"; status: DiagnosticStatus }
  | { kind: "lifecycle"; action: DiagnosticLifecycleAction }
  | { kind: "failure"; category: DiagnosticFailureCategory; code?: string }
  | { kind: "recovery"; action: DiagnosticRecoveryAction }
  | { kind: "translation_backoff"; reason: "rateLimited" | "temporarilyUnavailable"; retryScheduled: boolean };
export type DiagnosticSummaryEvent = EventFacts & { sequence: number; elapsedMs: number };
export interface SupportDiagnosticSummary {
  status: DiagnosticStatus;
  apiLatencyMs: number | null;
  translationLatencyMs: number | null;
  translationLatencyKind: "request" | "follow" | null;
  lastError: DiagnosticFailureCategory | null;
  lastErrorCode?: string;
  lastErrorAgeMs: number | null;
  recoveredAfterError: boolean;
  elapsedMs: number | null;
  audioRecentWindowMs: number | null;
  capture: { pcm: boolean; sound: boolean; ageMs: number | null } | null;
  outputAvailability: "available" | "unavailable" | "unknown" | null;
  translationPending: boolean | null;
  translationPreviewPending: boolean | null;
  translationTimedOut: boolean | null;
  translationRecovery: { reason: "rateLimited" | "temporarilyUnavailable"; retryScheduled: boolean } | null;
  textEvents: { source: number | null; translation: number | null; confirmed: number | null };
  omittedEvents: number | null;
  allEvents: DiagnosticSummaryEvent[];
  recentEvents: DiagnosticSummaryEvent[];
}

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}
function enumeration<T extends string>(value: unknown, allowed: readonly T[]): T | null {
  return typeof value === "string" && allowed.includes(value as T) ? value as T : null;
}
function nonNegativeInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : null;
}
function boolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}
function addCounts(first: unknown, second: unknown): number | null {
  const a = nonNegativeInteger(first), b = nonNegativeInteger(second);
  return a !== null && b !== null && Number.isSafeInteger(a + b) ? a + b : null;
}
function translationRecovery(value: unknown): SupportDiagnosticSummary["translationRecovery"] {
  const recovery = object(value);
  const reason = enumeration(recovery?.reason, ["rateLimited", "temporarilyUnavailable"] as const);
  return reason && typeof recovery?.retryScheduled === "boolean" ? { reason, retryScheduled: recovery.retryScheduled } : null;
}
function failureCategory(value: unknown): DiagnosticFailureCategory | null {
  return enumeration(object(value)?.category, FAILURE_CATEGORIES);
}
function safeFailureCode(value: unknown): string | undefined {
  const code = object(value)?.code;
  return typeof code === "string" ? openAIRealtimeErrorCode(code.toLowerCase()) : undefined;
}
function summaryEvent(value: unknown): DiagnosticSummaryEvent | null {
  const event = object(value);
  if (!event) return null;
  const sequence = nonNegativeInteger(event.sequence), elapsedMs = nonNegativeInteger(event.elapsed_since_app_start_ms);
  if (sequence === null || elapsedMs === null) return null;
  const timing = { sequence, elapsedMs };
  switch (event.kind) {
    case "status": {
      const status = enumeration(event.status, STATUSES);
      return status ? { ...timing, kind: "status", status } : null;
    }
    case "lifecycle": {
      const action = enumeration(event.action, LIFECYCLE_ACTIONS);
      return action ? { ...timing, kind: "lifecycle", action } : null;
    }
    case "failure": {
      const category = failureCategory(event.classification);
      const code = safeFailureCode(event.classification);
      return category ? { ...timing, kind: "failure", category, ...(code && { code }) } : null;
    }
    case "recovery": {
      const action = enumeration(event.action, RECOVERY_ACTIONS);
      return action ? { ...timing, kind: "recovery", action } : null;
    }
    case "translation_backoff": {
      const recovery = translationRecovery(event.recovery);
      return recovery ? { ...timing, kind: "translation_backoff", ...recovery } : null;
    }
    default: return null;
  }
}

/** Presentation reads only typed, bounded facts; arbitrary report strings stay out of the summary. */
export function supportDiagnosticSummary(report: string | null): SupportDiagnosticSummary | null {
  if (report === null) return null;
  try {
    const data = object(JSON.parse(report));
    if (data?.schema !== "mimi.support.v2") return null;
    const status = enumeration(data.session_status, STATUSES);
    if (!status) return null;
    const service = object(data.service);
    const translationLatencyKind = enumeration(service?.translation_duration_kind, ["request", "follow"] as const);
    const journal = object(data.journal);
    const events = journal?.recent_events;
    const allEvents = Array.isArray(events) ? events.slice(-24).map(summaryEvent).filter((event): event is DiagnosticSummaryEvent => event !== null) : [];
    const failure = object(data.last_error);
    const recovery = object(data.recent_recovery);
    const lastErrorCode = safeFailureCode(failure?.classification);
    const lastErrorAgeMs = nonNegativeInteger(failure?.observed_ago_ms);
    const recoveryAgeMs = nonNegativeInteger(recovery?.observed_ago_ms);
    // A retained recovery must not dismiss a newer failure. Missing ages do
    // not establish that the journal's bounded events describe this error.
    const recoveredAfterError = recovery?.action === "recovered" && lastErrorAgeMs !== null && recoveryAgeMs !== null &&
      recoveryAgeMs < lastErrorAgeMs;
    const capture = object(data.capture);
    const observation = object(capture?.observation);
    const pcm = boolean(observation?.pcm_data_recent), sound = boolean(observation?.sound_recent);
    const snapshot = object(data.snapshot);
    const counts = object(journal?.text_event_counts);
    return {
      status,
      apiLatencyMs: nonNegativeInteger(service?.api_round_trip_ms),
      translationLatencyMs: translationLatencyKind ? nonNegativeInteger(service?.translation_duration_ms) : null,
      translationLatencyKind,
      lastError: failureCategory(object(data.last_error)?.classification),
      ...(lastErrorCode && { lastErrorCode }),
      lastErrorAgeMs,
      recoveredAfterError,
      elapsedMs: nonNegativeInteger(snapshot?.elapsed_since_app_start_ms),
      audioRecentWindowMs: nonNegativeInteger(snapshot?.audio_recent_window_ms),
      capture: pcm !== null && sound !== null ? { pcm, sound, ageMs: nonNegativeInteger(observation?.observation_age_ms) } : null,
      outputAvailability: enumeration(capture?.output_availability, ["available", "unavailable", "unknown"] as const),
      translationPending: boolean(service?.translation_pending),
      translationPreviewPending: boolean(service?.translation_preview_pending),
      translationTimedOut: boolean(service?.translation_timed_out),
      translationRecovery: translationRecovery(service?.translation_recovery),
      textEvents: {
        source: addCounts(counts?.source_draft_events, counts?.source_final_events),
        translation: addCounts(counts?.translation_draft_events, counts?.translation_final_events),
        confirmed: nonNegativeInteger(counts?.confirmed_pair_events),
      },
      omittedEvents: nonNegativeInteger(journal?.events_omitted),
      allEvents,
      recentEvents: allEvents.slice(-6),
    };
  } catch { return null; }
}

/** These are observations and navigation suggestions, never root-cause claims. */
export function diagnosticAudioState(summary: SupportDiagnosticSummary): "unavailable" | "past" | "unknown" | "sound" | "quiet" | "noAudio" {
  if (summary.outputAvailability === "unavailable") return "unavailable";
  const capture = summary.capture;
  if (!capture) return "unknown";
  if (summary.status !== "listening" || capture.ageMs === null || summary.audioRecentWindowMs === null ||
    capture.ageMs > summary.audioRecentWindowMs) return "past";
  return !capture.pcm ? "noAudio" : capture.sound ? "sound" : "quiet";
}

export function diagnosticNextStep(summary: SupportDiagnosticSummary): "audio" | "services" | null {
  if (summary.outputAvailability === "unavailable") return "audio";
  if (summary.status === "error" && !summary.recoveredAfterError) {
    if (summary.lastError && ["permission", "device_unavailable", "start", "stopped", "processing"].includes(summary.lastError)) return "audio";
    if (summary.lastError && ["configuration", "authentication", "credential_storage", "request_rejected"].includes(summary.lastError)) return "services";
  }
  if (summary.status === "listening") {
    if (summary.translationRecovery && !summary.translationRecovery.retryScheduled) return "services";
    if (diagnosticAudioState(summary) === "noAudio") return "audio";
  }
  return null;
}
