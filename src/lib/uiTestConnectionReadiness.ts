import { appIsUiTest, testProfileConnection } from "./ipc";

/** Probe the native diagnostic contract only in credential-free UI-test mode. */
export async function verifyUiTestConnectionReadiness(profileId: string): Promise<void> {
  if (!await appIsUiTest()) return;

  let result;
  try {
    result = await testProfileConnection(profileId);
  } catch (error) {
    // A fresh installation has no profile. Still exercise the real command and
    // capability; accept only its exact missing-profile response, never a
    // missing IPC command or another startup error.
    if (!profileId && error === "profile_not_found") return;
    throw error;
  }
  if (
    !profileId ||
    result.credential !== "present" ||
    result.service !== "notTested" ||
    result.reason !== null
  ) {
    throw new Error("connection_diagnostic_smoke_failed");
  }
}
