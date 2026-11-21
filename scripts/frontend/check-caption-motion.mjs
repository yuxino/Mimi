#!/usr/bin/env node
// Execute the existing browser geometry assertions with an installed Chromium.
// The browser profile and Vite listener belong only to this process.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { access, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

class DevTools {
  calls = new Map();
  events = new Set();
  nextId = 0;

  constructor(socket) {
    this.socket = socket;
    socket.addEventListener("message", event => {
      const message = JSON.parse(event.data);
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
    });
    const disconnected = () => {
      for (const call of this.calls.values()) { clearTimeout(call.timer); call.reject(new Error("Chromium disconnected")); }
      this.calls.clear();
      for (const listener of this.events) listener.reject(new Error("Chromium disconnected"));
    };
    socket.addEventListener("close", disconnected);
    socket.addEventListener("error", disconnected);
  }

  command(method, params = {}, sessionId) {
    return new Promise((resolve, reject) => {
      const id = ++this.nextId;
      const timer = setTimeout(() => { this.calls.delete(id); reject(new Error(`${method} timed out`)); }, 30000);
      this.calls.set(id, { method, resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }

  event(method, sessionId) {
    return new Promise((resolve, reject) => {
      const finish = (callback, value) => { clearTimeout(timer); this.events.delete(listener); callback(value); };
      const listener = { method, sessionId, resolve: value => finish(resolve, value), reject: error => finish(reject, error) };
      const timer = setTimeout(() => listener.reject(new Error(`${method} timed out`)), 30000);
      this.events.add(listener);
    });
  }
}

async function openSocket(url) {
  const socket = new WebSocket(url);
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => { socket.close(); reject(new Error("Chromium connection timed out")); }, 10000);
    socket.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
    socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Chromium connection failed")); }, { once: true });
  });
  return socket;
}

// Only the page methods used by subtitleMotionOnly; all DOM input is real CDP
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

let vite, browser, socket, temporary;
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
  browser = spawn(executable, ["--headless=new", "--remote-debugging-address=127.0.0.1", "--remote-debugging-port=0",
    `--user-data-dir=${temporary}`, "--disable-background-networking", "--disable-component-update", "--disable-sync",
    "--no-first-run", "--no-default-browser-check", "--password-store=basic", "--use-mock-keychain", "about:blank"],
  { stdio: ["ignore", "ignore", "pipe"] });
  let browserError = "";
  let launchError;
  browser.stderr.on("data", data => { browserError = (browserError + data).slice(-1500); });
  browser.once("error", error => { launchError = error; });
  let endpoint;
  const deadline = Date.now() + 15000;
  while (!endpoint) {
    if (launchError) throw launchError;
    if (browser.exitCode !== null || browser.signalCode !== null) throw new Error(`Chromium exited: ${browserError}`);
    try {
      const [port, route] = (await readFile(path.join(temporary, "DevToolsActivePort"), "utf8")).trim().split("\n");
      if (/^\d+$/.test(port) && /^\/devtools\/browser\/[a-zA-Z0-9-]+$/.test(route)) endpoint = `ws://127.0.0.1:${port}${route}`;
    } catch (error) { if (error.code !== "ENOENT") throw error; }
    if (Date.now() > deadline) throw new Error(`Chromium debugging endpoint did not start: ${browserError}`);
    if (!endpoint) await delay(25);
  }
  socket = await openSocket(endpoint);
  const devtools = new DevTools(socket);
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
  const script = await readFile(path.join(root, "scripts/frontend/verify-overlay-service-layout.js"), "utf8");
  const verify = new Function(`return (${script}\n);`)();
  const result = await verify(page, { baseUrl, subtitleMotionOnly: true });
  assert.equal(result.checked, 4, "Expected quiet/hover × motion off/on");
  for (const hovered of [false, true]) for (const motion of [false, true]) {
    const cases = result.subtitleMotion.filter(item => item.hovered === hovered && item.motion === motion);
    assert.equal(cases.length, 1, "Missing or duplicate caption motion case");
    assert.ok(cases[0].frames > 0, "Caption geometry was not sampled");
  }
  assert.deepEqual(errors, [], "Unexpected browser errors");
  console.log(JSON.stringify({ browser: (await devtools.command("Browser.getVersion")).product, ...result }));
} finally {
  socket?.close();
  if (browser?.pid && browser.exitCode === null && browser.signalCode === null) {
    await new Promise(resolve => {
      const timer = setTimeout(() => browser.kill("SIGKILL"), 3000);
      browser.once("exit", () => { clearTimeout(timer); resolve(); });
      browser.kill();
    });
  }
  await vite?.close();
  if (temporary) await rm(temporary, { recursive: true, force: true });
}
