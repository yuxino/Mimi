import { describe, expect, it, vi } from "vitest";
import type { SettingsSnapshot, ProviderCredentialsInput } from "../../../../lib/types";
import { saveAndSelectProfile } from "./saveAndSelectProfile";

const credentials = { kind: "apiKey", apiKey: "test-only" } as ProviderCredentialsInput;
const snapshot = (id: string) => ({ activeProfileId: id }) as SettingsSnapshot;

describe("save and select a service", () => {
  it("does not activate when secure storage fails", async () => {
    const select = vi.fn();
    await expect(
      saveAndSelectProfile(
        "new",
        credentials,
        async () => {
          throw new Error("storage unavailable");
        },
        select,
      ),
    ).rejects.toThrow("storage unavailable");
    expect(select).not.toHaveBeenCalled();
  });
  it("keeps activation failure visible after a successful save", async () => {
    const save = vi.fn().mockResolvedValue(snapshot("old"));
    const select = vi.fn().mockRejectedValue(new Error("activation failed"));
    await expect(saveAndSelectProfile("new", credentials, save, select)).rejects.toThrow(
      "activation failed",
    );
    expect(save).toHaveBeenCalledOnce();
    expect(select).toHaveBeenCalledWith("new");
  });
  it("does not reselect the active profile or normalize its preferences again", async () => {
    const saved = snapshot("active");
    const select = vi.fn();
    expect(await saveAndSelectProfile("active", credentials, async () => saved, select)).toBe(
      saved,
    );
    expect(select).not.toHaveBeenCalled();
  });
  it("returns the activated snapshot, including normalized language settings", async () => {
    const activated = { ...snapshot("new"), sourceLanguage: "en" } as SettingsSnapshot;
    expect(
      await saveAndSelectProfile(
        "new",
        credentials,
        async () => snapshot("old"),
        async () => activated,
      ),
    ).toBe(activated);
  });
});


it("normalizes capabilities after changing the text route of an already active profile", async () => {
  const save = vi.fn().mockResolvedValue(snapshot("ali"));
  const select = vi.fn().mockResolvedValue({ ...snapshot("ali"), translationMode: "turbo" });
  const update: ProviderCredentialsInput = { kind: "alibabaTranslation", model: "", apiKey: "", textTranslation: "deepLX", endpoint: "https://example.com", token: "" };
  expect((await saveAndSelectProfile("ali", update, save, select)).translationMode).toBe("turbo");
  expect(select).toHaveBeenCalledWith("ali");
});


it("saves an unfinished LiveTranslate key without activating or falling back to ASR", async () => {
  const saved = { ...snapshot("old"), profiles: [{ id: "new", name: "New", provider: "alibabaCloud", credentialState: "present", alibabaRealtimeMode: "liveTranslate", alibabaWorkspaceId: "" }] } as SettingsSnapshot;
  const select = vi.fn();
  expect(await saveAndSelectProfile("new", credentials, async () => saved, select)).toBe(saved);
  expect(select).not.toHaveBeenCalled();
});


it("saves each independent xAI stage without activating an incomplete configuration", async () => {
  const saved = { ...snapshot("old"), targetLanguage: "zh", profiles: [{ id: "new", name: "New", provider: "xAIRealtime", xaiSpeechMode: "asrMt", textTranslation: "xAI", credentialState: "missing", speechCredentialState: "present", textCredentialState: "missing" }] } as SettingsSnapshot;
  const select = vi.fn();
  expect(await saveAndSelectProfile("new", credentials, async () => saved, select)).toBe(saved);
  expect(select).not.toHaveBeenCalled();
});
