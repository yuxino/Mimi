import { I18N } from "/src/lib/i18n.ts";

// An asynchronous fixture import can outlive the mounted page during HMR or
// navigation. Fail with the actual lifecycle/state instead of a null DOM read;
// never seed another session or replace the missing product surface here.
export function requireOverlayTimeline(store, stage) {
  const surface = document.querySelector(".overlay-surface");
  const timeline = surface?.querySelector(".overlay-timeline");
  const route = new URLSearchParams(location.search).get("window");
  const { session, settings } = store.getState();
  if (route !== "overlay" || !surface?.isConnected || !timeline?.isConnected || session.subtitles.history.length === 0) {
    throw new Error("Overlay fixture is not mounted for measurement: " + JSON.stringify({ stage, route,
      readyState: document.readyState, visibility: document.visibilityState,
      rootChildren: document.getElementById("root")?.childElementCount ?? 0,
      surfaces: document.querySelectorAll(".overlay-surface").length,
      canvases: document.querySelectorAll(".overlay-expanded-canvas").length,
      timelines: document.querySelectorAll(".overlay-timeline").length,
      active: session.isActive, status: session.status.kind, paused: session.isPaused,
      collapsed: session.isOverlayCollapsed, immersive: settings.subtitleBlendsWithBackground,
      historyCount: session.subtitles.history.length, trackCount: session.subtitles.tracks?.length ?? 0,
    }));
  }
  return { surface, timeline };
}

// Check painted text through its ancestors and clipping layers. AX/DOM presence
// alone cannot distinguish quiet chrome from a visible status or measurement.
export function measureTextVisibility(text, timeline) {
  if (!text) return { present: false, rendered: false, uncovered: false, captionOverlap: false };
  const range = document.createRange();
  range.selectNodeContents(text);
  const rects = [...range.getClientRects()].filter(rect => rect.width > 0 && rect.height > 0);
  let opacity = 1, styled = true;
  let clip = { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
  for (let node = text; node; node = node.parentElement) {
    const style = getComputedStyle(node), rect = node.getBoundingClientRect();
    opacity *= Number(style.opacity);
    styled &&= style.display !== "none" && !["hidden", "collapse"].includes(style.visibility)
      && style.contentVisibility !== "hidden";
    if (/(hidden|clip|scroll|auto)/.test(style.overflowX)) clip = { ...clip, left: Math.max(clip.left, rect.left), right: Math.min(clip.right, rect.right) };
    if (/(hidden|clip|scroll|auto)/.test(style.overflowY)) clip = { ...clip, top: Math.max(clip.top, rect.top), bottom: Math.min(clip.bottom, rect.bottom) };
  }
  const unclipped = rects.length > 0 && rects.every(rect => rect.left >= clip.left - 1 && rect.right <= clip.right + 1
    && rect.top >= clip.top - 1 && rect.bottom <= clip.bottom + 1);
  const color = getComputedStyle(text).color;
  const colorAlpha = color.startsWith("rgba(") ? Number(color.match(/,\s*([\d.]+)\)$/)?.[1])
    : Number(color.match(/\/\s*([\d.]+)\s*\)$/)?.[1] ?? 1);
  const rendered = styled && opacity * colorAlpha > 0.01 && unclipped;
  const view = timeline?.getBoundingClientRect();
  const captionOverlap = Boolean(view && rects.some(rect => rect.left < view.right && rect.right > view.left
    && rect.top < view.bottom && rect.bottom > view.top));
  let uncovered = false;
  if (rendered) {
    // Status text can opt out of pointer hits. Include only this text briefly;
    // neither its paint nor layout changes, and restore the original hit rule.
    const pointerEvents = text.style.pointerEvents;
    try {
      text.style.pointerEvents = "auto";
      uncovered = rects.every(rect => [0.2, 0.5, 0.8].every(fraction =>
        text.contains(document.elementFromPoint(rect.left + rect.width * fraction, rect.top + rect.height / 2))));
    } finally { text.style.pointerEvents = pointerEvents; }
  }
  return { present: true, rendered, opacity, unclipped, uncovered, captionOverlap };
}

