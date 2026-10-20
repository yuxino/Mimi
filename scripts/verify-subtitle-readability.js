// Real browser geometry for the desktop subtitle range and settings preview.
// Pass an Ego Lite managed Page; all text and IPC responses are synthetic.
export async function verifySubtitlePreviewLayout(page, baseUrl = "http://127.0.0.1:1420", {
  languages = ["zh", "zh-TW", "en", "ja", "de", "fr", "ko", "th"],
  widths = [520, 952], fontSizes = [16, 20, 24, 32, 40, 48],
} = {}) {
  await page.goto(`${baseUrl}/scripts/fixtures/settings-layout.html`);
  await page.waitForFunction(() => window.layoutReady === true);
  await page.cdp("Page.bringToFront", {});
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: true });
  const failures = [];
  let checked = 0;
  for (const language of languages) for (const width of widths) {
    await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
    for (const theme of ["light", "dark"]) for (const fontSize of fontSizes) {
      await page.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
      for (const subtitleDisplayMode of ["original", "translation", "bilingual"]) {
        const issues = await page.evaluate(async next => {
          await window.applyLayoutCase({
            ...next, platform: "windows", category: "subtitle-settings", state: "idle",
            subtitleAppearance: { fontSize: next.fontSize, subtitleDisplayMode: next.subtitleDisplayMode },
          });
          await document.fonts.ready;
          await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
          const issues = [];
          const preview = document.querySelector(".subtitle-preview__text");
          if (!preview) return ["preview missing"];
          const original = preview.querySelector("span");
          const translation = preview.querySelector("strong");
          if (Boolean(original) !== (next.subtitleDisplayMode !== "translation")
            || Boolean(translation) !== (next.subtitleDisplayMode !== "original")) issues.push("preview display mode");
          for (const [lane, reference] of [[original, next.subtitleDisplayMode === "bilingual"], [translation, false]]) {
            if (!lane) continue;
            const size = next.fontSize * (reference ? 0.9 : 1);
            const style = getComputedStyle(lane);
            if (Math.abs(parseFloat(style.fontSize) - size) > 0.05) issues.push("preview font size");
            if (parseFloat(style.lineHeight) !== Math.ceil(size * 1.32)) issues.push("preview line height");
            if (Number(style.fontWeight) !== (reference ? 400 : 500)) issues.push("preview weight");
            const range = document.createRange();
            range.selectNodeContents(lane);
            const box = preview.getBoundingClientRect();
            if ([...range.getClientRects()].some(rect => rect.left < box.left - 1 || rect.right > box.right + 1
              || rect.top < box.top - 1 || rect.bottom > box.bottom + 1)) issues.push("preview glyphs clipped");
          }
          for (const row of document.querySelectorAll("#subtitle-settings-panel .settings-row, .subtitle-preview__controls .settings-row")) {
            if (row.scrollWidth > row.clientWidth + 1) issues.push("setting row overflows");
          }
          const slider = document.querySelector('.font-size-control input[type="range"]');
          if (!slider || slider.min !== "14" || slider.max !== "48" || Number(slider.value) !== next.fontSize) issues.push("font slider range/value");
          if (document.documentElement.scrollWidth > innerWidth + 1) issues.push("horizontal page overflow");
          return [...new Set(issues)];
        }, { language, width, theme, fontSize, subtitleDisplayMode });
        checked++;
        if (issues.length) failures.push({ language, width, theme, fontSize, subtitleDisplayMode, issues });
      }
    }
  }
  return { checked, failures };
}

