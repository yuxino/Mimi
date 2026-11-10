// Import these helpers from the existing browser controller with its managed
// Page. They do not start a server/browser or access native services/user files.
// Warm the Vite root once before opening optimized-dependency fixtures.
const languages = ["zh", "zh-TW", "en", "ja", "de", "ko", "fr", "th", "ru"];
const savedTitle = "Synthetic product review · 字幕と書き出し · ".repeat(3).trim();

async function openWorkspaceFixture(page, baseUrl) {
  await page.goto(new URL("/", baseUrl).href);
  await page.goto(new URL("/scripts/fixtures/settings-layout.html", baseUrl).href);
  await page.cdp("Page.bringToFront", {});
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: true });
  await page.waitForFunction(() => window.layoutReady === true);
  await page.evaluate(() => {
    window.workspaceReviewStage = "ready";
    window.workspaceReviewFrameFallbacks = 0;
    // A native window may own focus while this tab remains visible. Chromium
    // can suspend RAF in that state, so frame observation must never be the
    // only way to complete a local regression step.
    window.workspaceReviewSettle = () => new Promise(resolve => {
      let frame = 0;
      const finish = fallback => {
        clearTimeout(timer);
        cancelAnimationFrame(frame);
        if (fallback) window.workspaceReviewFrameFallbacks += 1;
        resolve();
      };
      const timer = setTimeout(() => finish(true), 80);
      frame = requestAnimationFrame(() => { frame = requestAnimationFrame(() => finish(false)); });
    });
    // Scroll events can wait for rendering even after geometry has changed.
    // Opening a portal menu before that event arrives would correctly dismiss
    // it as outside scrolling. Require actual frames at this boundary; a
    // suspended renderer is incomplete verification, never a passing wait.
    window.workspaceReviewAfterScroll = () => new Promise((resolve, reject) => {
      let frame = 0;
      const timer = setTimeout(() => {
        cancelAnimationFrame(frame);
        reject(new Error("scroll rendering deadline exceeded: two animation frames required"));
      }, 3_000);
      frame = requestAnimationFrame(() => {
        frame = requestAnimationFrame(() => {
          clearTimeout(timer);
          resolve();
        });
      });
    });
  });
}

