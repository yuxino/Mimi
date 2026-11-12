#!/usr/bin/env node
// Render documentation images from the real UI, using Node + Vite + an
// installed Chromium browser. No extra npm dependency or browser download.
import { createHash } from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { access, copyFile, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const fixture = path.join(root, "scripts/readme-images");
const assets = path.join(root, "docs/assets");
const locales = JSON.parse(await readFile(path.join(fixture, "locales.json"), "utf8"));
const args = process.argv.slice(2);
const check = args.includes("--check");
const localeIndex = args.indexOf("--locale");
const selected = localeIndex < 0 ? Object.keys(locales) : [args[localeIndex + 1]];
const usage = "node scripts/readme-images.mjs [--locale en] [--check]";
if (args.includes("--help")) { console.log(usage); process.exit(0); }
const knownArgs = new Set(["--check", "--locale", ...selected]);
if (args.some(arg => !knownArgs.has(arg)) || selected.some(locale => !locales[locale])) {
  throw new Error(`Unknown option or locale. Usage: ${usage}`);
}

const begin = "<!-- mimi-preview:start -->";
const end = "<!-- mimi-preview:end -->";
function imageName(locale) { return `readme-preview-${locale}.webp`; }
function imageBlock(locale, file) {
  const relative = path.relative(path.dirname(path.join(root, file)), path.join(assets, imageName(locale))).split(path.sep).join("/");
  return `${begin}\n![${locales[locale].alt}](${relative})\n${end}`;
}
async function updateReadme(locale, file, verify) {
  const filename = path.join(root, file);
  const content = await readFile(filename, "utf8");
  if (content.split(begin).length !== 2 || content.split(end).length !== 2 || content.indexOf(end) < content.indexOf(begin)) {
    throw new Error(`${file}: add exactly one ${begin} / ${end} block.`);
  }
  const updated = content.slice(0, content.indexOf(begin)) + imageBlock(locale, file)
    + content.slice(content.indexOf(end) + end.length);
  if (verify && content !== updated) throw new Error(`${file}: image reference is out of date.`);
  if (/demos\/README\.md|readme-preview\.png/.test(content)) throw new Error(`${file}: obsolete demonstration reference.`);
  if (!verify && content !== updated) await writeFile(filename, updated);
}
const registeredReadmes = new Set();
for (const [locale, entry] of Object.entries(locales)) {
  if (!/^[a-z]{2}(?:-[A-Z]{2})?$/.test(locale) || !entry.uiLanguage || !entry.sourceLanguage || !entry.targetLanguage
      || !entry.alt || /[\[\]\r\n]/.test(entry.alt) || !Array.isArray(entry.source)
      || entry.source.length !== 3 || entry.translation?.length !== entry.source.length
      || [...entry.source, ...entry.translation].some(text => typeof text !== "string" || !text.trim())
      || !Array.isArray(entry.readmes) || !entry.readmes.length) throw new Error(`Invalid locale configuration: ${locale}`);
  for (const file of entry.readmes) {
    if (registeredReadmes.has(file) || path.isAbsolute(file) || file.split("/").includes("..")) throw new Error(`Invalid or duplicate README: ${file}`);
    registeredReadmes.add(file);
  }
}
const readmes = ["README.md", "README_EN.md", ...(await readdir(path.join(root, "docs/readme"))).filter(name => name.endsWith(".md")).map(name => `docs/readme/${name}`)];
for (const file of readmes) if (!registeredReadmes.has(file)) throw new Error(`Add ${file} to scripts/readme-images/locales.json.`);
const i18n = await readFile(path.join(root, "src/lib/i18n.ts"), "utf8");
const uiLanguages = JSON.parse(i18n.match(/export const UI_LANGUAGES = (\[[^\]]+\])/)[1]);
for (const language of uiLanguages) {
  if (!Object.values(locales).some(entry => entry.uiLanguage === language)) throw new Error(`Add README artwork for UI language ${language}.`);
}
for (const [locale, entry] of Object.entries(locales)) {
  if (!uiLanguages.includes(entry.uiLanguage)) throw new Error(`${locale}: unsupported UI language.`);
}
const sha256 = data => createHash("sha256").update(data).digest("hex");
const manifestPath = path.join(assets, "readme-images.json");
let manifest = { images: {} };
try { manifest = JSON.parse(await readFile(manifestPath, "utf8")); }
catch (error) { if (error.code !== "ENOENT") throw error; }
if (check) {
  for (const locale of selected) {
    const image = await readFile(path.join(assets, imageName(locale)));
    if (manifest.images[locale]?.sha256 !== sha256(image)) throw new Error(`Missing or changed image: ${locale}`);
    if (manifest.images[locale]?.localeSha256 !== sha256(JSON.stringify(locales[locale]))) throw new Error(`Regenerate updated locale: ${locale}`);
    for (const file of locales[locale].readmes) await updateReadme(locale, file, true);
  }
  console.log(`Verified ${selected.length} localized images and their README references.`);
  process.exit(0);
}

const browserCandidates = [process.env.MIMI_DOCS_BROWSER,
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome",
  process.env.PROGRAMFILES && path.join(process.env.PROGRAMFILES, "Google/Chrome/Application/chrome.exe"),
].filter(Boolean);
let executable;
for (const candidate of browserCandidates) {
  try { await access(candidate); executable = candidate; break; } catch { /* Try the next installed browser. */ }
}
if (!executable) throw new Error("Install Chromium/Chrome or set MIMI_DOCS_BROWSER to its executable path.");

