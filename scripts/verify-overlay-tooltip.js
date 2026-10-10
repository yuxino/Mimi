// Actual overlay components inside a stub native boundary; no capture/provider proof.
// Serve the repository with npm run dev and pass an Ego Lite managed Page.
export default async function verifyOverlayTooltip(page, baseUrl = "http://127.0.0.1:5177") {
  await page.goto(new URL("/scripts/fixtures/overlay-tooltip.html", baseUrl).href);
  await page.waitForFunction(() => window.tooltipReady === true);
  const failures = [];
  let checked = 0;
  for (const width of [280, 360]) for (const compact of [true, false]) {
    await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: compact ? 54 : 136, deviceScaleFactor: 1, mobile: false });
    for (const language of ["zh", "zh-TW", "en", "ja", "de", "fr", "ko", "th"]) for (const audioInput of ["system", "microphone", "both"]) {
      await page.evaluate(({ language, compact, audioInput }) => window.renderTooltipFixture(language, compact, audioInput), { language, compact, audioInput });
      await page.waitForFunction(compact => document.querySelector('[data-testid="drag-handle"]')?.getBoundingClientRect().height === (compact ? 30 : 18), compact);
      await page.hover('[data-testid="drag-handle"]');
      await page.waitForSelector('[role="tooltip"]');
      const issues = await page.evaluate(compact => {
        const issues = [];
        const tooltip = document.querySelector('[role="tooltip"]');
        const rect = tooltip.getBoundingClientRect();
        if (rect.left < 6 || rect.right > innerWidth - 6 || rect.top < 6 || rect.bottom > innerHeight - 6) issues.push("tooltip leaves native bounds");
        if (tooltip.scrollHeight > tooltip.clientHeight || tooltip.scrollWidth > tooltip.clientWidth) issues.push("tooltip text clipped");
        if (compact) for (const button of document.querySelectorAll('button')) {
          const other = button.getBoundingClientRect();
          if (rect.left < other.right && rect.right > other.left && rect.top < other.bottom && rect.bottom > other.top) issues.push(`tooltip covers ${button.getAttribute('aria-label')}`);
        }
        return issues;
      }, compact);
      checked++;
      if (issues.length) failures.push({ width, compact, language, audioInput, issues });
      if (compact) {
        await page.dblclick('[data-testid="drag-handle"]');
        await page.waitForFunction(() => !document.querySelector('[data-testid="expand-subtitles"]'));
      }
    }
  }
  await page.cdp("Emulation.clearDeviceMetricsOverride");
  if (failures.length) throw new Error(JSON.stringify({ checked, failures }));
  return { checked, failures };
}
