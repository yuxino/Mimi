// Browser geometry regression. Serve the repository with npm run dev, then
// import this function from an ego-browser nodejs round with its managed Page.
// Platform bridges and device state are fixtures, not native-device evidence.
export default async function verifySettingsLayout(page) {
  await page.goto("http://127.0.0.1:1420/scripts/fixtures/settings-layout.html");
  await page.waitForFunction(() => window.layoutReady === true);
  await page.waitForFunction(() => getComputedStyle(document.querySelector("#audio-input .settings-row + .settings-row")).marginTop === "16px");
  const failures = [];
  let checked = 0;
  for (const width of [520, 680, 760, 952, 1920]) {
    await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
    for (const language of ["en", "zh", "ja"]) for (const theme of ["light", "dark"]) {
      for (const platform of ["windows", "macos", "linux"]) {
        const cases = (platform === "windows" ? ["idle", "receiving", "silent", "noData", "paused", "missing", "empty", "failed"] : ["idle"])
          .map(state => ({ width, language, theme, platform, state, deviceName: "FixtureHeadphones音声出力 / ".repeat(24) }));
        if (platform === "windows") {
          cases.push({ width, language, theme, platform, state: "idle", editor: true, deviceName: "Fixture headphones" });
          for (const category of ["subtitle-settings", "application-settings", "session-export", "diagnostics", "getting-started"]) {
            cases.push({ width, language, theme, platform, category, state: "idle", deviceName: "Fixture headphones" });
          }
        }
        const results = await page.evaluate(async candidates => {
          const results = [];
          for (const next of candidates) {
            await window.applyLayoutCase(next);
            const issues = [];
            const rect = node => node.getBoundingClientRect();
            const frame = document.querySelector(".settings-console__frame");
            if (frame.scrollWidth > frame.clientWidth + 1) issues.push("frame overflow");
            const audioRows = [...document.querySelectorAll("#audio-input .settings-card__body > .settings-row")].filter(row => rect(row).width > 0);
            for (let i = 1; i < audioRows.length; i++) {
              if (rect(audioRows[i]).top - rect(audioRows[i - 1]).bottom < 12) issues.push("crowded audio rows");
            }
            for (const row of document.querySelectorAll(".settings-row")) {
              const label = row.querySelector(".settings-row__label");
              const control = row.querySelector(".settings-row__control");
              const lr = rect(label), cr = rect(control), rr = rect(row);
              if (rr.width === 0 || rr.height === 0) continue;
              if (row.scrollWidth > row.clientWidth + 1 || cr.right > rr.right + 1) issues.push("row overflow");
              if (lr.right > cr.left + 1 && lr.bottom > cr.top + 1 && cr.bottom > lr.top + 1 && cr.width > 0) issues.push("label/control overlap");
              if (row.querySelector(".settings-row__copy").offsetWidth < 100 && row.querySelector('[role="combobox"]')) issues.push("squeezed picker label");
              const feedback = row.querySelector(".settings-row__feedback");
              if (feedback && rect(feedback).top < cr.bottom - 1) issues.push("feedback beside control");
              if (row.querySelector('.settings-row__control [role="status"]')) issues.push("status inside control");
            }
            const output = [...document.querySelectorAll(".settings-row")].find(row => row.querySelector(".settings-row__feedback") || row.querySelector('[role="combobox"]')?.getAttribute("aria-label") === ({ en: "Output device", zh: "输出设备", ja: "出力デバイス" })[next.language]);
            if (!next.category && !next.editor && (next.platform === "windows") !== Boolean(output)) issues.push("platform output visibility");
            if (output && ["idle", "paused"].includes(next.state) && output.querySelector('[role="status"]')) issues.push("persistent idle guidance");
            if (!document.querySelector(".settings-console").classList.contains(`settings-console--${next.theme}`)) issues.push("wrong fixture theme");
            results.push(issues);
          }
          return results;
        }, cases);
        results.forEach((issues, index) => {
          if (issues.length) failures.push({ ...cases[index], deviceName: undefined, issues });
          checked++;
        });
      }
    }
  }
  await page.cdp("Emulation.clearDeviceMetricsOverride");
  if (failures.length) throw new Error(JSON.stringify({ checked, failureCount: failures.length, failures: failures.slice(0, 12) }));
  return { checked, failures };
}
