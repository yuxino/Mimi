// Browser geometry regression. Serve the repository with npm run dev, then
// import this function from an ego-browser nodejs round with its managed Page.
// Platform bridges and device state are fixtures, not native-device evidence.
export default async function verifySettingsLayout(page, baseUrl = "http://127.0.0.1:1420", { widths = [520, 680, 760, 952, 1920], appleOnly = false, appleTranslationOnly = false } = {}) {
  await page.goto(new URL("/scripts/fixtures/settings-layout.html", baseUrl).href);
  await page.waitForFunction(() => window.layoutReady === true);
  await page.waitForFunction(() => getComputedStyle(document.querySelector("#audio-input .settings-row + .settings-row")).marginTop === "16px");
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: true });
  const failures = [];
  let checked = 0;
  for (const width of widths) {
    await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
    for (const language of ["en", "zh", "ja"]) for (const theme of ["light", "dark"]) {
      await page.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
      for (const platform of (appleOnly || appleTranslationOnly ? ["macos"] : ["windows", "macos", "linux"])) {
        const cases = (appleOnly || appleTranslationOnly ? [] : platform === "windows" ? ["idle", "receiving", "silent", "noData", "paused", "missing", "empty", "failed"] : ["idle"])
          .map(state => ({ width, language, theme, platform, state, deviceName: "FixtureHeadphones音声出力 / ".repeat(24) }));
        if (platform === "macos" && !appleTranslationOnly) {
          for (const variant of ["ready", "missing", "prepared", "failed", "installed", "paused"]) cases.push({ width, language, theme, platform, state: variant === "paused" ? "paused" : "idle", editor: true, provider: "appleSpeech", appleExpanded: true,
            appleSpeechVariant: variant, appleSpeechSelection: ["missing", "prepared", "failed"].includes(variant) ? "ja" : variant === "installed" ? "fr" : undefined,
            appleSpeechApply: ["prepared", "failed", "installed"].includes(variant), appleSpeechPrepareError: variant === "failed" ? "fixture_prepare_failed" : undefined,
            profileName: "Apple Speech", sourceLanguage: "en", targetLanguage: "original", appleSupport: { available: true, languages: [
              { sourceLanguage: "en", locale: "en-US", installed: true },
              { sourceLanguage: "ja", locale: "ja-JP", installed: false },
              { sourceLanguage: "fr", locale: "fr-FR", installed: true },
            ] } });
        }
        if (platform === "macos" && !appleOnly) {
          for (const variant of ["missing", "ready", "prepared", "cancelled", "failed", "paused", "sameLanguage"]) {
            cases.push({ width, language, theme, platform, state: variant === "paused" ? "paused" : "idle", editor: true,
              provider: "appleSpeech", profileName: "Apple Speech", textTranslation: "apple", sourceLanguage: "en", targetLanguage: variant === "sameLanguage" ? "en" : "zh",
              appleTranslationExpanded: true, appleTranslationVariant: variant,
              appleTranslationStatus: variant === "ready" ? "installed" : "supported",
              appleTranslationPrepare: ["prepared", "cancelled", "failed"].includes(variant),
              appleTranslationPrepareError: variant === "cancelled" ? "apple_translation_cancelled" : variant === "failed" ? "apple_translation_failed" : undefined,
              appleTranslationSupport: { available: true, sourceLanguages: ["en", "zh", "ja"], targetLanguages: ["en", "zh", "ja"] },
              appleSupport: { available: true, languages: [{ sourceLanguage: "en", locale: "en-US", installed: true }, { sourceLanguage: "zh", locale: "zh-CN", installed: false }] } });
          }
        }
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
            for (const button of document.querySelectorAll(".saved-credential-input__toggle")) {
              const group = button.closest(".config-input");
              const input = group.querySelector("input"), paste = group.querySelector(".config-input__paste");
              if (!button.getAttribute("aria-label") || !button.title) issues.push("unlabeled credential eye");
              if (rect(button).left < rect(input).left || rect(button).right > rect(paste).left) issues.push("credential action overlap");
              if (group.querySelectorAll("input").length !== 1) issues.push("duplicate credential input");
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
            if (next.appleExpanded) {
              const apple = document.querySelector(".apple-speech-settings");
              const speech = apple?.querySelector("#apple-speech-resources");
              if (!speech?.querySelector(".apple-speech-tutorial")) issues.push("Apple guide missing");
              if (speech?.querySelectorAll('button[role="combobox"]').length !== 1 || apple?.querySelector(".apple-speech-pack-manager")) issues.push("Apple duplicate language selector");
              const requests = window.layoutFixtureRequests;
              if (requests.speechPrepare !== (["prepared", "failed"].includes(next.appleSpeechVariant) ? 1 : 0)) issues.push("Apple speech unexpected preparation");
              if (requests.speechSave !== (["prepared", "installed"].includes(next.appleSpeechVariant) ? 1 : 0)) issues.push("Apple speech selection not applied once");
              if (next.appleSpeechVariant === "failed" && !speech.querySelector('[role="alert"]')) issues.push("Apple speech failure missing");
              if (["prepared", "installed"].includes(next.appleSpeechVariant) && speech.querySelector(".apple-speech-resource-actions button")) issues.push("Apple speech selected action remains");
              for (const group of apple?.querySelectorAll(".apple-speech-resource-actions, .apple-speech-tutorial") ?? []) {
                if (group.scrollWidth > group.clientWidth + 1 || rect(group).right > rect(apple).right + 1) issues.push("Apple resource overflow");
              }
              for (const button of apple?.querySelectorAll("button[aria-expanded]") ?? []) {
                if (!button.disabled && getComputedStyle(button).cursor !== "pointer") issues.push("Apple disclosure cursor");
              }
              const tutorial = apple?.querySelector(".apple-speech-tutorial");
              if (tutorial && parseFloat(getComputedStyle(tutorial).fontSize) < 14) issues.push("Apple tutorial small print");
              if (tutorial && getComputedStyle(tutorial.querySelector("ol")).listStyleType !== "decimal") issues.push("Apple tutorial missing step numbers");
            }
            if (next.appleTranslationExpanded) {
              const translation = document.querySelector(".apple-translation-settings");
              const stage = translation?.closest(".service-stage--translation");
              const tutorial = stage?.querySelector(".apple-speech-tutorial");
              if (!translation || !tutorial) issues.push("Apple translation guide missing");
              if (stage?.querySelector("header")?.nextElementSibling !== tutorial) issues.push("Apple translation detached guide");
              if (stage?.lastElementChild?.className !== "apple-speech-connection-check") issues.push("Apple translation detached check");
              const languages = stage?.querySelector(".apple-translation-language-controls");
              if (!languages?.querySelector('[role="combobox"]') || languages.nextElementSibling !== translation) issues.push("Apple translation detached target");
              if ([...document.querySelectorAll('#translation-languages, [role="switch"]')].some(node => rect(node).width > 0)) issues.push("Apple translation duplicate language section");
              for (const group of stage?.querySelectorAll(".apple-speech-resource-actions, .apple-speech-tutorial, .settings-feedback") ?? []) {
                if (group.scrollWidth > group.clientWidth + 1 || rect(group).right > rect(translation).right + 1) issues.push("Apple translation overflow");
                if (parseFloat(getComputedStyle(group).fontSize) < 14) issues.push("Apple translation small print");
              }
              if (tutorial && getComputedStyle(tutorial.querySelector("ol")).listStyleType !== "decimal") issues.push("Apple translation missing step numbers");
              if (document.querySelector('input[type="password"], .network-proxy-form')) issues.push("Apple translation credentials/proxy visible");
              const download = translation?.querySelector(".apple-speech-resource-actions button");
              const variant = next.appleTranslationVariant;
              const attempts = ["prepared", "cancelled", "failed"].includes(variant) ? 1 : 0;
              if (window.layoutFixtureRequests.translationPrepare !== attempts) issues.push("Unexpected native preparation count");
              if (variant !== "sameLanguage") {
                const expected = ["ready", "prepared"].includes(variant)
                  ? { en: "Ready", zh: "已就绪", ja: "準備完了" }[next.language]
                  : { en: "Download or permission needed", zh: "需要下载或启用", ja: "ダウンロードまたは有効化が必要" }[next.language];
                if (!translation?.querySelector(".apple-speech-resource-status")?.textContent.includes(expected)) issues.push("Apple translation readiness label");
              }
              if (["missing", "cancelled", "failed", "paused"].includes(variant) !== Boolean(download)) issues.push("Apple translation download visibility");
              if (download && (variant === "paused") !== download.disabled) issues.push("Apple translation stop lock");
              if (variant === "sameLanguage" && window.layoutFixtureRequests.translationStatus !== 0) issues.push("Same-language resource query");
              if (["cancelled", "failed"].includes(variant) && !translation?.querySelector(".settings-feedback")) issues.push("Apple translation preparation feedback missing");
            }
            const output = [...document.querySelectorAll(".settings-row")].find(row => row.querySelector(".settings-row__feedback") || row.querySelector('[role="combobox"]')?.getAttribute("aria-label") === ({ en: "Output device", zh: "输出设备", ja: "出力デバイス" })[next.language]);
            if (!next.category && !next.editor && !next.providerPicker && !next.providerConfirmation && (next.platform === "windows") !== Boolean(output)) issues.push("platform output visibility");
            if (!next.editor && output && ["idle", "paused"].includes(next.state) && output.querySelector('[role="status"]')) issues.push("persistent idle guidance");
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
  await page.cdp("Emulation.setEmulatedMedia", { features: [] });
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled: false });
  if (failures.length) throw new Error(JSON.stringify({ checked, failureCount: failures.length, failures: failures.slice(0, 12) }));
  return { checked, failures };
}

/** Actual settings, tray and floating controls; catalogs and native actions are fixtures. */
export async function verifyLanguageCatalogLayouts(page, baseUrl = "http://127.0.0.1:1420") {
  await page.goto(new URL("/scripts/fixtures/settings-layout.html", baseUrl).href);
  await page.waitForFunction(() => window.layoutReady === true);
  const failures = [];
  let checked = 0;
  for (const surface of ["settings", "overlay", "tray"]) {
    for (const width of surface === "settings" ? [520, 952] : surface === "tray" ? [320, 420] : [360, 420]) {
      await page.cdp("Emulation.setDeviceMetricsOverride", { width, height: 1000, deviceScaleFactor: 1, mobile: false });
      for (const language of ["zh", "en", "ja"]) for (const theme of ["light", "dark"]) {
        await page.cdp("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
        const cases = surface === "settings"
          ? [{ provider: "googleGeminiLive", sourceLanguage: "auto", targetLanguage: "pt-BR", count: 78, targetMenu: true },
            { provider: "tencentCloud", sourceLanguage: "zh_en", targetLanguage: "en", count: 9 },
            { provider: "volcanoEngine", sourceLanguage: "zh_en", targetLanguage: "zh_en", count: 23 }]
          : [{ provider: "googleGeminiLive", sourceLanguage: "auto", targetLanguage: "pt-BR", count: 78, targetMenu: true },
            { provider: "xAIRealtime", sourceLanguage: "auto", targetLanguage: "pt-PT", count: 21 },
            { provider: "tencentCloud", sourceLanguage: "zh_en", targetLanguage: "en", count: 9 },
            { provider: "volcanoEngine", sourceLanguage: "zh_en", targetLanguage: "zh_en", count: 23 }];
        for (const entry of cases) {
          const issues = await page.evaluate(async ({ entry, surface, language, theme }) => {
            await window.applyLayoutCase({ ...entry, surface: surface === "settings" ? undefined : surface,
              editor: surface === "settings", platform: "macos", language, theme, state: "idle",
              profileName: entry.provider, textTranslation: "followService" });
            const label = entry.targetMenu ? window.layoutFixtureLabels.target : surface === "overlay" ? window.layoutFixtureLabels.overlaySource
              : surface === "tray" ? window.layoutFixtureLabels.traySource : window.layoutFixtureLabels.source;
            const trigger = [...document.querySelectorAll('button[role="combobox"]')].find(button => button.getAttribute("aria-label") === label);
            if (!trigger) return ["language picker missing"];
            const sourceLabel = surface === "overlay" ? window.layoutFixtureLabels.overlaySource : surface === "tray" ? window.layoutFixtureLabels.traySource : window.layoutFixtureLabels.source;
            const pickers = [...document.querySelectorAll('button[role="combobox"]')];
            const source = pickers.find(button => button.getAttribute("aria-label") === sourceLabel);
            const target = pickers.find(button => button.getAttribute("aria-label") === window.layoutFixtureLabels.target);
            if (!source || !target) return ["source or target picker missing"];
            if (entry.provider === "googleGeminiLive" && !source.disabled) return ["automatic source should be visibly locked"];
            const sourceRect = source.getBoundingClientRect(), targetRect = target.getBoundingClientRect();
            if (Math.abs(sourceRect.left - targetRect.left) > 1 || Math.abs(sourceRect.right - targetRect.right) > 1) return ["language controls misaligned"];
            const profile = document.querySelector('.service-detail__name [role="combobox"], .service-detail__name input');
            if (surface === "settings" && profile && Math.abs(profile.getBoundingClientRect().left - sourceRect.left) > 1) return ["language controls misaligned with configuration field"];
            const heading = document.querySelector(".profile-language-settings__heading");
            if (heading) {
              const title = heading.querySelector("h3").getBoundingClientRect(), help = heading.querySelector("button").getBoundingClientRect();
              if (help.left - title.right > 12 || Math.abs((title.top + title.height / 2) - (help.top + help.height / 2)) > 1) return ["detached language catalog help"];
            }
            trigger.scrollIntoView({ block: "nearest" });
            // The picker dismisses on page scroll; settle our reveal before opening it.
            await new Promise(resolve => requestAnimationFrame(resolve));
            trigger.click();
            await new Promise(resolve => setTimeout(resolve, 0));
            const issues = [];
            if (document.querySelectorAll('[role="option"]').length !== entry.count) issues.push("catalog options missing");
            if (!document.querySelector("input.mimi-select__search")) issues.push("search unavailable");
            if (document.documentElement.scrollWidth > innerWidth + 1) issues.push("page horizontal overflow");
            for (const element of document.querySelectorAll(".mimi-select__menu")) {
              const rect = element.getBoundingClientRect();
              if (rect.left < -1 || rect.right > innerWidth + 1 || rect.top < -1 || rect.bottom > innerHeight + 1) issues.push("menu outside viewport");
            }
            if ([...document.querySelectorAll('[role="alert"]')].some(node => node.getBoundingClientRect().width > 0)) issues.push("unexpected action error");
            return issues;
          }, { entry, surface, language, theme });
          checked++;
          if (issues.length) failures.push({ surface, width, language, theme, provider: entry.provider, issues });
        }
      }
    }
  }
  await page.cdp("Emulation.clearDeviceMetricsOverride");
  await page.cdp("Emulation.setEmulatedMedia", { features: [] });
  if (failures.length) throw new Error(JSON.stringify({ checked, failures }));
  return { checked, failures };
}
