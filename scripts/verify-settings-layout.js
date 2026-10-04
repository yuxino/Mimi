// Browser geometry regression. Serve the repository with npm run dev, then
// import this function from an ego-browser nodejs round with its managed Page.
// Platform bridges and device state are fixtures, not native-device evidence.
export default async function verifySettingsLayout(page, baseUrl = "http://127.0.0.1:1420") {
  await page.goto(new URL("/scripts/fixtures/settings-layout.html", baseUrl).href);
  await page.waitForFunction(() => window.layoutReady === true);
  await page.waitForFunction(() => getComputedStyle(document.querySelector("#audio-input .settings-row + .settings-row")).marginTop === "16px");
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: true });
  const failures = [];
  let checked = 0;
  for (const width of [520, 680, 760, 952, 1920]) {
    await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
    for (const language of ["en", "zh", "ja"]) for (const theme of ["light", "dark"]) {
      for (const platform of ["windows", "macos", "linux"]) {
        const cases = (platform === "windows" ? ["idle", "receiving", "silent", "noData", "paused", "missing", "empty", "failed"] : ["idle"])
          .map(state => ({ width, language, theme, platform, state, deviceName: "FixtureHeadphones音声出力 / ".repeat(24) }));
        if (platform === "windows") {
          for (const provider of ["alibabaCloud", "customDashScopeASR", "openAIRealtime"]) {
            cases.push({ width, language, theme, platform, state: "idle", editor: true, provider, deviceName: "Fixture headphones" });
            cases.push({ width, language, theme, platform, state: "idle", editor: true, provider, reveal: true, deviceName: "Fixture headphones" });
          }
          cases.push({ width, language, theme, platform, state: "idle", providerPicker: true, deviceName: "Fixture headphones" });
          cases.push({ width, language, theme, platform, state: "idle", providerConfirmation: true, deviceName: "Fixture headphones" });
          for (const category of ["subtitle-settings", "application-settings", "session-export", "diagnostics", "getting-started"]) {
            cases.push({ width, language, theme, platform, category, state: "idle", deviceName: "Fixture headphones" });
          }
        }
        const results = [];
        // Keep each browser evaluation below its timeout when rendering is throttled.
        for (let offset = 0; offset < cases.length; offset += 4) {
          results.push(...await page.evaluate(async candidates => {
          const results = [];
          for (const next of candidates) {
            await window.applyLayoutCase(next);
            const issues = [];
            const rect = node => node.getBoundingClientRect();
            const frame = document.querySelector(".settings-console__frame");
            if (frame.scrollWidth > frame.clientWidth + 1) issues.push("frame overflow");
            if (document.querySelector(".services-hint, .settings-diagnostic-privacy")) issues.push("detached help footer");
            for (const group of document.querySelectorAll(".credential-storage-help, .service-detail__name-help, .provider-picker__name-help, .service-stage__name-help")) {
              const context = group.firstElementChild, help = group.querySelector(".settings-help-control__button");
              if (rect(group).width > 0 && rect(help).left - rect(context).right > 8) issues.push("detached contextual help");
            }
            for (const dialog of document.querySelectorAll('[role="dialog"], [role="alertdialog"]')) {
              if (dialog.scrollWidth > dialog.clientWidth + 1 || rect(dialog).left < 0 || rect(dialog).right > innerWidth + 1) issues.push("dialog overflow");
            }
            for (const reveal of document.querySelectorAll(".stored-credential-reveal")) {
              const button = reveal.querySelector("button"), caption = button.querySelector("span");
              if (rect(caption).width < 2 || rect(caption).height < 2) issues.push("unlabeled saved-value action");
              const field = reveal.closest(".settings-field"), input = field?.querySelector(".config-input-group");
              if (input && rect(button).bottom > rect(input).top + 1) issues.push("saved-value action below field");
            }
            const hasOutline = node => {
              const style = getComputedStyle(node);
              return style.outlineStyle !== "none" && parseFloat(style.outlineWidth) > 0;
            };
            for (const group of document.querySelectorAll(".config-input")) {
              const input = group.querySelector("input");
              if (!input || input.disabled || rect(group).width === 0) continue;
              input.focus({ preventScroll: true });
              if (!hasOutline(group)) issues.push("compound field missing focus outline");
              if (hasOutline(input)) issues.push("compound field inner focus outline");
              for (const button of group.querySelectorAll("button:not(:disabled)")) {
                button.focus({ preventScroll: true });
                if (hasOutline(group)) issues.push("compound action has field focus outline");
                if (button.matches(":focus-visible") && !hasOutline(button)) issues.push("compound action missing focus outline");
              }
              document.activeElement?.blur();
            }
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
            if (!next.category && !next.editor && !next.providerPicker && !next.providerConfirmation && (next.platform === "windows") !== Boolean(output)) issues.push("platform output visibility");
            if (output && ["idle", "paused"].includes(next.state) && output.querySelector('[role="status"]')) issues.push("persistent idle guidance");
            if (!document.querySelector(".settings-console").classList.contains(`settings-console--${next.theme}`)) issues.push("wrong fixture theme");
            results.push(issues);
          }
          return results;
          }, cases.slice(offset, offset + 4)));
        }
        results.forEach((issues, index) => {
          if (issues.length) failures.push({ ...cases[index], deviceName: undefined, issues });
          checked++;
        });
      }
    }
  }
  await page.cdp("Emulation.clearDeviceMetricsOverride");
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: false });
  if (failures.length) throw new Error(JSON.stringify({ checked, failureCount: failures.length, failures: failures.slice(0, 12) }));
  return { checked, failures };
}
