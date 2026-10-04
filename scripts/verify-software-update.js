// Serve this checkout with npm run dev, then import in an ego-browser round.
// Uses real SettingsView/SoftwareUpdate with simulated updater responses only.
export default async function verifySoftwareUpdate(page, baseUrl = "http://127.0.0.1:1420") {
  await page.goto(new URL("/scripts/fixtures/software-update.html", baseUrl).href);
  await page.waitForFunction(() => window.updateFixtureReady === true);
  const failures = [];
  let checked = 0;
  try {
    for (const width of [520, 760]) {
      await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
      for (const language of ["zh", "en", "ja"]) for (const theme of ["light", "dark"]) {
        const cases = ["normal", "checking", "noUpdate", "available", "checkError", "downloading", "verifying", "downloaded", "downloadError", "installError", "restartReady", "restartError"]
          .flatMap(state => [{ width, language, theme, state, notesOpen: false },
            ...(["normal", "checking", "noUpdate", "checkError"].includes(state) ? [] : [{ width, language, theme, state, notesOpen: true }])]);
        const results = await page.evaluate(async candidates => {
          const results = [];
          const rect = node => node.getBoundingClientRect();
          for (const next of candidates) {
            const actual = await window.applyUpdateCase(next);
            const issues = [];
            const update = document.querySelector(".software-update");
            const panel = document.querySelector("#application-settings-panel");
            const frame = document.querySelector(".settings-console__frame");
            const copy = update.querySelector(".software-update__copy");
            const actions = update.querySelector(".software-update__actions");
            if (actual.state !== (["normal", "noUpdate"].includes(next.state) ? "noUpdate" : next.state === "verifying" ? "downloading" : next.state)) issues.push("wrong session state");
            if (panel.classList.contains("is-inactive")) issues.push("General is not selected");
            if (window.__TAURI_INTERNALS__) issues.push("unexpected native bridge");
            for (const node of [frame, update, copy, actions, ...update.querySelectorAll("button, .software-update__status, .software-update__notes, pre")]) {
              if (!node || rect(node).width === 0 || node.closest("details:not([open])") && !node.closest("summary")) continue;
              if (node.scrollWidth > node.clientWidth + 1 || rect(node).left < -1 || rect(node).right > innerWidth + 1) issues.push(`horizontal clipping: ${node.className || node.tagName}`);
            }
            const cr = rect(copy), ar = rect(actions);
            if (cr.right > ar.left + 1 && cr.bottom > ar.top + 1 && ar.bottom > cr.top + 1) issues.push("copy/action overlap");
            if (!document.querySelector(".settings-console").classList.contains(`settings-console--${next.theme}`)) issues.push("wrong theme");
            if (next.state === "normal" && document.querySelector(".settings-toast")) issues.push("automatic no-update toast");
            if (next.state === "noUpdate" && !document.querySelector(".settings-toast")) issues.push("manual no-update missing toast");
            if (next.state === "checkError" && document.querySelector(".settings-toast")) issues.push("automatic failure toast");
            const button = update.querySelector(".software-update-button");
            if (button && button.disabled !== ["checking", "downloading", "verifying"].includes(next.state)) issues.push("wrong action availability");
            if (next.state === "downloading" && update.querySelector("progress")?.value !== 42) issues.push("wrong progress");
            if (next.state === "verifying" && !window.updateFixture.session.getSnapshot().state.transferComplete) issues.push("missing verification phase");
            if (next.notesOpen) {
              const notes = update.querySelector(".software-update__notes");
              const heading = { en: "English update details", zh: "中文更新内容", ja: "日本語の更新内容" }[next.language];
              if (notes.querySelector("h4")?.textContent !== heading) issues.push("wrong release-note language");
              if (!notes.querySelector("strong") || !notes.querySelector("em") || !notes.querySelector("li") || !notes.querySelector("code")) issues.push("missing parsed markdown");
              if (/## English|## 中文|## 日本語|\*\*|\]\(https:/.test(notes.textContent)) issues.push("raw markdown markers");
              if (notes.querySelector("a, img, iframe, script")) issues.push("external release content became active");
            }
            results.push({ ...next, issues });
          }
          return results;
        }, cases);
        checked += results.length;
        failures.push(...results.filter(result => result.issues.length));
      }
    }
  } finally {
    await page.cdp("Emulation.clearDeviceMetricsOverride");
  }
  if (failures.length) throw new Error(JSON.stringify({ checked, failureCount: failures.length, failures: failures.slice(0, 12) }));
  return { checked, failures, scope: "actual SettingsView and SoftwareUpdate; browser fixture only" };
}