export async function verifySubtitleOverlayLayout(page, baseUrl = "http://127.0.0.1:1420", {
  languages = ["zh", "en", "ja"], widths = [360, 640],
  fontSizes = [16, 20, 24, 32, 40, 48],
  audioInputs = ["system", "microphone", "both"],
  subtitleStates = ["confirmed", "live"],
  dividers = true,
} = {}) {
  if (!Array.isArray(subtitleStates) || subtitleStates.length === 0
    || subtitleStates.some(state => state !== "confirmed" && state !== "live")) {
    throw new Error("subtitleStates must contain confirmed and/or live");
  }
  await page.goto(`${baseUrl}/scripts/fixtures/overlay-tooltip.html`);
  await page.waitForFunction(() => window.tooltipReady === true);
  await page.cdp("Page.bringToFront", {});
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: true });
  const failures = [];
  let checked = 0;
  for (const language of languages) for (const width of widths) for (const fontSize of fontSizes) {
    for (const subtitleDisplayMode of ["original", "translation", "bilingual"]) {
      for (const audioInput of audioInputs) for (const showSubtitleTimestamps of [false, true]) {
        for (const subtitleBlendsWithBackground of [false, true]) for (const subtitleState of subtitleStates) {
          const next = { language, width, fontSize, subtitleDisplayMode, audioInput, showSubtitleTimestamps, subtitleBlendsWithBackground, subtitleState, showSubtitleDividers: dividers };
          const minimumHeight = await page.evaluate(async next => {
            window.renderTooltipFixture(next.language, false, next.audioInput);
            const loadedModule = path => {
              const entries = performance.getEntriesByType("resource").filter(entry => new URL(entry.name).pathname === path);
              return import(entries.filter(entry => new URL(entry.name).search).at(-1)?.name ?? entries.at(-1)?.name ?? path);
            };
            const { useStore } = await loadedModule("/src/lib/store.ts");
            const { minimumOverlayHeight } = await loadedModule("/src/windows/overlay/overlayMinimumHeight.ts");
            const settings = { ...useStore.getState().settings, fontSize: next.fontSize,
              subtitleDisplayMode: next.subtitleDisplayMode, audioInput: next.audioInput,
              showSubtitleTimestamps: next.showSubtitleTimestamps,
              subtitleBlendsWithBackground: next.subtitleBlendsWithBackground,
              showSubtitleDividers: next.showSubtitleDividers, showIntermediateSubtitles: true,
              sourceLanguage: "auto", targetLanguage: "zh",
              subtitleAlignment: "center", activeProfileId: "readability-fixture",
              profiles: [{ id: "readability-fixture", name: "Synthetic configuration", provider: "alibabaCloud",
                alibabaRealtimeMode: "disabled", textTranslation: "deepL", credentialState: "present" }] };
            const pairs = [
              { audioSource: next.audioInput === "microphone" ? "microphone" : "system",
                source: next.language === "ja" ? "休憩" : next.language === "zh" ? "今天" : "Hello", translation: "你好", createdAt: 1700000000000 },
              ...(next.audioInput === "both" ? [{ audioSource: "microphone", source: "Mic", translation: "收到", createdAt: 1700000001000 }] : []),
            ];
            const confirmed = next.subtitleState === "confirmed";
            // Alibaba ASR + text MT publishes a complete replaceable preview.
            // The pair's owner and confirmation flag are independent of whether
            // both lanes already contain text. Keep live pairs out of history.
            const snapshots = pairs.map(pair => {
              const utteranceId = `readability-${next.subtitleState}-${pair.audioSource}`;
              const displayPair = { source: pair.source, translation: pair.translation, utteranceId };
              return {
                source: { text: pair.source, isFinal: confirmed, utteranceId },
                translation: { text: pair.translation, isFinal: confirmed, utteranceId },
                history: confirmed ? [pair] : [],
                previewPair: confirmed ? null : displayPair,
                displayPair,
                displayPairFinal: confirmed,
                ...(confirmed ? { displayPairCreatedAt: pair.createdAt } : {}),
              };
            });
            const tracks = snapshots.map((snapshot, index) => ({
              ...snapshot, audioSource: pairs[index].audioSource, detectedLanguage: "en",
              isTranslationPending: false, isTranslationPreviewPending: false, isTranslationTimedOut: false,
            }));
            useStore.setState(state => ({
              settings,
              session: { ...state.session, status: { kind: "listening" }, isActive: true, isPaused: false,
                isOverlayCollapsed: false, atomicPreview: true, detectedLanguage: "en",
                isTranslationPending: false, isTranslationPreviewPending: false, isTranslationTimedOut: false,
                apiLatencyMs: 128, translationLatencyMs: 246, translationLatencyKind: "request",
                // A single input reads the top-level snapshot; dual input reads
                // its independent tracks and the chronological common history.
                subtitles: { ...snapshots[0], history: confirmed ? pairs : [], tracks } },
            }));
            window.readabilityExpected = pairs.flatMap(pair => [
              ...(next.subtitleDisplayMode !== "translation" ? [{ text: pair.source, reference: next.subtitleDisplayMode === "bilingual" }] : []),
              ...(next.subtitleDisplayMode !== "original" ? [{ text: pair.translation, reference: false }] : []),
            ]);
            return minimumOverlayHeight(settings);
          }, next);
          await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: minimumHeight, deviceScaleFactor: 1, mobile: false });
          const issues = await page.evaluate(async next => {
            await document.fonts.ready;
            await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(resolve))));
            const issues = [];
            const timeline = document.querySelector(".overlay-timeline");
            if (!timeline) return ["timeline missing"];
            const box = timeline.getBoundingClientRect();
            const rows = [...timeline.querySelectorAll("[data-utterance-id]")];
            const sourceCount = next.audioInput === "both" ? 2 : 1;
            if (rows.length !== sourceCount) issues.push("subtitle source row count");
            if (rows.some(row => row.getAttribute("data-utterance-id").includes("live-utterance-")
              !== (next.subtitleState === "live"))) issues.push("subtitle confirmation state");
            const separatorCount = next.showSubtitleDividers && !next.subtitleBlendsWithBackground ? sourceCount - 1 : 0;
            if (timeline.querySelectorAll(".subtitle-separator").length !== separatorCount) issues.push("subtitle separator count");
            const timestampCount = next.subtitleState === "confirmed" && next.showSubtitleTimestamps ? sourceCount : 0;
            if (timeline.querySelectorAll(".subtitle-timestamp").length !== timestampCount) issues.push("subtitle timestamp state");
            for (const expected of window.readabilityExpected) {
              const spans = [...timeline.querySelectorAll("span")].filter(node => !node.childElementCount && node.textContent === expected.text);
              if (spans.length !== 1) { issues.push(`missing lane ${expected.text}`); continue; }
              const span = spans[0], style = getComputedStyle(span);
              const size = next.fontSize * (expected.reference ? 0.9 : 1);
              if (Math.abs(parseFloat(style.fontSize) - size) > 0.05) issues.push("live font size");
              const range = document.createRange();
              range.selectNodeContents(span);
              const glyphs = [...range.getClientRects()];
              if (!glyphs.length || glyphs.some(rect => rect.left < box.left - 1 || rect.right > box.right + 1
                || rect.top < box.top - 1 || rect.bottom > box.bottom + 1)) issues.push(`clipped lane ${expected.text}`);
              for (let ancestor = span.parentElement; ancestor && ancestor !== timeline; ancestor = ancestor.parentElement) {
                const overflow = getComputedStyle(ancestor).overflowY;
                if (overflow === "hidden" || overflow === "clip") {
                  const bounds = ancestor.getBoundingClientRect();
                  if (glyphs.some(rect => rect.top < bounds.top - 1 || rect.bottom > bounds.bottom + 1)) issues.push(`inner clip ${expected.text}`);
                }
              }
            }
            if (!next.subtitleBlendsWithBackground) {
              const dragArea = document.querySelector('[data-testid="overlay-drag-area"]');
              if (!dragArea) issues.push("top drag area missing");
              if (!document.querySelector(".overlay-service__button")) issues.push("configured service missing");
              if (document.querySelectorAll(".overlay-service__stage").length !== 2) issues.push("configured ASR/MT stages missing");
              for (const button of document.querySelectorAll(".overlay-service__button, .overlay-control-button, [data-testid='drag-handle']")) {
                const rect = button.getBoundingClientRect();
                if (rect.width > 0 && rect.height > 0 && !button.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2))) {
                  issues.push("top control obstructed");
                }
              }
            }
            return [...new Set(issues)];
          }, next);
          checked++;
          if (issues.length) failures.push({ ...next, minimumHeight, issues });
        }
      }
    }
  }
  return { checked, failures };
}