const temporary = await mkdtemp(path.join(tmpdir(), "mimi-readme-"));
let browser;
try {
  const { build } = await import("vite");
  const site = path.join(temporary, "site");
  await build({ root, configFile: path.join(root, "config/vite.config.ts"),
    mode: "development", base: "./", envDir: false, logLevel: "error",
    build: { outDir: site, emptyOutDir: true, copyPublicDir: false,
      rollupOptions: { input: { stage: path.join(fixture, "stage.html"), overlay: path.join(fixture, "overlay.html") } },
    },
  });
  browser = spawn(executable, ["--headless=new", "--remote-debugging-pipe", `--user-data-dir=${temporary}/chrome`,
    "--allow-file-access-from-files", "--disable-background-networking", "--disable-component-update", "--disable-sync", "--no-first-run",
    "--no-default-browser-check", "--password-store=basic", "--use-mock-keychain", "about:blank"],
    { stdio: ["ignore", "ignore", "pipe", "pipe", "pipe"] });
  let browserErrors = "";
  browser.stderr.on("data", chunk => { browserErrors = (browserErrors + chunk).slice(-1500); });
  let id = 0;
  const pending = new Map();
  const errors = [];
  const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const callId = ++id;
    const timer = setTimeout(() => { pending.delete(callId); reject(new Error(`${method} timed out.`)); }, 60000);
    pending.set(callId, { resolve, reject, timer });
    browser.stdio[3].write(JSON.stringify({ id: callId, method, params, ...(sessionId ? { sessionId } : {}) }) + "\0");
  });
  const receive = message => {
    if (message.id) {
      const call = pending.get(message.id);
      if (!call) return;
      clearTimeout(call.timer); pending.delete(message.id);
      if (message.error) call.reject(new Error(JSON.stringify(message.error))); else call.resolve(message.result);
    } else if (message.method === "Fetch.requestPaused") {
      const { requestId, request } = message.params;
      const local = request.url.startsWith(pathToFileURL(site + path.sep).href) || request.url.startsWith("data:");
      if (!local) errors.push(`Blocked non-local request: ${new URL(request.url).origin}`);
      void send(local ? "Fetch.continueRequest" : "Fetch.failRequest",
        local ? { requestId } : { requestId, errorReason: "BlockedByClient" }, message.sessionId).catch(error => errors.push(error.message));
    } else if (message.method === "Runtime.exceptionThrown") {
      errors.push(message.params.exceptionDetails.exception?.description ?? message.params.exceptionDetails.text);
    }
  };
  let incoming = "";
  browser.stdio[4].setEncoding("utf8");
  browser.stdio[4].on("data", chunk => {
    incoming += chunk;
    let boundary;
    while ((boundary = incoming.indexOf("\0")) >= 0) {
      const message = incoming.slice(0, boundary);
      incoming = incoming.slice(boundary + 1);
      if (message) receive(JSON.parse(message));
    }
  });
  const disconnected = () => {
    for (const call of pending.values()) { clearTimeout(call.timer); call.reject(new Error(`Chromium disconnected: ${browserErrors}`)); }
    pending.clear();
  };
  browser.once("error", disconnected);
  browser.once("exit", disconnected);
  browser.stdio[3].on("error", disconnected);
  const { targetId } = await send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  const page = (method, params) => send(method, params, sessionId);
  await page("Runtime.enable");
  await page("Fetch.enable", { patterns: [{ urlPattern: "*" }] });
  await page("Emulation.setDeviceMetricsOverride", { width: 1920, height: 1080, deviceScaleFactor: 1, mobile: false });
  await page("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
  const version = await send("Browser.getVersion");
  const revision = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
  const dirtyUi = Boolean(execFileSync("git", ["status", "--porcelain", "--", "src"], { cwd: root, encoding: "utf8" }).trim());
  for (const locale of selected) {
    errors.length = 0;
    await page("Page.navigate", { url: `${pathToFileURL(path.join(site, "scripts/readme-images/stage.html")).href}?locale=${encodeURIComponent(locale)}` });
    const result = await page("Runtime.evaluate", {
      expression: `(async () => { const until = Date.now() + 45000; while (!window.readmeReady) { if (Date.now() > until) throw Error('Fixture did not load'); await new Promise(r => setTimeout(r, 50)); } return await window.readmeReady; })()`,
      awaitPromise: true, returnByValue: true,
    });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    if (result.result.value?.locale !== locale) throw new Error(`Unexpected rendered locale: ${locale}`);
    if (errors.length) throw new Error(errors.join("\n"));
    const { data } = await page("Page.captureScreenshot", { format: "webp", quality: 94, captureBeyondViewport: false });
    const image = Buffer.from(data, "base64");
    await writeFile(path.join(temporary, imageName(locale)), image);
    manifest.images[locale] = { ...result.result.value, file: imageName(locale), width: 1920, height: 1080,
      browser: version.product, platform: process.platform, revision, dirtyUi,
      localeSha256: sha256(JSON.stringify(locales[locale])), sha256: sha256(image) };
    console.log(`Rendered ${locale}: ${(image.length / 1024).toFixed(0)} KiB`);
  }
  // Publish only after every requested locale rendered successfully.
  for (const locale of selected) {
    // The OS temp directory may be on another volume (for example Linux tmpfs).
    await copyFile(path.join(temporary, imageName(locale)), path.join(assets, imageName(locale)));
    for (const file of locales[locale].readmes) await updateReadme(locale, file, false);
  }
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
} finally {
  if (browser?.pid && browser.exitCode === null && browser.signalCode === null) {
    await new Promise(resolve => {
      const timer = setTimeout(() => browser.kill("SIGKILL"), 3000);
      browser.once("exit", () => { clearTimeout(timer); resolve(); });
      browser.kill();
    });
  }
  await rm(temporary, { recursive: true, force: true });
}
