// Runs in the mounted, memory-only overlay while the pointer stays in one place.
export async function verifyCaptionMotion(store, { hovered, motion }) {
  const surface = document.querySelector(".overlay-surface");
  const timeline = surface.querySelector(".overlay-timeline");
  const box = surface.getBoundingClientRect();
  if (Math.abs(box.width - 580) > 1 || Math.abs(box.height - 240) > 1
    || surface.matches(":hover") !== hovered || document.body.classList.contains("motion-reduced") === motion) {
    throw new Error("Unexpected caption fixture: " + JSON.stringify({ hovered, motion, width: box.width,
      height: box.height, actualHover: surface.matches(":hover"), bodyClass: document.body.className }));
  }

  const session = store.getState().session;
  const history = session.subtitles.history;
  const previous = history.at(-1);
  const source = { text: "The way to", utteranceId: "current", isFinal: false };
  const empty = { text: "", isFinal: false };
  const draft = { source, translation: empty, history };
  const translated = { ...draft, translation: { text: "前往的路", utteranceId: "current", isFinal: true } };
  const next = {
    source: { text: "Another path.", utteranceId: "next", isFinal: false }, translation: empty,
    history: [...history, { audioSource: "system", source: source.text, translation: translated.translation.text,
      createdAt: previous.createdAt + 1000 }],
  };
  function publish(content, pending = false, previewPending = false) {
    const track = { ...content, audioSource: "system", detectedLanguage: "en",
      isTranslationPending: pending, isTranslationTimedOut: false };
    store.setState({ session: { ...session, isTranslationPending: pending, isTranslationPreviewPending: previewPending,
      subtitles: { ...content, tracks: [track] } } });
  }

  const span = text => [...timeline.querySelectorAll("span")].find(node => !node.childElementCount && node.textContent === text);
  const tailGap = () => timeline.scrollHeight - timeline.clientHeight - timeline.scrollTop;
  function visible(node) {
    const view = timeline.getBoundingClientRect();
    const rect = node?.getBoundingClientRect();
    return rect?.width > 0 && rect.bottom > view.top && rect.top < view.bottom;
  }
  // Wait for the actual text and tail, then sample through the next paints so
  // a transient movement cannot pass just because the final position is right.
  async function watch(stage, ready, sample) {
    const start = performance.now();
    let steadySince;
    while (true) {
      const now = await new Promise(requestAnimationFrame);
      sample?.();
      if (ready()) {
        steadySince ??= now;
        if (now - steadySince >= 200) return;
      } else steadySince = undefined;
      if (now - start > 2500) throw new Error("Captions did not settle: " + JSON.stringify({ stage, hovered,
        motion, tailGap: tailGap(), lastRow: timeline.lastElementChild?.dataset.utteranceId }));
    }
  }

  publish(draft);
  await watch("initial draft", () => visible(span(previous.source)) && visible(span(source.text)) && Math.abs(tailGap()) <= 2);
  const oldText = span(previous.source);
  const liveId = span(source.text).closest("[data-utterance-id]").dataset.utteranceId;
  if (!liveId || timeline.scrollHeight <= timeline.clientHeight + 2) throw new Error("Expected a live draft above an overflowing history");
  function read() {
    const { top, bottom } = timeline.getBoundingClientRect();
    return { top, bottom, oldY: oldText.getBoundingClientRect().top, scrollTop: timeline.scrollTop };
  }
  const initial = read();
  let lowestY = initial.oldY;
  const result = { hovered, motion, frames: 0, viewportShift: 0, pendingShift: 0, downward: 0 };
  async function transition(stage, content, pending, previewPending, expectedText) {
    const sameDraft = content === draft;
    publish(content, pending, previewPending);
    await watch(stage, () => visible(span(expectedText)) && Math.abs(tailGap()) <= 2, () => {
      const current = read();
      result.frames++;
      result.viewportShift = Math.max(result.viewportShift, Math.abs(current.top - initial.top), Math.abs(current.bottom - initial.bottom));
      if (sameDraft) result.pendingShift = Math.max(result.pendingShift, Math.abs(current.oldY - initial.oldY));
      result.downward = Math.max(result.downward, current.oldY - lowestY);
      lowestY = Math.min(lowestY, current.oldY);
      const liveChanged = sameDraft && span(source.text)?.closest("[data-utterance-id]")?.dataset.utteranceId !== liveId;
      if (!result.failure && (!oldText.isConnected || liveChanged || result.viewportShift > 2
        || result.pendingShift > 2 || result.downward > 2)) result.failure = { stage, initial, current, liveChanged };
    });
  }
  await transition("final pending", draft, true, false, source.text);
  await transition("final cleared", draft, false, false, source.text);
  await transition("preview pending", draft, false, true, source.text);
  await transition("preview cleared", draft, false, false, source.text);
  await transition("translation arrived", translated, false, false, translated.translation.text);
  await transition("next sentence", next, false, false, next.source.text);
  if (initial.oldY - read().oldY <= 2) result.failure ??= { stage: "added text did not advance the history", initial, current: read() };
  return result;
}