// Scope the ordinary progress/readout to the main subtitle window. A separate
// details panel may legitimately expose the same full session observations.
export function translationIndicator(surface, timeline) {
  const phase = surface.querySelector('.overlay-control-embedded[data-mode="island"] .overlay-control-island__phase');
  const statuses = [...surface.querySelectorAll('.overlay-expanded-canvas .overlay-latency [role="status"]')];
  const work = [...surface.querySelectorAll('.overlay-expanded-canvas .overlay-latency span, .overlay-expanded-canvas .overlay-latency strong')]
    .filter(node => !node.childElementCount && node.textContent === I18N.overlay.translating);
  return { expected: I18N.overlay.translating, phase: phase?.textContent ?? null,
    phaseGlyph: measureTextVisibility(phase, timeline),
    rootWorkGlyphs: work.map(node => measureTextVisibility(node, timeline)),
    statuses: statuses.map(node => ({ text: node.textContent,
      ...measureTextVisibility(node.querySelector("strong") ?? node, timeline) })) };
}

function readoutPlacement(surface, timeline, nodes) {
  const box = surface.getBoundingClientRect();
  const strip = surface.querySelector(".overlay-action-strip")?.getBoundingClientRect();
  const island = surface.querySelector('.overlay-control-embedded[data-mode="island"] .overlay-control-island')?.getBoundingClientRect();
  const rects = nodes.map(node => node.getBoundingClientRect());
  if (!strip || !island || !rects.length) return { issues: ["readout, toolbar or real island missing"] };
  const edges = rect => ({ left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom });
  const bounds = { left: Math.min(...rects.map(rect => rect.left)), top: Math.min(...rects.map(rect => rect.top)),
    right: Math.max(...rects.map(rect => rect.right)), bottom: Math.max(...rects.map(rect => rect.bottom)) };
  const overlaps = (a, b) => a.left < b.right - 1 && b.left < a.right - 1 && a.top < b.bottom - 1 && b.top < a.bottom - 1;
  const issues = [];
  if (rects.some(rect => overlaps(rect, strip) || overlaps(rect, island) || overlaps(rect, timeline.getBoundingClientRect()))) issues.push("readout overlaps controls or captions");
  if (rects.some(rect => Math.abs((rect.top + rect.bottom) / 2 - (rects[0].top + rects[0].bottom) / 2) > 2)) issues.push("progress and number are not on one line");
  const summary = surface.querySelector('.overlay-control-embedded[data-mode="island"] .overlay-control-island__summary');
  if (!summary || summary.scrollHeight > summary.clientHeight + 1 || island.height > 31) issues.push("language island is not a single compact line");
  if (bounds.left < box.left + 12 || bounds.right > box.right - 12) issues.push("readout outside canvas edges");
  if (box.width >= 552) {
    if (bounds.right > strip.left - 3 || rects.some(rect => rect.top + rect.height / 2 < strip.top
      || rect.top + rect.height / 2 > strip.bottom)) issues.push("wide readout is not beside the toolbar");
  } else if (bounds.top < strip.bottom - 1 || bounds.top > strip.bottom + 20
    || bounds.right > strip.right + 1 || strip.right - bounds.right > 16) issues.push("narrow readout is not below and aligned right");
  return { issues, width: box.width, placement: box.width >= 552 ? "beside" : "below",
    readout: bounds, toolbar: edges(strip), island: edges(island) };
}

