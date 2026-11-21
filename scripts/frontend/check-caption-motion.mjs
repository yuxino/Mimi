#!/usr/bin/env node
// Execute the existing browser geometry assertions with an installed Chromium.
// The browser profile and Vite listener belong only to this process.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { access, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

class DevTools {
  calls = new Map();
  events = new Set();
  nextId = 0;

  constructor(browser, diagnostic) {
    this.input = browser.stdio[3];
    this.output = browser.stdio[4];
    this.diagnostic = diagnostic;
    let incoming = "";
    this.output.setEncoding("utf8");
    this.output.on("data", chunk => {
      incoming += chunk;
      let boundary;
      while ((boundary = incoming.indexOf("\0")) >= 0) {
        const frame = incoming.slice(0, boundary);
        incoming = incoming.slice(boundary + 1);
        if (!frame) continue;
        let message;
        try { message = JSON.parse(frame); }
        catch { this.disconnected(new Error("Invalid Chromium pipe message")); return; }
        this.receive(message);
      }
    });
    browser.once("exit", () => this.disconnected(new Error(`Chromium exited: ${diagnostic()}`)));
    browser.once("error", error => this.disconnected(error));
    this.input.on("error", error => this.disconnected(error));
    this.output.on("error", error => this.disconnected(error));
    this.output.once("end", () => this.disconnected(new Error(`Chromium pipe ended: ${diagnostic()}`)));
  }

  receive(message) {
    const call = this.calls.get(message.id);
    if (call) {
      clearTimeout(call.timer);
      this.calls.delete(message.id);
      if (message.error) call.reject(new Error(`${call.method}: ${message.error.message}`));
      else call.resolve(message.result);
    }
    for (const listener of this.events) {
      if (listener.method === message.method && listener.sessionId === message.sessionId) listener.resolve(message.params);
    }
    this.onEvent?.(message);
  }

  disconnected(error) {
    this.closedError ??= error;
    for (const call of this.calls.values()) { clearTimeout(call.timer); call.reject(this.closedError); }
    this.calls.clear();
    for (const listener of this.events) listener.reject(this.closedError);
  }

  close() {
    this.disconnected(new Error("Chromium transport closed"));
    this.input.end();
    this.output.destroy();
  }

  command(method, params = {}, sessionId, timeout = 30000) {
    if (this.closedError) return Promise.reject(this.closedError);
    return new Promise((resolve, reject) => {
      const id = ++this.nextId;
      const timer = setTimeout(() => { this.calls.delete(id); reject(new Error(`${method} timed out after ${timeout}ms: ${this.diagnostic()}`)); }, timeout);
      this.calls.set(id, { method, resolve, reject, timer });
      this.input.write(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }) + "\0",
        error => { if (error) this.disconnected(error); });
    });
  }

  event(method, sessionId) {
    if (this.closedError) return Promise.reject(this.closedError);
    return new Promise((resolve, reject) => {
      const finish = (callback, value) => { clearTimeout(timer); this.events.delete(listener); callback(value); };
      const listener = { method, sessionId, resolve: value => finish(resolve, value), reject: error => finish(reject, error) };
      const timer = setTimeout(() => listener.reject(new Error(`${method} timed out`)), 30000);
      this.events.add(listener);
    });
  }
}

// Only the page methods used by the caption checks; all DOM input is real CDP
// input. Fixture state and geometric assertions remain in the existing script.
function captionPage(devtools, sessionId) {
  const cdp = (method, params) => devtools.command(method, params, sessionId);
  const evaluate = async (fn, arg) => {
    const result = await cdp("Runtime.evaluate", {
      expression: `(${fn.toString()})(${arg === undefined ? "undefined" : JSON.stringify(arg)})`,
      awaitPromise: true, returnByValue: true,
    });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    return result.result.value;
  };
  let buttons = 0;
  let position = { x: 0, y: 0 };
  const move = (x, y) => { position = { x, y }; return cdp("Input.dispatchMouseEvent", { type: "mouseMoved", ...position, buttons }); };
  const page = {
    cdp, evaluate,
    async goto(url) {
      const loaded = devtools.event("Page.loadEventFired", sessionId);
      await Promise.all([cdp("Page.navigate", { url }), loaded]);
    },
    waitForSelector: selector => evaluate(async selector => {
      const deadline = performance.now() + 15000;
      while (!document.querySelector(selector)) {
        if (performance.now() >= deadline) throw new Error(`Selector did not appear: ${selector}`);
        await new Promise(requestAnimationFrame);
      }
    }, selector),
    mouse: {
      move,
      async down() { buttons = 1; await cdp("Input.dispatchMouseEvent", { type: "mousePressed", ...position, button: "left", buttons, clickCount: 1 }); },
      async up() { buttons = 0; await cdp("Input.dispatchMouseEvent", { type: "mouseReleased", ...position, button: "left", buttons, clickCount: 1 }); },
    },
    async hover(selector) {
      const point = await evaluate(selector => {
        const element = document.querySelector(selector);
        if (!element) throw new Error(`Hover target missing: ${selector}`);
        const box = element.getBoundingClientRect();
        return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
      }, selector);
      await move(point.x, point.y);
    },
  };
  return page;
}

