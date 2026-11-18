import { credentialStateForTarget, isIndependentAsrProfile } from "../../../lib/providers/providerCapabilities";
import type { ProviderCredentialsInput, SettingsSnapshot } from "../../../lib/types";

/** Save first: a failed activation must never make a stored key look unsaved. */
export async function saveAndSelectProfile(
  profileId: string,
  credentials: ProviderCredentialsInput,
  save: (id: string, credentials: ProviderCredentialsInput) => Promise<SettingsSnapshot>,
  select: (id: string) => Promise<SettingsSnapshot>,
): Promise<SettingsSnapshot> {
  const saved = await save(profileId, credentials);
  const profile = saved.profiles?.find(item => item.id === profileId);
  // An incomplete LiveTranslate profile can keep its key while setup continues.
  // Activation requires its separately saved workspace, and must not fall back.
  if (profile && isIndependentAsrProfile(profile) && credentialStateForTarget(profile, saved.targetLanguage) !== "present") return saved;
  if (profile?.alibabaRealtimeMode === "liveTranslate" && !profile.alibabaWorkspaceId?.trim()) return saved;
  return saved.activeProfileId === profileId && credentials.kind !== "alibabaTranslation" ? saved : select(profileId);
}