async function inspectCase(page, next) {
  await page.cdp("Emulation.setDeviceMetricsOverride", {
    width: next.width, height: next.height, deviceScaleFactor: 1, mobile: false,
  });
  await page.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: next.theme }] });
  await page.evaluate(async next => {
    await window.applyLayoutCase(next);
    await window.workspaceReviewSettle();
  }, next);
  // Entry reads are mocked promises, but React still commits their results in a
  // later turn. Wait for the target read rather than an arbitrary long delay.
  await page.waitForFunction(() => {
    const counts = window.workspaceFixture.snapshot().counts;
    return (counts.support_diagnostics ?? 0) + (counts.session_archive_state ?? 0) > 0;
  });
  const preparation = next.saved ? await selectSavedSession(page) : { issues: [] };
  return page.evaluate(async next => {
    await window.workspaceReviewSettle();
    const issues = [...next.preparation.issues];
    const panel = document.getElementById(next.category === "diagnostics" ? "diagnostics-panel" : "session-export-panel");
    const visible = element => element.getClientRects().length > 0 && getComputedStyle(element).visibility !== "hidden";
    const bounds = element => element.getBoundingClientRect();
    if (!panel || !visible(panel)) return { issues: ["requested page not visible"] };
    const scroll = document.querySelector(".settings-console__scroll");
    if (document.documentElement.scrollWidth > innerWidth + 1) issues.push("document horizontal overflow");
    if (scroll && scroll.scrollWidth > scroll.clientWidth + 1) issues.push("settings viewport horizontal overflow");
    if (panel.scrollWidth > panel.clientWidth + 1) issues.push("page horizontal overflow");
    for (const control of panel.querySelectorAll("button, select, input, [role='combobox'], summary")) {
      if (!visible(control)) continue;
      const rect = bounds(control);
      if (rect.left < -1 || rect.right > innerWidth + 1) issues.push(`control outside window: ${control.textContent.trim().slice(0, 60)}`);
      if (control.matches("button, summary") && control.scrollWidth > control.clientWidth + 2) {
        issues.push(`control label clipped: ${control.textContent.trim().slice(0, 60)}`);
      }
      if (control.matches("button, [role='combobox']") && !control.disabled && getComputedStyle(control).cursor !== "pointer") {
        issues.push(`interactive cursor missing: ${control.textContent.trim().slice(0, 60)}`);
      }
    }
    for (const dialog of document.querySelectorAll("[role='dialog'], [role='alertdialog']")) {
      if (!visible(dialog)) continue;
      const rect = bounds(dialog);
      if (rect.left < -1 || rect.right > innerWidth + 1 || rect.top < -1 || rect.bottom > innerHeight + 1) issues.push("dialog outside viewport");
    }
    if (next.saved) {
      const read = window.workspaceFixture.snapshot().actions.filter(action => action.command === "session_history_page").at(-1);
      if (read?.args.id !== next.preparation.id) issues.push("saved record was not read");
      if (panel.querySelector(".session-workspace__catalog")) issues.push("library remains visible in the reader");
      if (panel.querySelectorAll(".session-workspace__reader-actions > button").length !== 2) issues.push("reader must have one export action and one more menu");
      if (!panel.querySelector(".session-workspace__back")) issues.push("reader back action missing");
      const toolbar = panel.querySelector(".session-transcript__toolbar");
      if (!toolbar?.querySelector("[role='combobox']") || !toolbar.querySelector("input")) issues.push("reading mode and search are not in the reader toolbar");
      const title = panel.querySelector(".session-transcript__heading h3");
      if (title?.textContent !== next.savedTitle) issues.push("saved long title missing from reader");
      const paragraphs = [...panel.querySelectorAll(".session-transcript__pair p")];
      if (paragraphs.length < 2 || paragraphs.some(node => !node.textContent.trim())) issues.push("saved bilingual reading content missing");
      for (const node of panel.querySelectorAll(".session-workspace__session-name, .session-transcript__heading h3, .session-transcript__pair p")) {
        const rect = bounds(node);
        if (rect.left < -1 || rect.right > innerWidth + 1 || (node.clientWidth && node.scrollWidth > node.clientWidth + 2)) {
          issues.push(`saved reading overflow: ${node.textContent.trim().slice(0, 60)}`);
        }
      }
    }
    if (!document.querySelector(".settings-console").classList.contains(`settings-console--${next.theme}`)) issues.push("wrong theme");
    return { issues, counts: window.workspaceFixture.snapshot().counts };
  }, { ...next, preparation, savedTitle });
}

async function selectSavedSession(page) {
  return page.evaluate(async title => {
    const issues = [];
    const settle = window.workspaceReviewSettle;
    const waitFor = async (predicate, label) => {
      for (let attempt = 0; attempt < 90; attempt += 1) {
        if (predicate()) return true;
        await new Promise(resolve => setTimeout(resolve, 16));
      }
      issues.push(label);
      return false;
    };
    const item = window.workspaceFixture.snapshot().history.find(entry => entry.count > 0);
    if (!item) return { issues: ["synthetic saved record missing"] };
    const initial = window.workspaceFixture.snapshot();
    if (document.querySelector(".session-transcript")) issues.push("fresh library unexpectedly opened a reader");
    if ((initial.counts.session_history_page ?? 0) || (initial.counts.session_transcript_page ?? 0)) issues.push("fresh library read transcript content before selection");
    const button = () => [...document.querySelectorAll("[data-history-id]")].find(node => node.dataset.historyId === item.id);
    // Seed only the fixture's in-memory catalog, then use the component's normal
    // lifecycle refresh. No file, capture, or provider operation is available here.
    await window.__TAURI_INTERNALS__.invoke("session_history_rename", { id: item.id, title });
    window.workspaceFixture.setActive(true);
    await settle();
    window.workspaceFixture.setActive(false);
    if (!await waitFor(() => button()?.textContent.includes(title), "saved long title did not load")) return { issues, id: item.id };
    button().scrollIntoView({ block: "nearest", behavior: "instant" });
    await window.workspaceReviewAfterScroll();
    button().click();
    await waitFor(() => document.querySelector(".session-transcript__heading h3")?.textContent === title
      && document.querySelectorAll(".session-transcript__pair p").length >= 2
      && window.workspaceFixture.snapshot().actions.filter(action => action.command === "session_history_page").at(-1)?.args.id === item.id,
    "saved reader did not load");
    document.querySelector(".session-transcript__heading")?.scrollIntoView({ block: "start", behavior: "instant" });
    await window.workspaceReviewAfterScroll();
    return { issues, id: item.id };
  }, savedTitle);
}

