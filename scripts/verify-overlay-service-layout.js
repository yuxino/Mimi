// Browser harness for the actual overlay components. Start Vite on port 1420,
// then pass this complete function to the project's supported browser page API.
// Synthetic state only; this does not verify native window or capture behavior.
async (page, { baseUrl = "http://127.0.0.1:1420", widths = [360, 420, 552, 640], languages = ["zh", "zh-TW", "en", "ja", "de", "fr", "ko", "th", "ru"], themes = ["dark", "light"], serviceCases, checkSubtitleLanes = true } = {}) => {
  await page.goto(`${baseUrl}/?window=overlay`);
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: true });
  await page.waitForSelector(".overlay-surface");

  async function configure({ language = "en", route = "deepL", audioInput = "system", timestamps = false, provider, alias, speechRecognitionName, theme = "dark", hasCaptions = true, longHistory = false }) {
    await page.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
    return page.evaluate(async ({ language, route, audioInput, timestamps, provider, alias, speechRecognitionName, hasCaptions, longHistory }) => {
      if (typeof window.__TAURI_INTERNALS__ !== "undefined") throw new Error("Memory-only browser preview required");
      // Vite may keep timestamped dependency imports even after page reload.
      // Import the actual module already loaded by the mounted UI; importing
      // the same path without its query creates a second Zustand/language store.
      const mountedModule = path => {
        const loaded = performance.getEntriesByType("resource")
          .filter(entry => new URL(entry.name).pathname === path);
        const latestVersion = loaded.filter(entry => new URL(entry.name).search).at(-1);
        return import(latestVersion?.name ?? loaded.at(-1)?.name ?? path);
      };
      const { useStore } = await mountedModule("/src/lib/store.ts");
      const { setStoredUiLanguage } = await mountedModule("/src/lib/i18n.ts");
      const { minimumOverlayHeight } = await import("/src/windows/overlay/overlayMinimumHeight.ts");
      const { translationService } = await mountedModule("/src/windows/overlay/translationService.ts");
      setStoredUiLanguage(language);
      // Short lanes distinguish clipping from normal compact long-text layout.
      const history = hasCaptions ? [
        { audioSource: audioInput === "microphone" ? "microphone" : "system", source: "Synthetic S", translation: "Synthetic T", createdAt: 1_700_000_000_000 },
        ...(longHistory ? Array.from({ length: 7 }, (_, i) => ({
          audioSource: "system",
          source: "Original sample " + (i + 1),
          translation: "Translated sample " + (i + 1),
          createdAt: 1_700_000_001_000 + i * 1000,
        })) : []),
        ...(audioInput === "both" ? [
          { audioSource: "microphone", source: "Mic S", translation: "Mic T", createdAt: 1_700_000_001_000 },
        ] : []),
      ] : [];
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
          pulseAnimation: false, subtitleAnimation: false, customSubtitleCountEnabled: false,
          activeProfileId: "preview",
          profiles: [{
            id: "preview", name: "Long configuration name · 長い設定名 · 较长的配置名称",
            provider: provider ?? (route === "followService" ? "openAIRealtime" : "alibabaCloud"),
            textTranslation: route === "original" ? "deepL" : route, credentialState: "present",
            textTranslationNames: alias ? { [route]: alias } : undefined,
            speechRecognitionName,
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
      const expectedService = translationService(useStore.getState().settings);
      const expectedTexts = history.flatMap(pair => route === "original" ? [pair.source] : [pair.source, pair.translation]);
      let fixtureState;
      // Verify the actual screen before measuring geometry. An unrelated HMR
      // reload must not turn an initial/default screen into a passing fixture.
      for (let frame = 0; frame < 12; frame++) {
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        const service = document.querySelector(".overlay-service__button");
        const actualLabels = [...document.querySelectorAll(".overlay-service__name")].map(node => node.textContent);
        const timeline = document.querySelector(".overlay-timeline");
        const actualTexts = [...(timeline?.querySelectorAll("span") ?? [])]
          .filter(element => element.childElementCount === 0).map(element => element.textContent);
        fixtureState = {
          expectedLabels: expectedService?.stages.map(stage => stage.label), actualLabels,
          profileMatches: service?.getAttribute("aria-label") === expectedService?.detail,
          activeChrome: Boolean(document.querySelector(".overlay-latency")),
          timelinePresent: Boolean(timeline),
          missingTexts: expectedTexts.filter(text => !actualTexts.includes(text)),
        };
        if (fixtureState.profileMatches && fixtureState.activeChrome && fixtureState.timelinePresent === hasCaptions
          && actualLabels.join("\n") === expectedService.stages.map(stage => stage.label).join("\n") && fixtureState.missingTexts.length === 0) {
          return minimumOverlayHeight(useStore.getState().settings);
        }
      }
      throw new Error(`Overlay fixture did not reach the mounted UI. Restart Vite and reload after HMR/rebase changes before retrying. ${JSON.stringify({ language, route, audioInput, timestamps, ...fixtureState })}`);
    }, { language, route, audioInput, timestamps, provider, alias, speechRecognitionName, hasCaptions, longHistory });
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
    if (Math.abs(from.width - width) <= 1 && Math.abs(from.height - height) <= 1) return;
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(from.x + width - from.width, from.y + height - from.height);
    await page.mouse.up();
    // ResizeObserver and compact-lane measurement each trigger another render.
    await page.evaluate(async () => {
      for (let frame = 0; frame < 4; frame++) await new Promise(requestAnimationFrame);
    });
  }

  await configure({ longHistory: true });
  await resize(580, 240);

  async function captionViewport(remember = false) {
    return page.evaluate(async remember => {
      for (let frame = 0; frame < 4; frame++) await new Promise(requestAnimationFrame);
      const timeline = document.querySelector(".overlay-timeline");
      if (remember) window.__timelineUnderReview = timeline;
      const { top, bottom } = timeline.getBoundingClientRect();
      const live = [...timeline.querySelectorAll("span")].find(node =>
        node.childElementCount === 0 && node.textContent === "Translated sample 7").getBoundingClientRect();
      return {
        top, bottom,
        sameTimeline: window.__timelineUnderReview === timeline,
        overflow: timeline.scrollHeight - timeline.clientHeight,
        distanceFromLive: timeline.scrollHeight - timeline.clientHeight - timeline.scrollTop,
        liveBottom: live.bottom,
      };
    }, remember);
  }

  await page.hover(".overlay-surface");
  const hoveredAfterBlur = await page.evaluate(() => {
    // Focus loss must not hide the controls while the pointer stays inside.
    window.dispatchEvent(new Event("blur"));
    const surface = document.querySelector(".overlay-surface");
    const pause = surface?.querySelector('button[aria-label="Pause translating"]');
    const rect = pause?.getBoundingClientRect();
    return {
      hover: surface?.matches(":hover"),
      nativeHover: surface?.hasAttribute("data-overlay-hover"),
      visible: pause && getComputedStyle(pause.closest(".overlay-hover-control")).opacity === "1",
      hit: rect && pause.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2)),
    };
  });
  if (!hoveredAfterBlur.hover || !hoveredAfterBlur.visible || !hoveredAfterBlur.hit) {
    throw new Error(`Hovered pause became unavailable after blur: ${JSON.stringify(hoveredAfterBlur)}`);
  }
  const hoveredViewport = await captionViewport(true);
  await page.mouse.move(4, 4, { label: "Move outside subtitle overlay" });
  const hiddenAfterLeave = await page.evaluate(() => {
    const surface = document.querySelector(".overlay-surface");
    const pause = surface?.querySelector('button[aria-label="Pause translating"]');
    const rect = pause?.getBoundingClientRect();
    return !surface?.matches(":hover") && pause
      && getComputedStyle(pause.closest(".overlay-hover-control")).opacity === "0"
      && !pause.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
  });
  if (!hiddenAfterLeave) throw new Error("Pause remained visible or clickable after pointer exit.");
  const quietViewport = await captionViewport();
  if (hoveredViewport.top - quietViewport.top < 34
    || Math.abs(hoveredViewport.bottom - quietViewport.bottom) > 1
    || !quietViewport.sameTimeline
    || [hoveredViewport, quietViewport].some(view => view.overflow <= 1 || Math.abs(view.distanceFromLive) > 2)
    || Math.abs(hoveredViewport.liveBottom - quietViewport.liveBottom) > 2) {
    throw new Error("Quiet chrome did not give its space to captions while keeping the latest caption in place: "
      + JSON.stringify({ hoveredViewport, quietViewport }));
  }
  async function historyViewport() {
    return page.evaluate(async () => {
      for (let frame = 0; frame < 4; frame++) await new Promise(requestAnimationFrame);
      const timeline = document.querySelector(".overlay-timeline");
      const top = timeline.getBoundingClientRect().top;
      const first = [...timeline.children].find(row => row.getBoundingClientRect().bottom > top);
      const back = document.querySelector(".overlay-return-to-live");
      return {
        top, id: first?.dataset.utteranceId, offset: first?.getBoundingClientRect().top - top,
        backBottom: back?.getBoundingClientRect().bottom,
      };
    });
  }
  await page.hover(".overlay-timeline");
  const readingPoint = await page.evaluate(() => {
    const { x, y, width, height } = document.querySelector(".overlay-timeline").getBoundingClientRect();
    return { x: Math.round(x + width / 2), y: Math.round(y + height / 2) };
  });
  // Use a single deterministic wheel input; smooth native gestures may still
  // be dispatched while the pointer moves outside the test overlay.
  await page.cdp("Input.dispatchMouseEvent", {
    type: "mouseWheel", ...readingPoint, deltaX: 0, deltaY: -140,
  });
  const hoveredHistory = await historyViewport();
  await page.mouse.move(4, 4, { label: "Hide tools while reading" });
  const quietHistory = await historyViewport();
  if (!hoveredHistory.id || hoveredHistory.id !== quietHistory.id
    || Math.abs(hoveredHistory.offset - quietHistory.offset) > 2
    || hoveredHistory.top - quietHistory.top < 28
    || quietHistory.backBottom === undefined || quietHistory.backBottom > quietHistory.top) {
    throw new Error("Hiding tools moved the reading anchor or covered the return action: "
      + JSON.stringify({ hoveredHistory, quietHistory }));
  }
  await configure({ hasCaptions: false });
  const listeningAfterLeave = await page.evaluate(() => {
    const surface = document.querySelector(".overlay-surface");
    const wave = surface?.querySelector(".overlay-listening-wave");
    const status = wave?.nextElementSibling;
    const isVisible = element => {
      if (!element) return false;
      const { width, height } = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      return width > 0 && height > 0 && style.visibility === "visible" && style.opacity !== "0";
    };
    return surface && !surface.matches(":hover") && !surface.hasAttribute("data-overlay-hover")
      && isVisible(wave) && isVisible(status) && status.textContent.trim() === "Listening";
  });
  if (!listeningAfterLeave) throw new Error("Listening wave or status was hidden with the pointer outside the overlay.");
  await page.hover(".overlay-surface");
  await page.waitForSelector(".overlay-service__button");
  const failures = [];
  let chromeChecked = 0;
  const subtitleCases = [];

  for (const width of widths) {
    for (const language of languages) {
      for (const theme of themes) {
        for (const candidate of serviceCases ?? [
          ...["followService", "deepL", "deepLX", "chatMock", "openAICompatible", "original"].map(route => ({ route })),
          { route: "openAICompatible", alias: "B 站 / long translator name with spaces 日本語 🌸 ".repeat(2) },
          { route: "deepL", provider: "customOpenAIASR" },
          { route: "followService", provider: "customDashScopeASR" },
          { route: "openAICompatible", provider: "customDashScopeASR", speechRecognitionName: "Whisper", alias: "Index · 本地" },
          { route: "chatMock", provider: "customOpenAIASR", speechRecognitionName: "会議の音声認識 / 长中文识别服务名称 🌸 ".repeat(2), alias: "Local translator" },
        ]) {
          const minimumHeight = await configure({ language, theme, ...candidate });
          await resize(width, minimumHeight);
          // Measurements and service labels are revealed only while using
          // the expanded canvas. Move inside after resizing from an edge.
          await page.hover(".overlay-surface");
          const result = await page.evaluate(() => {
            const rect = selector => document.querySelector(selector)?.getBoundingClientRect();
            const service = rect(".overlay-service__button");
            const drag = rect('[data-testid="overlay-drag-area"]');
            const icons = [...document.querySelectorAll(".overlay-service .provider-icon")];
            const names = [...document.querySelectorAll(".overlay-service__name")];
            const latency = document.querySelector(".overlay-latency");
            const issues = [];
            if (!service || !drag || !names.length || !latency) return { issues: ["overlay chrome missing"] };
            if (service.width < 28 || icons.some(icon => Math.abs(icon.getBoundingClientRect().width - 16) > 0.5)) issues.push("icon squeezed");
            if (service.left < drag.right && service.right > drag.left && service.top < drag.bottom && service.bottom > drag.top) {
              issues.push("service overlaps drag");
            }
            const timing = latency.getBoundingClientRect();
            if (latency.scrollWidth > latency.clientWidth + 1) issues.push("latency clipped");
            if (timing.right > service.left + 1 && timing.top < service.bottom && timing.bottom > service.top) issues.push("timing and services overlap");
            if (service.right > innerWidth || service.height > 20.5) issues.push("service overflow or wrapping");
            for (const name of names) {
              const style = getComputedStyle(name), box = name.getBoundingClientRect();
              if (style.whiteSpace !== "nowrap" || style.textOverflow !== "ellipsis" || box.height > 16.5) issues.push("name wraps");
              if (box.width < 22) issues.push("service name squeezed beyond recognition");
              if (box.left < service.left || box.right > service.right + 1) issues.push("name escapes button");
            }
            const naturalWidth = names.reduce((sum, name) => sum + name.scrollWidth, 0)
              + (names.length === 2 ? 66 : 28);
            if (service.width + 1 >= naturalWidth && names.some(name => name.scrollWidth > name.clientWidth + 1)) {
              issues.push("name clipped despite sufficient combined width");
            }
            return { issues, labels: names.map(name => name.textContent), nameWidths: names.map(name => name.clientWidth) };
          });
          chromeChecked++;
          if (result.issues.length) failures.push({ check: "chrome", width, minimumHeight, language, theme, ...candidate, ...result });
        }
      }
    }
    console.log(JSON.stringify({ progress: "chrome width complete", width, chromeChecked, failures: failures.length }));
  }

  for (const width of widths.filter(width => checkSubtitleLanes && width < 552)) {
    for (const language of languages) {
      for (const { audioInput, timestamps } of [
        { audioInput: "system", timestamps: false },
        { audioInput: "system", timestamps: true },
        { audioInput: "microphone", timestamps: false },
        { audioInput: "microphone", timestamps: true },
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
              const ancestorBox = visibleBox(ancestor);
              const clipsX = /^(hidden|clip|auto|scroll)$/.test(style.overflowX);
              const clipsY = /^(hidden|clip|auto|scroll)$/.test(style.overflowY);
              if (glyphRects.some(rect =>
                clipsX && (rect.left < ancestorBox.left - 1 || rect.right > ancestorBox.right + 1)
                || clipsY && (rect.top < ancestorBox.top - 1 || rect.bottom > ancestorBox.bottom + 1))) {
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
    console.log(JSON.stringify({ progress: "subtitle width complete", width, subtitleChecked: subtitleCases.length, failures: failures.length }));
  }

  await page.cdp("Emulation.setEmulatedMedia", { features: [] });
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: false });
  const result = { checked: chromeChecked + subtitleCases.length, chromeChecked, subtitleChecked: subtitleCases.length, subtitleCases, failures };
  console.log(JSON.stringify(result));
  if (failures.length) throw new Error(JSON.stringify(failures));
  return result;
}
