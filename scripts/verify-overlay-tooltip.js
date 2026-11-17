// Actual overlay components inside a stub native boundary; no capture/provider proof.
// Serve the repository with npm run dev and pass an Ego Lite managed Page.
export default async function verifyOverlayTooltip(page, baseUrl = "http://127.0.0.1:5177") {
  await page.goto(new URL("/scripts/fixtures/overlay-tooltip.html", baseUrl).href);
  await page.waitForFunction(() => window.tooltipReady === true);
  const failures = [];
  let checked = 0;
  for (const [width, compact] of [[280, true], [360, true], [360, false]]) {
    await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: compact ? 54 : 136, deviceScaleFactor: 1, mobile: false });
    for (const language of ["zh", "zh-TW", "en", "ja", "de", "fr", "ko", "th", "ru"]) for (const audioInput of ["system", "microphone", "both"]) {
      await page.evaluate(({ language, compact, audioInput }) => window.renderTooltipFixture(language, compact, audioInput), { language, compact, audioInput });
      const drag = '[data-testid="overlay-drag-area"]';
      const action = compact ? '[data-testid="expand-subtitles"]' : '.overlay-action-strip > span:last-child button';
      // Re-rendering at the same size can leave the pointer on the new
      // button. Enter from the drag area as a person moving the mouse would.
      await page.hover(drag);
      await page.waitForSelector(action);
      await page.hover(action);
      try {
        await page.waitForSelector('[role="tooltip"]');
      } catch (error) {
        throw new Error(`No tooltip: ${width}px, ${compact ? "compact" : "expanded"}, ${language}, ${audioInput}`, { cause: error });
      }
      const issues = await page.evaluate(compact => {
        const issues = [];
        const tooltip = document.querySelector('[role="tooltip"]');
        const rect = tooltip.getBoundingClientRect();
        if (rect.left < 6 || rect.right > innerWidth - 6 || rect.top < 6 || rect.bottom > innerHeight - 6) issues.push("tooltip leaves native bounds");
        if (tooltip.scrollHeight > tooltip.clientHeight || tooltip.scrollWidth > tooltip.clientWidth) issues.push("tooltip text clipped");
        if (compact) for (const button of document.querySelectorAll('.overlay-control-button')) {
          const other = button.getBoundingClientRect();
          if (rect.left < other.right && rect.right > other.left && rect.top < other.bottom && rect.bottom > other.top) issues.push(`tooltip covers ${button.getAttribute('aria-label')}`);
        }
        return issues;
      }, compact);
      checked++;
      if (issues.length) failures.push({ width, compact, language, audioInput, issues });
      if (compact) {
        await page.dblclick(drag);
        if (!await page.evaluate(() => Boolean(document.querySelector('[data-testid="expand-subtitles"]')))) {
          failures.push({ width, compact, language, audioInput, issues: ["double-click unexpectedly expands"] });
        }
        await page.click(action);
        await page.waitForFunction(() => !document.querySelector('[data-testid="expand-subtitles"]'));
      }
    }
  }
  await page.cdp("Emulation.clearDeviceMetricsOverride");
  if (failures.length) throw new Error(JSON.stringify({ checked, failures }));
  return { checked, failures };
}
