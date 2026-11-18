export function formatLatency(milliseconds: number | null | undefined): string {
  if (milliseconds == null || !Number.isFinite(milliseconds) || milliseconds < 0) return "—";
  return milliseconds >= 1_000
    ? `${(milliseconds / 1_000).toFixed(1)} s`
    : `${Math.round(milliseconds)} ms`;
}

/** Presentation bands for observed samples, not provider or network guarantees. */
export function latencyTone(milliseconds: number | null | undefined, measurement: "api" | "translation"): "neutral" | "warning" | "slow" {
  if (milliseconds == null || !Number.isFinite(milliseconds) || milliseconds < 0) return "neutral";
  const [warning, slow] = measurement === "api" ? [500, 1_500] : [1_000, 3_000];
  return milliseconds >= slow ? "slow" : milliseconds >= warning ? "warning" : "neutral";
}
