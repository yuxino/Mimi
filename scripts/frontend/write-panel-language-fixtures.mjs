// Expected native accessibility labels come from the same locale modules as
// the product. No HTTP listener, application startup or provider is involved.
import { writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { createServer } from "vite";

if (process.argv.length !== 3) {
  throw new Error("Usage: node scripts/frontend/write-panel-language-fixtures.mjs <output.json>");
}

// Locale selection must not consult Node's optional on-disk web storage.
Object.defineProperty(globalThis, "localStorage", { configurable: true, value: undefined });

const server = await createServer({
  root: fileURLToPath(new URL("../../", import.meta.url)),
  configFile: false,
  envDir: false,
  publicDir: false,
  appType: "custom",
  logLevel: "error",
  server: { middlewareMode: true, hmr: false, ws: false, watch: null, preTransformRequests: false },
  optimizeDeps: { noDiscovery: true, include: [] },
});

try {
  const { I18N, UI_LANGUAGES, setStoredUiLanguage } = await server.ssrLoadModule("/src/lib/i18n.ts");
  const { sourceLanguageDisplayName, targetLanguageDisplayName } = await server.ssrLoadModule("/src/lib/types.ts");
  const { SUBTITLE_DISPLAY_OPTIONS } = await server.ssrLoadModule("/src/lib/subtitles/subtitleDisplay.ts");
  const fixtures = UI_LANGUAGES.map(locale => {
    setStoredUiLanguage(locale);
    // Only these locales use product-owned language names. For the other six,
    // Node and WebView2 can ship different Intl/CLDR names; null delegates the
    // value check to the closed trigger and selected option in the same renderer.
    const hasFixedLanguageNames = locale === "zh" || locale === "en" || locale === "ja";
    return {
      locale,
      openControls: I18N.overlay.openControls,
      source: I18N.overlay.sourceLanguage,
      target: I18N.settings.translateTo,
      moreSettings: I18N.overlay.moreSettings,
      sourceValue: hasFixedLanguageNames ? sourceLanguageDisplayName("en", locale) : null,
      targetValue: hasFixedLanguageNames ? targetLanguageDisplayName("zh_tw", locale) : null,
      modes: SUBTITLE_DISPLAY_OPTIONS.map(option => option.shortLabel),
      systemAudio: I18N.settings.audioInputSystem,
      microphone: I18N.settings.audioInputMicrophone,
    };
  });
  if (fixtures.some(({ modes, sourceValue, targetValue, ...labels }) => modes.length !== 3 ||
      [...Object.values(labels), ...modes, ...[sourceValue, targetValue].filter(value => value !== null)]
        .some(label => typeof label !== "string" || !label.trim()))) {
    throw new Error("A panel language fixture has missing labels.");
  }
  await writeFile(resolve(process.argv[2]), `${JSON.stringify(fixtures, null, 2)}\n`, "utf8");
} finally {
  await server.close();
}