/**
 * 66 cases: preserve the 44 baseline page renders and add 22 selected saved
 * records with long names and bilingual text. Eight locales at native 520×560,
 * zh/en/ja at 760×720, in both themes. This proves browser geometry, not native
 * device acceptance. Set includeSaved:false for the original 44-case matrix.
 */
export async function verifyDiagnosticsSessionWorkspace(page, baseUrl = "http://127.0.0.1:1420", {
  minimumLanguages = languages, defaultLanguages = ["zh", "en", "ja"], themes = ["light", "dark"], includeSaved = true,
} = {}) {
  await openWorkspaceFixture(page, baseUrl);
  const failures = [];
  let checked = 0;
  for (const size of [{ width: 520, height: 560, languages: minimumLanguages }, { width: 760, height: 720, languages: defaultLanguages }]) {
    for (const language of size.languages) for (const theme of themes) for (const variant of [
      { category: "diagnostics" }, { category: "session-export" }, ...(includeSaved ? [{ category: "session-export", saved: true }] : []),
    ]) {
      const { category } = variant;
      const next = { width: size.width, height: size.height, platform: "macos", language, theme, category,
        diagnostics: category === "diagnostics" ? "listening" : "idle",
        sessions: category === "session-export" ? "mixed" : "empty", active: category === "diagnostics", ...variant };
      const result = await inspectCase(page, next);
      checked += 1;
      if (result.issues.length) failures.push({ ...next, ...result });
    }
  }
  return { checked, failures };
}

/**
 * Sixteen minimum-window cases, each opening rename and export for a selected
 * saved record through the More menu. Checks TXT/Markdown/WAV choices and
 * recording sources without confirming export or starting playback.
 */
