// Local synthetic browser evidence; no provider, credential or audio access.
export default async function verifyConfigInputs(page) {
  await page.goto("http://127.0.0.1:1420/scripts/fixtures/settings-layout.html");
  await page.waitForFunction(() => window.layoutReady === true);
  const failures = [];
  let checked = 0;
  for (const width of [520, 952]) {
    await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
    for (const language of ["zh", "en", "ja"]) for (const theme of ["light", "dark"]) {
      for (const provider of ["alibabaCloud", "customDashScopeASR", "azureOpenAIRealtime"]) {
        const state = { width, language, theme, provider, platform: "macos", state: "idle", editor: true, reveal: true };
        const issues = await page.evaluate(async next => {
          await window.applyLayoutCase(next);
          const issues = [];
          const frame = document.querySelector(".settings-console__frame");
          if (!frame) throw new Error(`fixture did not render ${next.provider}`);
          const flush = () => new Promise(resolve => requestAnimationFrame(resolve));
          for (const group of document.querySelectorAll(".config-input--expandable")) {
            const input = group.querySelector("input");
            const value = "https://service.example/" + "configuration-path-".repeat(9) + "tail";
            const intended = value.slice(0, input.maxLength > 0 ? input.maxLength : undefined);
            Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, intended);
            input.dispatchEvent(new Event("input", { bubbles: true }));
            await flush();
            group.querySelector(".config-input__expand").click();
            await flush();
            const textarea = group.querySelector("textarea");
            if (!textarea || textarea.value !== intended) { issues.push("expanded value mismatch"); continue; }
            if (group.querySelectorAll("input, textarea").length !== 1) issues.push("duplicate textbox");
            if (textarea.scrollWidth > textarea.clientWidth + 1) issues.push("unwrapped value");
            if (textarea.getBoundingClientRect().height < 70) issues.push("collapsed expanded editor");
            if (textarea !== document.activeElement) issues.push("lost expansion focus");
            const toggle = group.querySelector(".config-input__expand"), paste = group.querySelector(".config-input__paste");
            if (toggle.getBoundingClientRect().right > paste.getBoundingClientRect().left) issues.push("overlapping actions");
            if (!toggle.title || !toggle.getAttribute("aria-label")) issues.push("unlabeled expansion");
            if (group.getBoundingClientRect().right > frame.getBoundingClientRect().right + 1) issues.push("field overflow");
            toggle.click();
            await flush();
            if (group.querySelector("input")?.value !== intended) issues.push("collapse discarded value");
            checkedField(group);
          }
          function checkedField(group) {
            if (group.querySelector(".saved-credential-input__toggle")) issues.push("secret expanded");
          }
          for (const group of document.querySelectorAll(".config-input--with-action")) {
            if (group.querySelector(".config-input__expand, textarea")) issues.push("secret expansion affordance");
          }
          if (frame.scrollWidth > frame.clientWidth + 1) issues.push("frame overflow");
          return issues;
        }, state);
        if (issues.length) failures.push({ ...state, issues });
        checked++;
      }
    }
  }
  await page.cdp("Emulation.clearDeviceMetricsOverride");
  if (failures.length) throw new Error(JSON.stringify({ checked, failures }));
  return { checked, failures };
}