function inspectReadout(surface, timeline, { samples, source, pending = false, recovery = false, status = null, follow = false }, hovered) {
  const numbers = [...surface.querySelectorAll(".overlay-expanded-canvas .overlay-latency__measurement strong")];
  const glyphs = numbers.map(node => measureTextVisibility(node, timeline));
  const indicator = translationIndicator(surface, timeline);
  const visible = glyph => glyph.rendered && glyph.uncovered && !glyph.captionOverlap;
  // Long language names may intentionally ellipsize, but the actual language
  // separator still supplies an unclipped glyph witness for the compact child.
  const languageGlyph = measureTextVisibility(surface.querySelector('.overlay-control-embedded[data-mode="island"] .overlay-control-island__summary [aria-hidden="true"]'), timeline);
  const labels = numbers.map(node => node.parentElement.getAttribute("aria-label"));
  const sourceLabel = source === "api" ? I18N.overlay.apiLatency
    : follow ? I18N.overlay.translationFollowLatency : I18N.overlay.translationLatency;
  const labelMatches = numbers.every(node => node.parentElement.getAttribute("aria-label") === `${sourceLabel}: ${node.textContent}`
    && node.parentElement.getAttribute("title")?.includes(node.textContent)
    && node.parentElement.textContent.trim() === node.textContent && !node.parentElement.querySelector("svg"));
  const statuses = [...surface.querySelectorAll('.overlay-expanded-canvas .overlay-latency [role="status"]')];
  const components = [...numbers.map(node => node.parentElement), ...(pending ? statuses : [])];
  const placement = hovered && !recovery && components.length ? readoutPlacement(surface, timeline, components) : null;
  const expectedStatus = recovery ? status : pending ? indicator.expected : null;
  const statusesMatch = expectedStatus === null ? indicator.statuses.length === 0
    : indicator.statuses.length === 1 && indicator.statuses[0].text === expectedStatus
      && ((hovered || recovery) ? visible(indicator.statuses[0]) : !indicator.statuses[0].rendered);
  const workMatches = pending ? indicator.rootWorkGlyphs.length === 1
    && indicator.rootWorkGlyphs.every(glyph => hovered ? visible(glyph) : !glyph.rendered) : indicator.rootWorkGlyphs.length === 0;
  const ready = numbers.map(node => node.textContent).join("\n") === samples.join("\n")
    && glyphs.every(glyph => hovered ? visible(glyph) : !glyph.rendered) && labelMatches
    && languageGlyph.present && (hovered ? visible(languageGlyph) : !languageGlyph.rendered)
    && indicator.phase === null && statusesMatch && workMatches && (!placement || placement.issues.length === 0);
  return { ready, samples: numbers.map(node => node.textContent), labels, glyphs, languageGlyph, indicator, placement };
}

// The caller owns real hover/quiet pointer input. Change only observations on
// the same live captions so a pending request cannot conceal a viewport jump.
export async function verifyOverlayLatency(store, { hovered, motion }) {
  const { surface, timeline } = requireOverlayTimeline(store, "latency observations");
  const session = store.getState().session;
  const anchor = [...timeline.querySelectorAll("span")].find(node => !node.childElementCount
    && node.textContent === session.subtitles.source.text);
  if (!anchor) throw new Error("Latency fixture needs the already visible live source line");
  const read = () => {
    const view = timeline.getBoundingClientRect(), line = anchor.getBoundingClientRect();
    return { top: view.top, bottom: view.bottom, anchorY: line.top,
      anchorVisible: line.width > 0 && line.top >= view.top - 1 && line.bottom <= view.bottom + 1,
      tailGap: timeline.scrollHeight - timeline.clientHeight - timeline.scrollTop };
  };
  const initial = read();
  if (!initial.anchorVisible || Math.abs(initial.tailGap) > 2) throw new Error("Latency fixture is not following its visible live line");
  const measured = { apiLatencyMs: 128, translationLatencyMs: 246, translationLatencyKind: "request" };
  const absent = { apiLatencyMs: null, translationLatencyMs: null, translationLatencyKind: null };
  const cases = [
    { stage: "no samples", values: absent, samples: [] },
    { stage: "API only", values: { ...absent, apiLatencyMs: 128 }, samples: ["128 ms"], source: "api" },
    { stage: "translation only", values: { ...absent, translationLatencyMs: 246, translationLatencyKind: "request" }, samples: ["246 ms"], source: "translation" },
    { stage: "both samples", values: measured, samples: ["128 ms"], source: "api" },
    { stage: "follow sample", values: { ...absent, translationLatencyMs: 246, translationLatencyKind: "follow" }, samples: ["246 ms"], source: "translation", follow: true },
    { stage: "final pending", values: { ...measured, isTranslationPending: true }, samples: ["128 ms"], source: "api", pending: true },
    { stage: "final cleared", values: measured, samples: ["128 ms"], source: "api" },
    { stage: "preview pending", values: { ...measured, isTranslationPreviewPending: true }, samples: ["128 ms"], source: "api", pending: true },
    { stage: "pending translation fallback", values: { ...absent, translationLatencyMs: 246, translationLatencyKind: "request", isTranslationPending: true },
      samples: ["246 ms"], source: "translation", pending: true },
    { stage: "pending without samples", values: { ...absent, isTranslationPending: true }, samples: [], pending: true },
    { stage: "zero API wins", values: { ...measured, apiLatencyMs: 0 }, samples: ["0 ms"], source: "api" },
    { stage: "unknown translation boundary", values: { ...absent, translationLatencyMs: 246 }, samples: [] },
    { stage: "recovery", values: { ...measured, translationRecovery: { reason: "rateLimited", retryAfterMs: 600, retryScheduled: true } },
      samples: [], status: I18N.overlay.translationRateLimited, recovery: true },
    { stage: "recovered", values: measured, samples: ["128 ms"], source: "api" },
  ];
  const results = [];
  try {
    for (const sample of cases) {
      const tracks = session.subtitles.tracks.map(track => ({
        ...track, isTranslationPending: Boolean(sample.values.isTranslationPending),
      }));
      store.setState({ session: { ...session, isTranslationPending: false, isTranslationPreviewPending: false,
        translationRecovery: null, ...sample.values, subtitles: { ...session.subtitles, tracks } } });
      const start = performance.now();
      let steadySince, frames = 0, maxShift = 0, viewportShift = 0, last;
      while (true) {
        const now = await new Promise(requestAnimationFrame);
        const current = read();
        frames++;
        maxShift = Math.max(maxShift, Math.abs(current.bottom - initial.bottom), Math.abs(current.anchorY - initial.anchorY));
        viewportShift = Math.max(viewportShift, Math.abs(current.top - initial.top));
        // A quiet actionable recovery needs its own visible line. Ordinary
        // samples/pending must preserve the viewport too, including its absence.
        if (!anchor.isConnected || !current.anchorVisible || Math.abs(current.tailGap) > 2 || maxShift > 2
          || (!sample.recovery && viewportShift > 2)) {
          throw new Error("Latency changed the live caption geometry: " + JSON.stringify({ stage: sample.stage,
            hovered, motion, initial, current, maxShift, viewportShift }));
        }
        last = inspectReadout(surface, timeline, sample, hovered);
        if (last.ready) {
          steadySince ??= now;
          if (now - steadySince >= 200) { results.push({ stage: sample.stage, ...last, frames, maxShift, viewportShift }); break; }
        } else steadySince = undefined;
        if (now - start > 2500) throw new Error("Latency visibility did not settle: " + JSON.stringify({ hovered, motion, stage: sample.stage, ...last }));
      }
    }
  } finally { store.setState({ session }); }
  return results;
}