export async function verifySessionWorkspaceDialogs(page, baseUrl = "http://127.0.0.1:1420", {
  languages: caseLanguages = languages, themes = ["light", "dark"],
} = {}) {
  await openWorkspaceFixture(page, baseUrl);
  const cases = [];
  const infrastructureErrors = [];
  // Isolate module loading from DOM interactions. A loader/controller timeout
  // is incomplete verification, never a product layout failure.
  try {
    const loaded = await page.evaluate(async () => {
      window.workspaceReviewStage = "copy-load";
      let timer;
      try {
        return await Promise.race([
          import("/src/windows/settings/sessionWorkspaceCopy.ts").then(module => {
            if (!module.sessionWorkspaceCopy) return { error: "sessionWorkspaceCopy export missing" };
            window.workspaceReviewCopy = module.sessionWorkspaceCopy;
            return { ready: true };
          }, error => ({ error: String(error) })),
          new Promise(resolve => { timer = setTimeout(() => resolve({ error: "copy module load deadline exceeded" }), 2_000); }),
        ]);
      } finally { clearTimeout(timer); }
    });
    if (!loaded.ready) infrastructureErrors.push({ stage: "copy-load", error: loaded.error });
  } catch (error) {
    infrastructureErrors.push({ stage: "copy-load", error: String(error) });
  }
  if (infrastructureErrors.length) return { checked: 0, failures: [], cases, infrastructureErrors };
  for (const language of caseLanguages) for (const theme of themes) {
    const next = { width: 520, height: 560, platform: "macos", language, theme, category: "session-export",
      diagnostics: "idle", sessions: "mixed", active: false, saved: true };
    let step = "saved-reader";
    try {
      const baseline = await inspectCase(page, next);
      step = "dialog-setup";
      await page.evaluate(({ language, title }) => {
        const copy = window.workspaceReviewCopy[language];
        const issues = [];
        const stages = [];
        const settle = window.workspaceReviewSettle;
        const visible = element => element?.getClientRects().length > 0 && getComputedStyle(element).visibility !== "hidden";
        const label = element => (element.getAttribute("aria-label") || element.textContent).trim().slice(0, 80);
        const dialog = () => document.querySelector(".settings-confirmation[role='dialog']");
        const exports = () => window.workspaceFixture.snapshot().counts.session_export ?? 0;
        const exportCountBefore = exports();
        const renameCountBefore = window.workspaceFixture.snapshot().counts.session_history_rename ?? 0;
        const audioReads = () => {
          const counts = window.workspaceFixture.snapshot().counts;
          return (counts.session_history_audio ?? 0) + (counts.session_archive_audio ?? 0);
        };
        const audioReadsBefore = audioReads();
        const panel = document.getElementById("session-export-panel");
        const action = text => [...panel.querySelectorAll("button")].find(node => node.textContent.trim() === text);
        const more = () => panel.querySelector(".session-workspace__reader-actions [aria-haspopup='menu']");
        const actionMenu = () => document.getElementById(more()?.getAttribute("aria-controls"));
        const picker = name => [...(dialog()?.querySelectorAll("[role='combobox']") ?? [])].find(node => node.getAttribute("aria-label") === name);
        const click = async (element, stage) => {
          if (!element || !visible(element) || element.disabled) {
            issues.push(`${stage}: action unavailable`);
            return false;
          }
          // Portal menu items already fit the viewport. Scrolling them can
          // scroll an ancestor and correctly dismiss the menu before this
          // test clicks it, leaving a detached node with no React handler.
          const rect = element.getBoundingClientRect();
          if (rect.top < 0 || rect.bottom > innerHeight || rect.left < 0 || rect.right > innerWidth) {
            element.scrollIntoView({ block: "nearest", behavior: "instant" });
            await window.workspaceReviewAfterScroll();
          } else await settle();
          if (!element.isConnected) {
            issues.push(`${stage}: action detached after scrolling`);
            return false;
          }
          element.click();
          await settle();
          return true;
        };
        const checkBox = (element, stage, vertical = true) => {
          if (!visible(element)) { issues.push(`${stage}: not visible`); return; }
          const rect = element.getBoundingClientRect();
          if (rect.width <= 0 || rect.height <= 0 || rect.left < -1 || rect.right > innerWidth + 1
            || (vertical && (rect.top < -1 || rect.bottom > innerHeight + 1))) issues.push(`${stage}: outside viewport`);
          if (!element.matches("input") && element.clientWidth && element.scrollWidth > element.clientWidth + 2) issues.push(`${stage}: horizontal overflow`);
        };
        const inspectDialog = stage => {
          const current = dialog();
          if (!current) { issues.push(`${stage}: dialog missing`); return; }
          checkBox(current, stage);
          const rect = current.getBoundingClientRect();
          stages.push({ stage, bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height } });
          for (const node of current.querySelectorAll(".settings-confirmation__message, label, .settings-row__label, .session-workspace__export-scope, input, button, .mimi-select__label")) {
            checkBox(node, `${stage}/${label(node)}`);
            if (node.matches("button") && !node.disabled && getComputedStyle(node).cursor !== "pointer") issues.push(`${stage}/${label(node)}: pointer cursor missing`);
          }
          const actions = [...current.querySelectorAll(".settings-confirmation__actions button")];
          if (actions.length !== 2) issues.push(`${stage}: expected cancel and confirm actions`);
          if (!current.contains(document.activeElement)) issues.push(`${stage}: focus outside modal`);
        };
        const inspectMenu = (trigger, stage) => {
          const menu = document.getElementById(trigger?.getAttribute("aria-controls"));
          if (!menu || menu.getAttribute("role") !== "listbox" || trigger.getAttribute("aria-expanded") !== "true") {
            issues.push(`${stage}: popup missing`);
            return null;
          }
          checkBox(menu, stage);
          if (!dialog()?.contains(menu)) issues.push(`${stage}: popup outside modal interaction boundary`);
          for (const node of menu.querySelectorAll("[role='option'], .mimi-select__label")) checkBox(node, `${stage}/${label(node)}`);
          const rect = menu.getBoundingClientRect();
          stages.push({ stage, bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height } });
          return menu;
        };
        const inspectActionMenu = stage => {
          const menu = actionMenu();
          if (!menu || menu.getAttribute("role") !== "menu" || more()?.getAttribute("aria-expanded") !== "true") {
            issues.push(`${stage}: action menu missing`);
            return null;
          }
          checkBox(menu, stage);
          const options = [...menu.querySelectorAll("[role='menuitem']")];
          if (options.length !== 2) issues.push(`${stage}: expected rename and delete only`);
          for (const option of options) checkBox(option, `${stage}/${label(option)}`);
          if (!menu.contains(document.activeElement)) issues.push(`${stage}: focus outside action menu`);
          return menu;
        };
        const choose = async (name, value, stage) => {
          const trigger = picker(name);
          if (!await click(trigger, stage)) return;
          const menu = inspectMenu(trigger, `${stage}/popup`);
          const option = [...(menu?.querySelectorAll("[role='option']") ?? [])].find(node => node.textContent.trim() === value);
          if (!await click(option, `${stage}/option`)) return;
          if (picker(name)?.textContent.trim() !== value) issues.push(`${stage}: displayed selection did not change`);
          if (trigger.getAttribute("aria-expanded") !== "false") issues.push(`${stage}: popup stayed open after selection`);
          // Reopen and inspect aria-selected as well as the closed label; a click
          // that was blocked by the modal must not look like a successful change.
          if (!await click(trigger, `${stage}/verify`)) return;
          const selected = inspectMenu(trigger, `${stage}/selected`)?.querySelector("[aria-selected='true']");
          if (selected?.textContent.trim() !== value) issues.push(`${stage}: selected option did not change`);
          trigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
          await settle();
          if (!dialog() || document.querySelector("[role='listbox']")) issues.push(`${stage}: Escape did not close only the popup`);
        };
        let escapeDialog;
        let escapeTrigger;
        let recordingSourceLabel;
        let otherRecordingSource;
        // Each stage is called in its own evaluate. This bounds the controller
        // request even when background timers run at one-second intervals.
        window.workspaceDialogReview = {
          run: async stage => {
            window.workspaceReviewStage = stage;
            if (stage === "more") {
              if (await click(more(), "more/open")) inspectActionMenu("more");
            } else if (stage === "more-escape") {
              const menu = actionMenu();
              menu?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
              await settle();
              if (actionMenu() || more()?.getAttribute("aria-expanded") !== "false") issues.push("more: Escape did not close menu");
              if (document.activeElement !== more()) issues.push("more: Escape did not restore trigger focus");
            } else if (stage === "rename") {
              if (await click(more(), "rename/menu")) {
                const menu = inspectActionMenu("rename/menu");
                const rename = [...(menu?.querySelectorAll("[role='menuitem']") ?? [])].find(node => node.textContent.trim() === copy.rename);
                if (await click(rename, "rename/open")) {
                  inspectDialog("rename");
                  if (actionMenu()) issues.push("rename: action menu remained open");
                  if (dialog()?.querySelector("input")?.value !== title) issues.push("rename: saved title not preserved in input");
                  await click(dialog()?.querySelector(".settings-confirmation__cancel"), "rename/cancel");
                  if (dialog()) issues.push("rename: cancel did not close dialog");
                }
              }
            } else if (stage === "export" && await click(action(copy.export), "export/open")) {
              inspectDialog("export");
              if (dialog()?.querySelector(".session-workspace__export-scope")?.textContent !== copy.whole) issues.push("export: whole-session scope missing");
              if (picker(copy.format)?.textContent.trim() !== copy.plain) issues.push("export: unexpected initial format");
              if (picker(copy.content)?.textContent.trim() !== copy.bilingual) issues.push("export: unexpected initial content");
            } else if (stage === "format") {
              await choose(copy.format, copy.markdown, "export/markdown");
            } else if (stage === "content") {
              await choose(copy.content, copy.translation, "export/translation");
            } else if (stage === "cancel") {
              // Check both final choices together so changing content cannot
              // accidentally reset the previously chosen format.
              if (picker(copy.format)?.textContent.trim() !== copy.markdown || picker(copy.content)?.textContent.trim() !== copy.translation) issues.push("export: final format/content were not retained");
              inspectDialog("export/changed");
              await click(dialog()?.querySelector(".settings-confirmation__cancel"), "export/cancel");
              if (dialog()) issues.push("export: cancel did not close dialog");
              if (exports() !== exportCountBefore) issues.push("export: cancel invoked session_export");
            } else if (stage === "wav-open" && await click(action(copy.export), "wav/open")) {
              await choose(copy.format, copy.wav, "export/wav");
              inspectDialog("export/wav");
              if (picker(copy.content) || dialog()?.querySelector("[role='switch']")) issues.push("wav: text-only options remain visible");
              const sources = [...(dialog()?.querySelectorAll("[role='combobox']") ?? [])].filter(node => node !== picker(copy.format));
              if (sources.length !== 1) issues.push("wav: separate recording source picker missing");
              recordingSourceLabel = sources[0]?.getAttribute("aria-label");
              if (await click(sources[0], "wav/source")) {
                const menu = inspectMenu(sources[0], "wav/source");
                const options = [...(menu?.querySelectorAll("[role='option']") ?? [])];
                if (options.length !== 2) issues.push("wav: expected two separate sources");
                otherRecordingSource = options.find(node => node.getAttribute("aria-selected") !== "true")?.textContent.trim();
                sources[0].dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
                await settle();
              }
            } else if (stage === "wav-source" && recordingSourceLabel && otherRecordingSource) {
              await choose(recordingSourceLabel, otherRecordingSource, "wav/change-source");
              if (picker(copy.format)?.textContent.trim() !== copy.wav) issues.push("wav: changing source reset format");
              if (audioReads() !== audioReadsBefore) issues.push("wav: source selection loaded recording audio");
            } else if (stage === "wav-cancel") {
              await click(dialog()?.querySelector(".settings-confirmation__cancel"), "wav/cancel");
              if (dialog()) issues.push("wav: cancel did not close dialog");
              if (exports() !== exportCountBefore) issues.push("wav: cancel invoked session_export");
            } else if (stage === "escape-open" && await click(action(copy.export), "escape/open")) {
              escapeDialog = dialog();
              escapeTrigger = picker(copy.format);
              if (await click(escapeTrigger, "escape/popup")) inspectMenu(escapeTrigger, "escape/popup");
            } else if (stage === "escape-menu" && escapeTrigger) {
              escapeTrigger.focus();
              escapeTrigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
              await settle();
              if (document.querySelector("[role='listbox']") || escapeTrigger.getAttribute("aria-expanded") !== "false") issues.push("escape: first Escape did not close menu");
              if (dialog() !== escapeDialog) issues.push("escape: first Escape also closed modal");
            } else if (stage === "escape-modal" && escapeTrigger) {
              escapeTrigger.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
              await settle();
              if (dialog()) issues.push("escape: second Escape did not close modal");
            }
            return { stage, issueCount: issues.length };
          },
          result: () => {
            if (exports() !== exportCountBefore) issues.push("cancel/Escape unexpectedly invoked session_export");
            if (audioReads() !== audioReadsBefore) issues.push("dialog review unexpectedly loaded audio");
            if ((window.workspaceFixture.snapshot().counts.session_history_rename ?? 0) !== renameCountBefore) issues.push("rename cancel unexpectedly saved the title");
            return { issues, stages, exportCountBefore, exportCountAfter: exports(),
              frameFallbacks: window.workspaceReviewFrameFallbacks };
          },
        };
      }, { language, title: savedTitle });
      for (step of ["more", "more-escape", "rename", "export", "format", "content", "cancel", "wav-open", "wav-source", "wav-cancel", "escape-open", "escape-menu", "escape-modal"]) {
        await page.evaluate(stage => window.workspaceDialogReview.run(stage), step);
      }
      step = "dialog-result";
      const result = await page.evaluate(() => window.workspaceDialogReview.result());
      cases.push({ ...next, ...result, issues: [...baseline.issues, ...result.issues] });
    } catch (error) {
      infrastructureErrors.push({ ...next, stage: step, error: String(error) });
      // A controller timeout may leave work pending in the page. Stop rather
      // than reset the fixture underneath that work or report false failures.
      return { checked: cases.length, failures: cases.filter(result => result.issues.length), cases, infrastructureErrors };
    }
  }
  return { checked: cases.length, failures: cases.filter(result => result.issues.length), cases, infrastructureErrors };
}

/** Fifteen distinct state renders at the minimum width; interaction/visual
 * review remains in the calling controller, which owns the single browser. */
export async function verifyDiagnosticsSessionStates(page, baseUrl = "http://127.0.0.1:1420", {
  language = "zh", theme = "light",
} = {}) {
  await openWorkspaceFixture(page, baseUrl);
  const cases = await page.evaluate(() => window.workspaceFixture.cases);
  const failures = [];
  let checked = 0;
  for (const category of ["diagnostics", "session-export"]) {
    for (const scenario of category === "diagnostics" ? cases.diagnostics : cases.sessions) {
      const next = { width: 520, height: 560, platform: "macos", language, theme, category,
        ...(category === "diagnostics" ? { diagnostics: scenario } : { sessions: scenario }) };
      const result = await inspectCase(page, next);
      checked += 1;
      if (result.issues.length) failures.push({ ...next, ...result });
    }
  }
  return { checked, failures };
}
