import type { ServiceProfile } from "./types";

/** Match the native Unicode-character limit while keeping copies distinguishable. */
export function profileCopyName(name: string, suffix: string, profiles: Pick<ServiceProfile, "name">[]): string {
  const used = new Set(profiles.map(profile => profile.name));
  for (let index = 1; ; index += 1) {
    const ending = ` (${suffix}${index === 1 ? "" : ` ${index}`})`;
    const candidate = Array.from(name).slice(0, Math.max(0, 64 - Array.from(ending).length)).join("").trimEnd() + ending;
    if (!used.has(candidate)) return candidate;
  }
}
