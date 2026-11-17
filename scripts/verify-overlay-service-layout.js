// Browser harness for the actual overlay components. Start Vite on port 1420,
// then pass this complete function to the project's supported browser page API.
// Synthetic state only; this does not verify native window or capture behavior.
async (page, { baseUrl = "http://127.0.0.1:1420", widths = [360, 420, 552, 640], languages = ["zh", "zh-TW", "en", "ja", "de", "fr", "ko", "th", "ru"], themes = ["dark", "light"], serviceCases, checkSubtitleLanes = true } = {}) => {
  await page.goto(`${baseUrl}/?window=overlay`);
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: true });
  await page.waitForSelector(".overlay-surface");

  async function configure({ language = "en", route = "deepL", audioInput = "system", timestamps = false, provider, alias, speechRecognitionName, profileName, theme = "dark", hasCaptions = true, longHistory = false, surface = "overlay", rejectStop = false }) {
    await page.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
    return page.evaluate(async ({ language, route, audioInput, timestamps, provider, alias, speechRecognitionName, profileName, hasCaptions, longHistory, surface, rejectStop }) => {
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
      const { effectiveUiLanguage, setStoredUiLanguage } = await mountedModule("/src/lib/i18n.ts");
      const { minimumOverlayHeight } = await import("/src/windows/overlay/overlayMinimumHeight.ts");
      const { translationService } = surface === "panel"
        ? await mountedModule("/src/windows/overlay/translationService.ts") : {};
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
        ...(rejectStop ? { stop: async () => { throw new Error("Synthetic stop failure"); } } : {}),
        ...(surface === "panel" ? { initializationStatus: "ready" } : {}),
        settings: {
          ...state.settings,
          uiLanguage: language, sourceLanguage: "en", targetLanguage: route === "original" ? "original" : "zh",
          audioInput, fontSize: 20, subtitleDisplayMode: route === "original" ? "original" : "bilingual",
          microphoneInputAvailable: surface === "panel" || state.settings.microphoneInputAvailable,
          subtitleAlignment: "center", showSubtitleTimestamps: timestamps, showSubtitleDividers: false,
          subtitleBlendsWithBackground: false, isOverlayLocked: false,
          pulseAnimation: false, subtitleAnimation: false, customSubtitleCountEnabled: false,
          activeProfileId: "preview",
          profiles: [{
            id: "preview", name: profileName ?? "Long configuration name · 長い設定名 · 较长的配置名称",
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
      const selectedService = translationService?.(useStore.getState().settings);
      const expectedService = selectedService?.showInProfileDetails ? selectedService : undefined;
      const expectedTexts = history.flatMap(pair => route === "original" ? [pair.source] : [pair.source, pair.translation]);
      let fixtureState;
      // Verify the actual screen before measuring geometry. An unrelated HMR
      // reload must not turn an initial/default screen into a passing fixture.
      for (let frame = 0; frame < 12; frame++) {
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        if (surface === "panel") {
          const details = document.querySelector(".overlay-session-details");
          const service = details?.querySelector(".overlay-service__button");
          const actualLabels = [...(details?.querySelectorAll(".overlay-service__name") ?? [])].map(node => node.textContent);
          const expectedLabels = expectedService?.stages.map(stage => stage.label) ?? [];
          const profileMatches = expectedService ? service?.getAttribute("aria-label") === expectedService.detail : !service;
          fixtureState = { expectedLabels, actualLabels, profileMatches, panelPresent: Boolean(details) };
          if (details && profileMatches && actualLabels.join("\n") === expectedLabels.join("\n")
            && details.querySelector(".overlay-latency strong")?.textContent === "128 ms" && effectiveUiLanguage() === language) return { showServiceDetails: Boolean(expectedService) };
          continue;
        }
        const canvas = document.querySelector(".overlay-expanded-canvas");
        const timeline = canvas?.querySelector(".overlay-timeline");
        const actualTexts = [...(timeline?.querySelectorAll("span") ?? [])]
          .filter(element => element.childElementCount === 0).map(element => element.textContent);
        fixtureState = {
          activeChrome: Boolean(canvas?.querySelector(".overlay-action-strip")),
          noServiceDetails: Boolean(canvas && !canvas.querySelector(".overlay-service, .overlay-latency__measurement")),
          timelinePresent: Boolean(timeline),
          missingTexts: expectedTexts.filter(text => !actualTexts.includes(text)),
        };
        if (fixtureState.noServiceDetails && fixtureState.activeChrome && fixtureState.timelinePresent === hasCaptions
          && fixtureState.missingTexts.length === 0 && effectiveUiLanguage() === language) {
          return minimumOverlayHeight(useStore.getState().settings);
        }
      }
      throw new Error(`Overlay fixture did not reach the mounted UI. Restart Vite and reload after HMR/rebase changes before retrying. ${JSON.stringify({ language, route, audioInput, timestamps, ...fixtureState })}`);
    }, { language, route, audioInput, timestamps, provider, alias, speechRecognitionName, profileName, hasCaptions, longHistory, surface, rejectStop });
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
  await configure({ rejectStop: true });
  await resize(360, 160);
  await page.hover(".overlay-surface");
  await page.click('.overlay-action-strip button[aria-label="Stop translating and close subtitles"]');
  await page.mouse.move(4, 4, { label: "Move outside after a rejected close" });
  await page.waitForSelector('.overlay-action-feedback[role="alert"]');
  const failureAfterLeave = await page.evaluate(() => {
    const alert = document.querySelector('.overlay-action-feedback[role="alert"]');
    const timeline = document.querySelector(".overlay-timeline");
    const feedback = alert?.getBoundingClientRect();
    return feedback && feedback.width > 0 && feedback.height > 0 && alert.textContent.trim().length > 0
      && getComputedStyle(alert).visibility === "visible" && getComputedStyle(alert).opacity !== "0"
      && feedback.bottom <= timeline?.getBoundingClientRect().top;
  });
  if (!failureAfterLeave) throw new Error("Quiet mode hid failure feedback or let it cover the caption viewport.");
  await page.goto(`${baseUrl}/?window=overlay`);
  await page.waitForSelector(".overlay-surface");

  const failures = [];
  let chromeChecked = 0;
  const subtitleCases = [];

  for (const width of widths) {
    for (const language of languages) {
      for (const theme of themes) {
        const minimumHeight = await configure({ language, theme });
        await resize(width, minimumHeight);
        await page.hover(".overlay-surface");
        const result = await page.evaluate(() => {
          const canvas = document.querySelector(".overlay-expanded-canvas");
          const strip = canvas?.querySelector(".overlay-action-strip");
          const drag = canvas?.querySelector('[data-testid="overlay-drag-area"]');
          const timeline = canvas?.querySelector(".overlay-timeline");
          const buttons = [...(strip?.querySelectorAll("button") ?? [])];
          const issues = [];
          if (!canvas || !strip || !drag || !timeline || buttons.length !== 6) return { issues: ["overlay chrome missing"] };
          const hitCenter = node => {
            const rect = node.getBoundingClientRect();
            return node.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
          };
          if (buttons.some(button => !hitCenter(button) || button.getBoundingClientRect().width < 24)) issues.push("toolbar button squeezed or blocked");
          if (!hitCenter(drag)) issues.push("drag area blocked");
          if (strip.getBoundingClientRect().bottom > timeline.getBoundingClientRect().top) issues.push("toolbar covers captions");
          if (canvas.querySelector(".overlay-service, .overlay-latency__measurement")) issues.push("details left in caption canvas");
          return { issues, actions: buttons.map(button => button.getAttribute("aria-label")) };
        });
        chromeChecked++;
        if (result.issues.length) failures.push({ check: "chrome", width, minimumHeight, language, theme, ...result });
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

  await page.cdp("Emulation.setDeviceMetricsOverride", { width: 280, height: 520, deviceScaleFactor: 1, mobile: false });
  await page.goto(`${baseUrl}/?window=overlay-control&mode=panel`);
  await page.waitForSelector(".overlay-control-panel");
  const candidates = serviceCases ?? [
    ...["followService", "deepL", "deepLX", "chatMock", "openAICompatible", "original"].map(route => ({ route })),
    { route: "openAICompatible", alias: "B 站 / long translator name with spaces 日本語 🌸 ".repeat(2) },
    { route: "deepL", provider: "customOpenAIASR" },
    { route: "followService", provider: "customDashScopeASR" },
    { route: "openAICompatible", provider: "customDashScopeASR", speechRecognitionName: "Whisper", alias: "Index · 本地" },
    { route: "chatMock", provider: "customOpenAIASR", speechRecognitionName: "会議の音声認識 / 长中文识别服务名称 🌸 ".repeat(2), alias: "Local translator" },
  ];
  let panelChecked = 0;
  for (const language of languages) {
    for (const theme of themes) {
      for (const candidate of candidates) {
        const route = candidate.route ?? "deepL";
        const ordinaryProfile = !candidate.provider && !candidate.alias && !candidate.speechRecognitionName && !candidate.profileName;
        const footerWithoutScrolling = ordinaryProfile && ["zh", "en"].includes(language) && ["followService", "deepL"].includes(route);
        const { showServiceDetails } = await configure({ language, theme, profileName: ordinaryProfile ? "Mimi preview" : undefined, ...candidate, surface: "panel" });
        const result = await page.evaluate(({ route, footerWithoutScrolling, showServiceDetails, compactRows }) => {
          const panel = document.querySelector(".overlay-control-panel");
          const content = panel?.querySelector(".overlay-control-panel__content");
          const details = panel?.querySelector(".overlay-session-details");
          const service = details?.querySelector(".overlay-service__button");
          const icons = [...(service?.querySelectorAll(".provider-icon") ?? [])];
          const names = [...(service?.querySelectorAll(".overlay-service__name") ?? [])];
          const latency = details?.querySelector(".overlay-latency");
          const issues = [];
          if (!panel || !content || !latency || Boolean(service) !== showServiceDetails
            || showServiceDetails && !names.length) return { issues: ["panel details missing or redundant"] };
          panel.scrollTop = 0;
          const serviceBox = service?.getBoundingClientRect();
          const timing = latency.getBoundingClientRect();
          const measurements = [...latency.querySelectorAll(".overlay-latency__measurement strong")].map(node => node.textContent);
          if (measurements.join("\n") !== (route === "original" ? "128 ms" : "128 ms\n246 ms")) issues.push("panel latency samples missing");
          if (icons.length !== names.length || icons.some(icon => Math.abs(icon.getBoundingClientRect().width - 16) > 0.5)) issues.push("provider icon squeezed");
          if (latency.scrollWidth > latency.clientWidth + 1 || timing.right > innerWidth + 1) issues.push("latency clipped");
          if (serviceBox && (timing.top < serviceBox.bottom || serviceBox.right > innerWidth + 1)) issues.push("panel details overlap or overflow");
          for (const name of names) {
            const style = getComputedStyle(name), box = name.getBoundingClientRect();
            if (style.whiteSpace !== "nowrap" || style.textOverflow !== "ellipsis" || box.height > 16.5) issues.push("name wraps");
            if (box.width < 22 || box.left < serviceBox.left || box.right > serviceBox.right + 1) issues.push("name squeezed or outside its button");
          }
          if (panel.scrollWidth > panel.clientWidth + 1 || content.scrollWidth > content.clientWidth + 1) issues.push("panel content overflows horizontally");
          for (const picker of panel.querySelectorAll(".overlay-control-language-pair .overlay-control-picker")) {
            const field = picker.querySelector('[role="combobox"]')?.getBoundingClientRect();
            const column = picker.getBoundingClientRect();
            if (!field || Math.abs(field.left - column.left) > 1 || Math.abs(field.right - column.right) > 1) issues.push("language picker does not fill its column");
          }
          if (compactRows && [...panel.querySelectorAll('.overlay-control-display button')]
            .some(button => button.getBoundingClientRect().height > 42)) issues.push("caption display row stretched beyond its text");
          const system = panel.querySelector('[data-audio-source="system"] button[role="switch"]');
          const microphone = panel.querySelector('[data-audio-source="microphone"] button[role="switch"]');
          if (system?.getAttribute("aria-checked") !== "true" || !system.hasAttribute("data-required")
            || microphone?.getAttribute("aria-checked") !== "false" || microphone.disabled) issues.push("system/microphone choices missing or unavailable");

          const footer = [...panel.querySelectorAll(".overlay-control-footer button")];
          const panelBox = panel.getBoundingClientRect();
          const clip = {
            left: panelBox.left + panel.clientLeft, top: panelBox.top + panel.clientTop,
            right: panelBox.left + panel.clientLeft + panel.clientWidth,
            bottom: Math.min(innerHeight, panelBox.top + panel.clientTop + panel.clientHeight),
          };
          const footerBottom = Math.max(...footer.map(button => button.getBoundingClientRect().bottom));
          if (footer.length !== 1 || !footer[0]?.matches(".overlay-control-settings-link") || footer.some(button => button.disabled)) issues.push("primary footer actions missing or disabled");
          if (footerWithoutScrolling && footerBottom > clip.bottom + 1) issues.push("primary footer actions need scrolling");

          const checkTargets = [".overlay-control-header__collapse", ".overlay-control-picker [role=combobox]", ".overlay-service__button",
            ".overlay-control-capture__toggle", ".overlay-control-display button", ".overlay-control-footer button"];
          for (const button of panel.querySelectorAll(checkTargets.join(","))) {
            // A longer locale may scroll vertically, but each target must be
            // reachable without horizontal clipping or a neighboring layer.
            button.scrollIntoView({ block: "center", inline: "nearest", behavior: "instant" });
            const box = button.getBoundingClientRect();
            const y = box.top + box.height / 2;
            const inset = Math.min(5, box.width / 4);
            const label = button.getAttribute("aria-label") ?? button.textContent.trim();
            if (box.width < 24 || box.height < 24 || box.left < clip.left - 1 || box.right > clip.right + 1
              || box.top < clip.top - 1 || box.bottom > clip.bottom + 1) issues.push(`${label}: incomplete click target`);
            else if (!button.disabled && ![box.left + inset, box.left + box.width / 2, box.right - inset]
              .every(x => button.contains(document.elementFromPoint(x, y)))) issues.push(`${label}: click target covered`);
          }
          panel.scrollTop = 0;
          return { issues, labels: names.map(name => name.textContent), nameWidths: names.map(name => name.clientWidth), measurements,
            panelHeight: panelBox.height, panelContentHeight: panel.scrollHeight, footerBottom };
        }, { route, footerWithoutScrolling, showServiceDetails, compactRows: ["zh", "en"].includes(language) });
        panelChecked++;
        if (result.issues.length) failures.push({ check: "panel", language, theme, ...candidate, ...result });
      }
    }
  }

  await page.cdp("Emulation.clearDeviceMetricsOverride");
  await page.cdp("Emulation.setEmulatedMedia", { features: [] });
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: false });
  const result = { checked: chromeChecked + subtitleCases.length + panelChecked, chromeChecked, subtitleChecked: subtitleCases.length, panelChecked, subtitleCases, failures };
  console.log(JSON.stringify(result));
  if (failures.length) throw new Error(JSON.stringify(failures));
  return result;
}
