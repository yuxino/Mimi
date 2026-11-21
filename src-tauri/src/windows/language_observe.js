// Document-start observer for one offline synthetic child. Only known element
// categories, bundled filenames, native events, numbers and CSS values leave it.
(() => {
  if (window.top !== window || window.__MIMI_CAPSULE_PROBE_STAGE__) return;
  const safeKinds = new Set(["Error", "TypeError", "SyntaxError", "ReferenceError", "RangeError", "DOMException", "AggregateError"]);
  const nativeModes = ["hidden", "island", "panel"];
  const queue = [];
  let sending = false, sequence = 0, lastSnapshot = "", lastBeat = 0, lastMove = 0;
  let reactMode = null, appRoute = null, failedTransfers = 0;
  const n = x => Number.isFinite(x) ? Math.round(x * 10) / 10 : null;
  const bridge = () => window.__TAURI_INTERNALS__;
  const kind = value => safeKinds.has(value?.name) ? value.name : "other";
  const asset = input => {
    let name;
    try {
      const url = new URL(input, location.href);
      if (!["tauri:", "http:", "https:"].includes(url.protocol) || url.origin !== location.origin) return null;
      name = url.pathname.split("/").pop();
    } catch { return null; }
    return /^[\w.-]{1,100}\.(?:m?js|css)$/.test(name) ? name : null;
  };
  const stack = error => Array.from(String(error?.stack || "").matchAll(/(?:^|[/])([\w.-]{1,100}\.m?js):(\d+):(\d+)/gm))
    .slice(0, 8).map(match => [match[1], +match[2], +match[3]]);
  const errorClass = error => {
    const s = String(error?.message || error || "");
    if (/before (initialization|initializing)|uninitialized variable/i.test(s)) return "initialization-order";
    if (/load|importing a module/i.test(s) && /module|script|stylesheet/i.test(s)) return "module-load";
    if (/undefined|null is not an object|properties of null/i.test(s)) return "null-or-undefined";
    if (/is not a function/i.test(s)) return "not-callable";
    if (/react.*error|reactjs\.org\/docs\/error|react\.dev\/errors/i.test(s)) return "react-error";
    return "other";
  };
  async function flush() {
    const api = bridge();
    if (sending || !api || typeof api.invoke !== "function") return;
    sending = true;
    try {
      while (queue.length) {
        await api.invoke("app_ui_test_frontend_ready", {capsuleProbe: queue[0]});
        queue.shift();
      }
    } catch { failedTransfers++; }
    finally { sending = false; }
  }
  function emit(type, details = {}) {
    if (sequence >= 260) return;
    queue.push({kind: type, seq: ++sequence, wallMs: Date.now(), perfMs: n(performance.now()), failedTransfers, ...details});
    void flush();
  }
  function describe(el) {
    if (!(el instanceof Element)) return "none";
    if (el.closest(".overlay-control-header__collapse")) return "collapse";
    if (el.closest(".overlay-control-island")) return "capsule";
    if (el.closest("[role=combobox]")) return "combobox";
    if (el.closest(".overlay-control-panel")) return "panel";
    if (el.closest(".overlay-control-island-measure")) return "measure";
    if (el.closest(".overlay-control-detached")) return "surface";
    if (el.closest("#root")) return "react-root";
    return el === document.body ? "body" : el === document.documentElement ? "html" : "other";
  }
  function view(el) {
    if (!el) return null;
    const style = getComputedStyle(el), b = el.getBoundingClientRect();
    return {box: [b.x,b.y,b.width,b.height].map(n), display: style.display, visibility: style.visibility,
      opacity: style.opacity, pointerEvents: style.pointerEvents, ariaHidden: el.getAttribute("aria-hidden") === "true",
      hidden: el.hasAttribute("hidden"), inert: el.hasAttribute("inert")};
  }
  function take(reason, force, pointer) {
    const q = s => document.querySelector(s);
    const capsule = q(".overlay-control-island"), surface = q(".overlay-control-detached");
    const b = capsule?.getBoundingClientRect();
    const x = b?.width ? b.x + b.width / 2 : innerWidth / 2, y = b?.height ? b.y + b.height / 2 : innerHeight / 2;
    const state = {doc: document.readyState, route: appRoute, reactMode, focus: document.hasFocus(),
      location: [location.protocol, ["/","/index.html"].includes(location.pathname) ? "index" : "other"],
      bridgeWindow: ["overlay", "overlay-control", "settings"].find(v => v === bridge()?.metadata?.currentWindow?.label) || null,
      viewport: [innerWidth,innerHeight, devicePixelRatio], root: view(q("#root")), surface: view(surface),
      attrs: ["data-quiet-island","data-overlay-hover","data-overlay-pointer-tracked","data-overlay-peer-hover","data-overlay-keyboard-focus"]
        .filter(name => surface?.hasAttribute(name)),
      parent: view(capsule?.parentElement), measure: view(q(".overlay-control-island-measure")),
      capsule: view(capsule), panel: view(q(".overlay-control-panel")),
      capsuleName: capsule?.getAttribute("aria-controls") === "overlay-control-panel" ? "panel-trigger" : capsule ? "other" : null,
      hit: [n(x),n(y),describe(document.elementFromPoint(x,y))], active:describe(document.activeElement),
      scripts: [...document.scripts].map(node => asset(node.src)).filter(Boolean),
      bundles: [...new Set(performance.getEntriesByType("resource").map(entry => asset(entry.name)).filter(Boolean))].slice(0, 45)
    };
    const key = JSON.stringify(state);
    if (force || key !== lastSnapshot || performance.now() - lastBeat > 1100) {
      lastSnapshot = key; lastBeat = performance.now();
      emit("surface", {reason, ...state, ...(pointer ? {pointer} : {})});
    }
  }
  window.__MIMI_CAPSULE_PROBE_STAGE__ = (name, value) => {
    if (name === "mode" && nativeModes.includes(value)) reactMode = value;
    else if (name === "route" && ["overlay","overlay-control","settings"].includes(value)) appRoute = value;
    else if (!["lazy-resolved","child-committed"].includes(name)) return;
    emit("react", {stage:name, route:appRoute, reactMode});
  };
  function fail(event, e) {
    emit("js-failure", {stage:event.type, errorType:kind(e), category:errorClass(e),
      filename: asset(event.filename || event.target?.src || event.target?.href || ""),
      line: n(event.lineno), column:n(event.colno), frames:stack(e)});
  }
  window.addEventListener("error", event => fail(event, event.error || event.message), true);
  window.addEventListener("unhandledrejection", event => fail(event, event.reason));
  for (const type of ["DOMContentLoaded","load","resize","pageshow"]) {
    window.addEventListener(type, () => take(type, true));
  }
  for (const type of ["pointerenter","pointermove","pointerleave","pointerdown","pointerup","click"]) {
    window.addEventListener(type, ev => {
      if (!ev.isTrusted) return;
      if (type === "pointermove" && performance.now() - lastMove < 100) return;
      if (type === "pointermove") lastMove = performance.now();
      take(type, true, {x:n(ev.clientX),y:n(ev.clientY),target:describe(ev.target),hit:describe(document.elementFromPoint(ev.clientX,ev.clientY))});
    }, true);
  }
  emit("document-start", {doc:document.readyState, originMs:n(performance.timeOrigin)});
  setInterval(() => { void flush(); take("sample", false); }, 225);
})();
