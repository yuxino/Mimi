import { measureTextVisibility, requireOverlayTimeline } from "./verify-overlay-latency.js";

// A memory-only product scene. Input remains in the outer browser driver.
export async function prepareCaptionHistory(store) {
  const { surface, timeline } = requireOverlayTimeline(store, "caption history");
  const empty = { text: "", isFinal: false };
  const history = count => Array.from({ length: count }, (_, index) => ({
    audioSource: "system", source: `History ${index + 1}`, translation: `字幕 ${index + 1}`,
    createdAt: 1_700_000_000_000 + index * 1000,
  }));
  function publish(count) {
    const rows = history(count);
    store.setState(state => ({
      settings: { ...state.settings, fontSize: 16 },
      session: { ...state.session, subtitles: { source: empty, translation: empty, history: rows,
        tracks: [{ audioSource: "system", source: empty, translation: empty, history: rows,
          detectedLanguage: "en", isTranslationPending: false, isTranslationTimedOut: false }],
      } },
    }));
  }
  function read() {
    if (!timeline.isConnected) throw new Error("Caption history timeline was replaced");
    const box = timeline.getBoundingClientRect();
    const root = surface.getBoundingClientRect();
    const count = store.getState().session.subtitles.history.length;
    const texts = [`History ${count}`, `字幕 ${count}`];
    const spans = [...timeline.querySelectorAll("span")].filter(node => !node.childElementCount);
    const latest = texts.map(text => {
      const node = spans.find(span => span.textContent === text);
      const visibility = measureTextVisibility(node, timeline);
      return { rendered: visibility.rendered === true, uncovered: visibility.uncovered === true,
        unclipped: visibility.unclipped === true };
    });
    const footer = surface.querySelector(".overlay-return-to-live");
    return { count, top: timeline.scrollTop, max: timeline.scrollHeight - timeline.clientHeight,
      contentHeight: timeline.scrollHeight, viewportHeight: timeline.clientHeight,
      box: { left: box.left, right: box.right, top: box.top, bottom: box.bottom, width: box.width, height: box.height },
      surface: { width: root.width, height: root.height },
      gutterWidth: Math.max(6, timeline.offsetWidth - timeline.clientWidth),
      footer: Boolean(footer && measureTextVisibility(footer).rendered), latest,
    };
  }
  const tailIsPainted = sample => Math.abs(sample.max - sample.top) <= 2 && !sample.footer
    && sample.latest.every(lane => lane.rendered && lane.uncovered && lane.unclipped);
  async function observe(stage, ready) {
    // Use the existing caption probe's 2.5 s bound and 200 ms paint witness.
    const start = performance.now();
    let steadySince, frames = 0, sample;
    while (true) {
      const now = await new Promise(requestAnimationFrame);
      frames++;
      sample = read();
      if (ready(sample)) {
        steadySince ??= now;
        if (now - steadySince >= 200) return { stage, ready: true, frames, sample };
      } else steadySince = undefined;
      if (now - start > 2500) return { stage, ready: false, frames, sample };
    }
  }
  async function requireState(stage, ready) {
    const result = await observe(stage, ready);
    if (!result.ready) throw new Error("Caption scrollbar prerequisite failed: " + JSON.stringify(result));
    return result.sample;
  }
  let events = [];
  function record(event) {
    if (event.type.startsWith("scroll") && event.target !== timeline) return;
    const at = performance.now();
    const viewportHeight = timeline.clientHeight;
    events.push({ type: event.type, at, top: timeline.scrollTop, max: timeline.scrollHeight - viewportHeight,
      viewportHeight, footer: Boolean(surface.querySelector(".overlay-return-to-live")), trusted: event.isTrusted,
      timelineTarget: event.target === timeline });
    if (events.length > 64) events.shift();
  }
  const names = ["pointerdown", "pointerup", "mouseup", "scroll", "scrollend"];
  publish(19);
  await requireState("19 bilingual captions", sample => sample.count === 19
    && timeline.children.length === 19 && tailIsPainted(sample));
  window.__captionHistory = {
    read,
    following() { return requireState("hovered live tail", sample => surface.matches(":hover") && tailIsPainted(sample)); },
    async reading(before) {
      return requireState("wheel entered history", sample => sample.footer && sample.top < before.top - 2
        && sample.viewportHeight >= 140 && sample.viewportHeight <= 170);
    },
    start() { events = []; for (const name of names) window.addEventListener(name, record, true); },
    held(before) {
      return requireState("thumb moved while held", sample => sample.footer && sample.top > before.top + 2
        && sample.max - sample.top > 2 && sample.max === before.max && sample.viewportHeight === before.viewportHeight);
    },
    async finish() {
      // Preserve the original failure, but still observe the next caption so a
      // passive new-layout clamp cannot masquerade as restored follow intent.
      const released = await observe("released at tail", tailIsPainted);
      const gesture = events.slice();
      publish(20);
      const appended = await observe("new caption follows", sample => sample.count === 20 && tailIsPainted(sample));
      const release = gesture.find(event => event.type === "pointerup") ?? gesture.find(event => event.type === "mouseup");
      const afterReleaseScrolls = gesture.filter(event => event.type === "scroll" && event.at >= (release?.at ?? Infinity));
      const result = { release: release ?? null, afterReleaseScrolls, released, appended,
        newContent: appended.sample.contentHeight > released.sample.contentHeight,
        gestureEvents: gesture };
      if (!release?.trusted || !released.ready || !appended.ready || !result.newContent) {
        throw new Error("Native scrollbar release did not restore painted following: " + JSON.stringify(result));
      }
      return result;
    },
    dispose() { for (const name of names) window.removeEventListener(name, record, true); delete window.__captionHistory; },
  };
  return read();
}
