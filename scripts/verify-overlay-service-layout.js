// Browser harness for the actual overlay components. Start Vite on port 1420,
// then pass this complete function to the project's supported browser page API.
// Synthetic state only; this does not verify native window or capture behavior.
async (page) => {
  await page.goto("http://127.0.0.1:1420/?window=overlay");

  async function configure({ language = "en", route = "deepL", audioInput = "system", timestamps = false }) {
    return page.evaluate(async ({ language, route, audioInput, timestamps }) => {
      const { useStore } = await import("/src/lib/store.ts");
      const { setStoredUiLanguage } = await import("/src/lib/i18n.ts");
      const { minimumOverlayHeight } = await import("/src/windows/overlay/overlayMinimumHeight.ts");
      setStoredUiLanguage(language);
      // Short lanes distinguish clipping from normal compact long-text layout.
      const history = [
        { audioSource: "system", source: "Synthetic S", translation: "Synthetic T", createdAt: 1_700_000_000_000 },
        ...(audioInput === "both" ? [
          { audioSource: "microphone", source: "Mic S", translation: "Mic T", createdAt: 1_700_000_001_000 },
        ] : []),
      ];
      const emptyLine = { text: "", isFinal: false };
      const tracks = history.map(pair => ({
        audioSource: pair.audioSource,
        source: emptyLine, translation: emptyLine, history: [pair],
        detectedLanguage: "en", isTranslationPending: false, isTranslationTimedOut: false,
      }));
      useStore.setState(state => ({
        settings: {
          ...state.settings,
          uiLanguage: language, sourceLanguage: "en", targetLanguage: route === "original" ? "original" : "zh",
          audioInput, fontSize: 20, subtitleDisplayMode: route === "original" ? "original" : "bilingual",
          subtitleAlignment: "center", showSubtitleTimestamps: timestamps, showSubtitleDividers: false,
          subtitleBlendsWithBackground: false, isOverlayLocked: false,
          pulseAnimation: false, subtitleAnimation: false,
          activeProfileId: "preview",
          profiles: [{
            id: "preview", name: "Long configuration name · 長い設定名 · 较长的配置名称",
            provider: route === "followService" ? "openAIRealtime" : "alibabaCloud",
            textTranslation: route === "original" ? "deepL" : route, credentialState: "present",
          }],
        },
        session: {
          ...state.session,
          isActive: true, isPaused: false, isOverlayCollapsed: false, status: { kind: "listening" },
          apiLatencyMs: 128, translationLatencyMs: 246, translationLatencyKind: "request",
          detectedLanguage: "en", isTranslationPending: false, isTranslationPreviewPending: false,
          isTranslationTimedOut: false, translationRecovery: null,
          subtitles: { source: emptyLine, translation: emptyLine, history, tracks },
        },
      }));
      await document.fonts.ready;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return minimumOverlayHeight(useStore.getState().settings);
    }, { language, route, audioInput, timestamps });
  }

  async function resize(width, height) {
    const from = await page.evaluate(() => {
      const handle = [...document.querySelectorAll('[style*="resize"]')].find(element =>
        element.style.right === "0px" && element.style.bottom === "0px" && element.style.width === "14px");
      if (!handle) throw new Error("Overlay bottom-right resize handle is missing");
      const rect = handle.getBoundingClientRect();
      const overlay = handle.parentElement.getBoundingClientRect();
      return { x: rect.x + 7, y: rect.y + 7, width: overlay.width, height: overlay.height };
    });
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(from.x + width - from.width, from.y + height - from.height);
    await page.mouse.up();
    // ResizeObserver and compact-lane measurement each trigger another render.
    await page.evaluate(async () => {
      for (let frame = 0; frame < 4; frame++) await new Promise(requestAnimationFrame);
    });
  }

  await configure({});
  await page.waitForSelector(".overlay-service__button");
  const failures = [];
  let chromeChecked = 0;
  const subtitleCases = [];

  for (const width of [360, 420, 552, 640]) {
    for (const language of ["zh", "en", "ja"]) {
      for (const route of ["followService", "deepL", "deepLX", "chatMock", "openAICompatible", "original"]) {
        const minimumHeight = await configure({ language, route });
        await resize(width, minimumHeight);
        const result = await page.evaluate(route => {
          const rect = selector => document.querySelector(selector)?.getBoundingClientRect();
          const service = rect(".overlay-service__button");
          const drag = rect('[data-testid="drag-handle"]');
          const icon = rect(".overlay-service .provider-icon");
          const name = document.querySelector(".overlay-service__name");
          const latency = document.querySelector(".overlay-latency");
          const issues = [];
          if (!service || !drag || !name || !latency) return { issues: ["overlay chrome missing"] };
          if (service.width < 28 || (icon && icon.width !== 16)) issues.push("icon squeezed");
          if (service.left < drag.right && service.right > drag.left && service.top < drag.bottom && service.bottom > drag.top) {
            issues.push("service overlaps drag");
          }
          if (latency.scrollWidth > latency.clientWidth + 1) issues.push("latency clipped");
          if (["deepL", "deepLX", "chatMock"].includes(route) && name.scrollWidth > name.clientWidth + 1) {
            issues.push("short name clipped");
          }
          return { issues, label: name.textContent };
        }, route);
        chromeChecked++;
        if (result.issues.length) failures.push({ check: "chrome", width, minimumHeight, language, route, ...result });
      }
    }
  }

  for (const width of [360, 420]) {
    for (const language of ["zh", "en", "ja"]) {
      for (const { audioInput, timestamps } of [
        { audioInput: "system", timestamps: false },
        { audioInput: "system", timestamps: true },
        { audioInput: "both", timestamps: true },
      ]) {
        const minimumHeight = await configure({ language, route: "deepL", audioInput, timestamps });
        await resize(width, minimumHeight);
        const result = await page.evaluate(({ width, minimumHeight, audioInput, timestamps }) => {
          const issues = [];
          const timeline = document.querySelector(".overlay-timeline");
          if (!timeline) return { issues: ["subtitle timeline missing"] };
          const visibleBox = element => {
            const rect = element.getBoundingClientRect();
            const left = rect.left + element.clientLeft;
            const top = rect.top + element.clientTop;
            return { left, top, right: left + element.clientWidth, bottom: top + element.clientHeight };
          };
          const outside = (rect, box) => rect.left < box.left - 1 || rect.top < box.top - 1
            || rect.right > box.right + 1 || rect.bottom > box.bottom + 1;
          const box = visibleBox(timeline);
          const expected = ["Synthetic S", "Synthetic T", ...(audioInput === "both" ? ["Mic S", "Mic T"] : [])];
          const textBounds = [];
          for (const text of expected) {
            // Test the actual text span and glyph range, not only the compact
            // wrapper, which can fit while clipping the text inside it.
            const matches = [...timeline.querySelectorAll("span")].filter(element =>
              element.childElementCount === 0 && element.textContent === text);
            if (matches.length !== 1) {
              issues.push(`${text}: expected one text span, found ${matches.length}`);
              continue;
            }
            const element = matches[0];
            const range = document.createRange();
            range.selectNodeContents(element);
            const glyphRects = [...range.getClientRects()];
            const textRect = element.getBoundingClientRect();
            textBounds.push({ text, top: textRect.top, bottom: textRect.bottom });
            if (glyphRects.length !== 1) issues.push(`${text}: short synthetic text wrapped or is absent`);
            if (textRect.width <= 0 || textRect.height <= 0 || outside(textRect, box)
              || glyphRects.some(rect => outside(rect, box))) issues.push(`${text}: outside visible timeline`);
            for (let ancestor = element; ancestor && ancestor !== timeline; ancestor = ancestor.parentElement) {
              const style = getComputedStyle(ancestor);
              if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) {
                issues.push(`${text}: hidden by an ancestor`);
              }
              if (/(hidden|clip|auto|scroll)/.test(`${style.overflowX} ${style.overflowY}`)
                && glyphRects.some(rect => outside(rect, visibleBox(ancestor)))) {
                issues.push(`${text}: clipped by a lane ancestor`);
              }
            }
          }
          const times = [...timeline.querySelectorAll(".subtitle-timestamp")];
          if (times.length !== (timestamps ? audioInput === "both" ? 2 : 1 : 0)) issues.push("unexpected timestamp count");
          if (times.some(time => outside(time.getBoundingClientRect(), box))) issues.push("timestamp outside visible timeline");
          const handle = [...document.querySelectorAll('[style*="resize"]')].find(element =>
            element.style.right === "0px" && element.style.bottom === "0px" && element.style.width === "14px");
          const overlay = handle?.parentElement.getBoundingClientRect();
          if (!overlay || Math.abs(overlay.width - width) > 1 || Math.abs(overlay.height - minimumHeight) > 1) {
            issues.push("overlay did not resize to the derived minimum");
          }
          return { issues, actualHeight: overlay?.height, timeline: box, textBounds };
        }, { width, minimumHeight, audioInput, timestamps });
        const context = { width, minimumHeight, language, audioInput, timestamps, fontSize: 20 };
        subtitleCases.push({ ...context, ...result });
        if (result.issues.length) failures.push({ check: "subtitle", ...context, ...result });
      }
    }
  }

  const result = { checked: chromeChecked + subtitleCases.length, chromeChecked, subtitleChecked: subtitleCases.length, subtitleCases, failures };
  console.log(JSON.stringify(result));
  if (failures.length) throw new Error(JSON.stringify(failures));
  return result;
}
