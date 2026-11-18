import { expect, it } from "vitest";
import { shareUnchangedSubtitleHistory } from "./sessionSnapshot";
import { useStore } from "../store";
import type { SessionStateEvent } from "../types";

const session: SessionStateEvent = {
  ...useStore.getState().session,
  subtitles: {
    source: { text: "Synthetic source", isFinal: false },
    translation: { text: "Synthetic translation", isFinal: false },
    history: [
      { source: "Synthetic first", translation: "Synthetic first translation", createdAt: 1 },
      { source: "Synthetic second", translation: "Synthetic second translation", createdAt: 2 },
    ],
  },
};
function snapshot() { return JSON.parse(JSON.stringify(session)) as SessionStateEvent; }

it("reuses identical history while preserving every incoming live and timing field without mutation", () => {
  const incoming = snapshot();
  incoming.subtitles.source = { text: "Synthetic revised source", isFinal: true, utteranceId: "new-id" };
  incoming.subtitles.translation.text = "Synthetic new draft";
  incoming.apiLatencyMs = 37;
  incoming.translationLatencyMs = 83;
  incoming.translationRecovery = { reason: "rateLimited", retryAfterMs: 4000 };
  incoming.status = { kind: "error", message: "Synthetic error" };
  const merged = shareUnchangedSubtitleHistory(session, incoming);
  expect(merged.subtitles.history).toBe(session.subtitles.history);
  expect(merged).toEqual(incoming);
  expect(merged.subtitles.source).toBe(incoming.subtitles.source);
  expect(merged.subtitles.translation).toBe(incoming.subtitles.translation);
  expect(incoming.subtitles.history).not.toBe(session.subtitles.history);
  expect(session.subtitles.source.text).toBe("Synthetic source");
});

it.each(["createdAt", "source", "translation"] as const)("keeps a correction to an earlier history item's %s", (field) => {
  const incoming = snapshot();
  if (field === "createdAt") incoming.subtitles.history[0].createdAt = 7;
  else incoming.subtitles.history[0][field] = "Synthetic correction";
  expect(shareUnchangedSubtitleHistory(session, incoming)).toBe(incoming);
});

it("keeps append, clear and bounded trim changes even when the latest pair is unchanged", () => {
  for (const history of [
    [], session.subtitles.history.slice(1),
    [...session.subtitles.history, { source: "Synthetic third", translation: "Synthetic third translation", createdAt: 3 }],
  ]) {
    const incoming = { ...snapshot(), subtitles: { ...snapshot().subtitles, history } };
    expect(shareUnchangedSubtitleHistory(session, incoming)).toBe(incoming);
    expect(incoming.subtitles.history).toBe(history);
  }
});

it("does not reuse equal text when the source identity changes", () => {
  const incoming = snapshot();
  incoming.subtitles.history[0].audioSource = "microphone";
  expect(shareUnchangedSubtitleHistory(session, incoming)).toBe(incoming);
});