// Used by the nine-language narrow/wide matrix. The same confirmed line and
// actual island survive pending/cleared states under a stationary real pointer.
export async function verifyReadoutLayout(store, { hovered }) {
  const { surface, timeline } = requireOverlayTimeline(store, "localized readout layout");
  const { session } = store.getState();
  const text = session.subtitles.history.at(-1).source;
  const anchor = [...timeline.querySelectorAll("span")].find(node => !node.childElementCount && node.textContent === text);
  if (!anchor) throw new Error("Readout layout fixture has no confirmed anchor");
  const initial = { y: anchor.getBoundingClientRect().top, top: timeline.getBoundingClientRect().top,
    bottom: timeline.getBoundingClientRect().bottom };
  const results = [];
  try {
    for (const pending of [true, false]) {
      store.setState({ session: { ...session, isTranslationPending: pending, isTranslationPreviewPending: false,
        translationRecovery: null, translationLatencyKind: "follow",
        subtitles: { ...session.subtitles, tracks: session.subtitles.tracks.map(track => ({ ...track, isTranslationPending: pending })) } } });
      const start = performance.now();
      let steadySince, frames = 0, maxShift = 0, last;
      while (true) {
        const now = await new Promise(requestAnimationFrame);
        frames++;
        const view = timeline.getBoundingClientRect(), line = anchor.getBoundingClientRect();
        maxShift = Math.max(maxShift, Math.abs(line.top - initial.y), Math.abs(view.top - initial.top), Math.abs(view.bottom - initial.bottom));
        if (!anchor.isConnected || maxShift > 2 || line.bottom > view.bottom + 1 || line.top < view.top - 1) {
          throw new Error("Pending readout moved or clipped the confirmed line: " + JSON.stringify({ hovered, pending, initial, maxShift }));
        }
        last = inspectReadout(surface, timeline, { samples: ["128 ms"], source: "api", pending }, hovered);
        if (last.ready) {
          steadySince ??= now;
          if (now - steadySince >= 200) { results.push({ pending, ...last, maxShift, frames }); break; }
        } else steadySince = undefined;
        if (now - start > 2500) throw new Error("Readout layout did not settle: " + JSON.stringify({ hovered, pending, ...last }));
      }
    }
  } finally { store.setState({ session }); }
  return results;
}
