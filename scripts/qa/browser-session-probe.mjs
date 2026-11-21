#!/usr/bin/env node
// Execute the existing browser geometry assertions with an installed Chromium.
// The browser profile and Vite listener belong only to this process.
import { spawn } from "node:child_process";
import { access, mkdtemp, readFile, rm, mkdir, appendFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

let observation;

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
      observation.fd4FirstMs ??= Date.now() - observation.startedMs;
      observation.fd4Bytes += Buffer.byteLength(chunk);
      incoming += chunk;
      let boundary;
      while ((boundary = incoming.indexOf("\0")) >= 0) {
        const frame = incoming.slice(0, boundary);
        incoming = incoming.slice(boundary + 1);
        if (!frame) continue;
        observation.fd4Frames++;
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
        error => {
          if (method === "Browser.getVersion") observation.fd3WriteMs = Date.now() - observation.startedMs;
          if (error) this.disconnected(error);
        });
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
  const move = (x, y) => {
    position = { x, y };
    // Chromium uses the active button to deliver a drag to the captured
    // handle. The buttons bitmask alone delivers the move beneath the pointer.
    return cdp("Input.dispatchMouseEvent", { type: "mouseMoved", ...position, button: buttons ? "left" : "none", buttons });
  };
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


const baseSource = "21bd148d253686de6013f761cc93d79e8a4afbf3";
async function sessionBus() {
  const value = process.env.DBUS_SESSION_BUS_ADDRESS;
  if (value === undefined) return { present: false };
  const parts = value.split(";").filter(Boolean);
  const kinds = parts.map(part => part.split(":", 1)[0]);
  const known = new Set(["unix", "tcp", "nonce-tcp", "autolaunch", "launchd"]);
  const result = { present: true, empty: value.length === 0,
    transports: kinds.map(kind => known.has(kind) ? kind : "other"),
    recognizedStructure: parts.length > 0 && kinds.every(kind => known.has(kind)) };
  const local = parts.find(part => part.startsWith("unix:path="));
  if (local) {
    try { result.unixPathIsSocket = (await stat(decodeURIComponent(local.slice(10).split(",")[0]))).isSocket(); }
    catch { result.unixPathIsSocket = false; }
  }
  return result;
}
function sanitize(value) {
  return String(value).split(process.env.DBUS_SESSION_BUS_ADDRESS || "\0").join("<session-bus-address>")
    .replaceAll(/mimi-caption-motion-[A-Za-z0-9]+/g, "<private-profile>");
}
async function resources() {
  const load = (await readFile("/proc/loadavg", "utf8")).trim().split(/\s+/);
  const mem = await readFile("/proc/meminfo", "utf8");
  return { load1: Number(load[0]), load5: Number(load[1]), runnable: Number(load[3].split("/")[0]),
    memAvailableKiB: Number(mem.match(/^MemAvailable:\s+(\d+)/m)?.[1]) };
}
async function sample(stage, arm, index) {
  observation = { stage, arm, index: Number(index), baseSource, node: process.version,
    image: process.env.ImageOS, imageVersion: process.env.ImageVersion,
    chromeCLI: process.env.MIMI_QA_CHROME_VERSION, sessionBus: await sessionBus(), resources: await resources(),
    fd4Bytes: 0, fd4Frames: 0, stderrBytes: 0, dbusParseErrors: 0 };
let firstErrors = "", lastErrors = "";
let vite, browser, devtools, temporary;
try {
  let executable;
  for (const candidate of [process.env.CHROME_BIN, "/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"].filter(Boolean)) {
    try { await access(candidate); executable = candidate; break; } catch { /* Try the next installed browser. */ }
  }
  if (!executable) throw new Error("Caption geometry requires the runner's installed Chromium; set CHROME_BIN if necessary.");
  temporary = await mkdtemp(path.join(tmpdir(), "mimi-caption-motion-"));
  observation.profileMode = (await stat(temporary)).mode & 0o777;
  if (observation.profileMode !== 0o700) throw new Error("QA profile is not private");
  const { createServer } = await import("vite");
  vite = await createServer({ root, configFile: path.join(root, "config/vite.config.ts"), envDir: false,
    logLevel: "error", server: { host: "127.0.0.1", port: 0, strictPort: false } });
  await vite.listen();
  const address = vite.httpServer.address();
  const baseUrl = `http://127.0.0.1:${address.port}`;
  const started = Date.now();
  observation.startedMs = started;
  // Chromium's standard inherited pipe avoids endpoint-file discovery and a
  // debugging TCP listener. The documentation renderer uses the same fd3/fd4.
  browser = spawn(executable, ["--remote-debugging-pipe",
    `--user-data-dir=${temporary}`, "--disable-background-networking", "--disable-component-update", "--disable-sync",
    "--no-first-run", "--no-default-browser-check", "--password-store=basic", "--use-mock-keychain", "about:blank"],
  { stdio: ["ignore", "ignore", "pipe", "pipe", "pipe"] });
  browser.stderr.setEncoding("utf8");
  browser.stderr.on("data", data => {
    observation.stderrBytes += Buffer.byteLength(data);
    observation.dbusParseErrors += (data.match(/Could not parse server address/g) ?? []).length;
    firstErrors = (firstErrors + data).slice(0, 4000);
    lastErrors = (lastErrors + data).slice(-4000);
  });
  const diagnostic = () => JSON.stringify({ executable, elapsedMs: Date.now() - started,
    exitCode: browser.exitCode, signal: browser.signalCode, stderrFirst: firstErrors, stderrLast: lastErrors });
  devtools = new DevTools(browser, diagnostic);
  const version = await devtools.command("Browser.getVersion", {}, undefined, Math.max(1, 15000 - (Date.now() - started)));
  observation.outcome = "ready";
  observation.elapsedMs = Date.now() - started;
  observation.browser = version.product;

} catch (error) {
  // Preserve the assertion or launch error even if cleanup also fails.
  observation.outcome = "failure";
  observation.elapsedMs = observation.startedMs ? Date.now() - observation.startedMs : null;
  observation.error = sanitize(error.message);
} finally {
  observation.fd4BytesAtOutcome = observation.fd4Bytes;
  observation.fd4FramesAtOutcome = observation.fd4Frames;
  observation.stderrFirst = sanitize(firstErrors);
  observation.stderrLast = sanitize(lastErrors);
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
  observation.exitCode = browser?.exitCode ?? null;
  observation.signal = browser?.signalCode ?? null;
  if (temporary) await rm(temporary, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}

  delete observation.startedMs;
  console.log(JSON.stringify(observation));
}
async function matrix(stage, order, evidence) {
  await mkdir(evidence, { recursive: true });
  for (let index = 0; index < order.length; index++) {
    const arm = order[index] === "A" ? "original" : "dbus-session";
    const args = [fileURLToPath(import.meta.url), "--sample", stage, arm, String(index + 1)];
    const child = arm === "original" ? spawn(process.execPath, args) : spawn("dbus-run-session", ["--", process.execPath, ...args]);
    let output = "", errors = "";
    child.stdout.setEncoding("utf8"); child.stdout.on("data", chunk => { output += chunk; });
    child.stderr.setEncoding("utf8"); child.stderr.on("data", chunk => { errors += chunk; });
    const code = await new Promise((resolve, reject) => { child.once("error", reject); child.once("exit", resolve); });
    let result;
    try { result = JSON.parse(output.trim()); }
    catch { throw new Error(`QA child did not produce a result: ${stage}/${arm}/${index + 1}, exit=${code}, stderrBytes=${errors.length}`); }
    result.wrapperExitCode = code;
    result.wrapperStderrBytes = errors.length;
    await appendFile(path.join(evidence, "samples.jsonl"), JSON.stringify(result) + "\n");
    console.log(JSON.stringify(result));
  }
}
if (process.env.GITHUB_ACTIONS !== "true" || process.env.RUNNER_ENVIRONMENT !== "github-hosted" || process.platform !== "linux") {
  throw new Error("This probe is restricted to the disposable GitHub-hosted Linux runner");
}
const args = process.argv.slice(2);
if (args[0] === "--sample") await sample(args[1], args[2], args[3]);
else if (args[0] === "--matrix" && ["ABBA", "BAAB"].includes(args[2])) await matrix(args[1], args[2], args[3]);
else throw new Error("Expected a fixed QA matrix or its one-shot sample");
