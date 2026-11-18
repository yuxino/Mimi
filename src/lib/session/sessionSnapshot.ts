import type { SessionStateEvent, SubtitleSnapshot } from "../types";

function sharedHistory(before: SubtitleSnapshot["history"], after: SubtitleSnapshot["history"]) {
  if (before === after || before.length !== after.length) return after;
  for (let index = 0; index < after.length; index++) {
    if (before[index].audioSource !== after[index].audioSource ||
        before[index].createdAt !== after[index].createdAt ||
        before[index].source !== after[index].source ||
        before[index].translation !== after[index].translation) return after;
  }
  return before;
}

/** Keep bounded confirmed histories stable without retaining a previous draft,
 * timing sample or lifecycle field, including independent source tracks. */
export function shareUnchangedSubtitleHistory(previous: SessionStateEvent, incoming: SessionStateEvent): SessionStateEvent {
  const history = sharedHistory(previous.subtitles.history, incoming.subtitles.history);
  let tracksChanged = false;
  const tracks = incoming.subtitles.tracks?.map(track => {
    const before = previous.subtitles.tracks?.find(value => value.audioSource === track.audioSource);
    if (!before) return track;
    const sourceHistory = sharedHistory(before.history, track.history);
    if (sourceHistory === track.history) return track;
    tracksChanged = true;
    return { ...track, history: sourceHistory };
  });
  if (history === incoming.subtitles.history && !tracksChanged) return incoming;
  return { ...incoming, subtitles: { ...incoming.subtitles, history,
    ...(tracksChanged ? { tracks } : {}) } };
}