let vite, browser, devtools, temporary;
try {
  let executable;
  for (const candidate of [process.env.CHROME_BIN, "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"].filter(Boolean)) {
    try { await access(candidate); executable = candidate; break; } catch { /* Try the next installed browser. */ }
  }
  if (!executable) throw new Error("Caption geometry requires the runner's installed Chromium; set CHROME_BIN if necessary.");
  temporary = await mkdtemp(path.join(tmpdir(), "mimi-caption-motion-"));
  const { createServer } = await import("vite");
  vite = await createServer({ root, configFile: path.join(root, "config/vite.config.ts"), envDir: false,
    logLevel: "error", server: { host: "127.0.0.1", port: 0, strictPort: false } });
  await vite.listen();
  const address = vite.httpServer.address();
  const baseUrl = `http://127.0.0.1:${address.port}`;
  const started = Date.now();
  // Chromium's standard inherited pipe avoids endpoint-file discovery and a
  // debugging TCP listener. The documentation renderer uses the same fd3/fd4.
  browser = spawn(executable, ["--remote-debugging-pipe",
    `--user-data-dir=${temporary}`, "--disable-background-networking", "--disable-component-update", "--disable-sync",
    "--no-first-run", "--no-default-browser-check", "--password-store=basic", "--use-mock-keychain", "about:blank"],
  { stdio: ["ignore", "ignore", "pipe", "pipe", "pipe"] });
  let firstErrors = "", lastErrors = "";
  browser.stderr.setEncoding("utf8");
  browser.stderr.on("data", data => {
    firstErrors = (firstErrors + data).slice(0, 4000);
    lastErrors = (lastErrors + data).slice(-4000);
  });
  const diagnostic = () => JSON.stringify({ executable, elapsedMs: Date.now() - started,
    exitCode: browser.exitCode, signal: browser.signalCode, stderrFirst: firstErrors, stderrLast: lastErrors });
  devtools = new DevTools(browser, diagnostic);
  const version = await devtools.command("Browser.getVersion", {}, undefined, Math.max(1, 15000 - (Date.now() - started)));
  console.log(JSON.stringify({ stage: "chromium-ready", transport: "pipe", browser: version.product, elapsedMs: Date.now() - started }));
  const { targetId } = await devtools.command("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await devtools.command("Target.attachToTarget", { targetId, flatten: true });
  const page = captionPage(devtools, sessionId);
  const errors = [];
  devtools.onEvent = message => {
    if (message.sessionId !== sessionId) return;
    if (message.method === "Runtime.exceptionThrown") errors.push(message.params.exceptionDetails.text);
    if (message.method === "Fetch.requestPaused") {
      const { requestId, request } = message.params;
      const local = request.url.startsWith(baseUrl + "/") || request.url.startsWith("data:");
      if (!local) errors.push("Non-fixture network request blocked");
      void page.cdp(local ? "Fetch.continueRequest" : "Fetch.failRequest", local ? { requestId } : { requestId, errorReason: "BlockedByClient" })
        .catch(error => errors.push(error.message));
    }
  };
  await page.cdp("Page.enable");
  await page.cdp("Runtime.enable");
  await page.cdp("Fetch.enable", { patterns: [{ urlPattern: "*" }] });
  await page.cdp("Emulation.setDeviceMetricsOverride", { width: 1000, height: 700, deviceScaleFactor: 1, mobile: false });
  const pointer = await page.evaluate(() => ({
    hover: matchMedia("(hover: hover)").matches,
    fine: matchMedia("(pointer: fine)").matches,
  }));
  console.log(JSON.stringify({ stage: "input-capabilities", ...pointer }));
  assert.ok(pointer.hover && pointer.fine, "Run the desktop hover checks with a mouse-capable display (xvfb-run on Linux)");
  const script = await readFile(path.join(root, "scripts/frontend/verify-overlay-service-layout.js"), "utf8");
  const verify = new Function(`return (${script}\n);`)();
  const result = await verify(page, { baseUrl, subtitleMotionOnly: true });
  assert.equal(result.checked, 4, "Expected quiet/hover × motion off/on");
  for (const hovered of [false, true]) for (const motion of [false, true]) {
    const cases = result.subtitleMotion.filter(item => item.hovered === hovered && item.motion === motion);
    assert.equal(cases.length, 1, "Missing or duplicate caption motion case");
    assert.ok(cases[0].frames > 0, "Caption geometry was not sampled");
    assert.equal(cases[0].latencyVisibility.length, 11, "Missing actual latency/status states");
    assert.ok(cases[0].latencyVisibility.every(state => state.ready && state.frames > 0 && state.maxShift <= 2),
      "Latency/status visibility or caption geometry was not verified");
  }
  const readouts = await verify(page, { baseUrl, readoutLayoutOnly: true, widths: [360, 580], themes: ["dark"] });
  assert.equal(readouts.checked, 18, "Expected nine locales at narrow and ordinary overlay widths");
  for (const width of [360, 580]) for (const language of ["zh", "zh-TW", "en", "ja", "de", "fr", "ko", "th", "ru"]) {
    const cases = readouts.readoutLayout.filter(item => item.width === width && item.language === language);
    assert.equal(cases.length, 1, "Missing or duplicate localized readout layout");
    assert.deepEqual(cases[0].exposures.map(item => item.hovered), [false, true, false], "Missing physical hover/leave sequence");
    for (const exposure of cases[0].exposures) {
      assert.deepEqual(exposure.states.map(state => state.pending), [true, false], "Pending/clear was not exercised");
      assert.ok(exposure.states.every(state => state.ready && state.frames > 0 && state.maxShift <= 2), "Localized readout moved captions or was not visible");
      if (exposure.hovered) assert.ok(exposure.states.every(state => state.placement?.placement === (width === 580 ? "beside" : "below")),
        "Readout did not use the expected toolbar placement");
    }
  }
  const modes = await verify(page, { baseUrl, subtitleModesOnly: true });
  assert.equal(modes.checked, 24, "Expected four representative routes, each switching all three modes twice");
  for (const route of ["caption-doubao", "caption-alibaba", "caption-gemini", "caption-independent"]) {
    const cases = modes.subtitleModes.filter(item => item.route === route);
    assert.deepEqual(cases.map(item => item.mode), ["original", "translation", "bilingual", "original", "translation", "bilingual"]);
    assert.ok(cases.every(item => item.frames > 0 && item.pendingShift <= 2 && item.historyShift <= 2
      && item.targetLanguage === "zh" && !/openai/i.test(item.provider)), "Unverified route or caption geometry");
  }
  assert.deepEqual(errors, [], "Unexpected browser errors");
  console.log(JSON.stringify({ browser: (await devtools.command("Browser.getVersion")).product, ...result,
    readoutChecked: readouts.checked, readoutLayout: readouts.readoutLayout,
    modeChecked: modes.checked, subtitleModes: modes.subtitleModes }));
} catch (error) {
  // Preserve the assertion or launch error even if cleanup also fails.
  console.error(error);
  process.exitCode = 1;
} finally {
  if (browser?.pid && browser.exitCode === null && browser.signalCode === null) {
    await new Promise(resolve => {
      const timer = setTimeout(() => browser.kill("SIGKILL"), 3000);
      browser.once("exit", () => { clearTimeout(timer); resolve(); });
      // Let Chrome flush the test profile and shut down its helpers before
      // closing the pipe. A failed launch can still need a process signal.
      if (devtools && !devtools.closedError) {
        void devtools.command("Browser.close").catch(() => browser.kill());
      } else browser.kill();
    });
  }
  devtools?.close();
  await vite?.close();
  if (temporary) await rm(temporary, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
