import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { sessionExport, sessionHistoryRename, sessionArchiveAudio } from "../../ipc";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(undefined) }));
beforeEach(() => vi.mocked(invoke).mockClear());

it("sends transcript options with the exact selected session and no recording source", async () => {
  const options = { format: "markdown", content: "translation", includeTimestamps: true } as const;
  await sessionExport("transcript", "saved-A", undefined, options);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("session_export", {
    kind: "transcript", id: "saved-A", audioSource: null, options,
  });
});

it("keeps omitted transcript options compatible with the native defaults", async () => {
  await sessionExport("transcript");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("session_export", {
    kind: "transcript", id: null, audioSource: null, options: null,
  });
});

it("keeps a historical microphone export separate from transcript options", async () => {
  await sessionExport("audio", "saved-mic", "microphone");
  expect(invoke).toHaveBeenCalledExactlyOnceWith("session_export", {
    kind: "audio", id: "saved-mic", audioSource: "microphone", options: null,
  });
});

it.each(["Product review", ""])("renames only the selected session with title %j", async title => {
  await sessionHistoryRename("saved-A", title);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("session_history_rename", { id: "saved-A", title });
});

it.each([undefined, "system", "microphone"] as const)("reads current audio with explicit source %s", async source => {
  await sessionArchiveAudio(source);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("session_archive_audio", { audioSource: source ?? null });
});
