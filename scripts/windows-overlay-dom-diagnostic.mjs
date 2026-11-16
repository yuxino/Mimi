// Read the packaged overlay through its test runner's loopback WebView2 port.
// The Windows probe owns all input and session assertions.
async function overlaySnapshot(phase, label) {
  const surface = document.querySelector(".overlay-surface");
  if (!surface) return null;

  const findButton = () => Array.from(surface.querySelectorAll("button[aria-label]"))
    .find((button) => button.getAttribute("aria-label") === label);
  const classify = (element) => {
    if (!(element instanceof Element)) return "none";
    const button = element.closest("button");
    if (button && button.getAttribute("aria-label") === label) return "expected-button";
    if (button?.closest(".overlay-drag-layer")) return "drag-button";
    if (button?.closest(".overlay-hover-control")) return "other-tool";
    if (button) return "other-button";
    return surface.contains(element) ? "surface" : "outside";
  };

  const probeKey = "__mimiWindowsUiTestDomDiagnostic";
  if (phase === "before-move") {
    window[probeKey]?.controller.abort();
    const controller = new AbortController();
    const events = [];
    const counts = {};
    window[probeKey] = { controller, events, counts };
    for (const type of ["blur", "pointerover", "pointerenter", "pointermove", "pointerdown", "pointerup", "mouseover", "mouseenter", "mousemove", "mousedown", "mouseup", "click"]) {
      window.addEventListener(type, (event) => {
        counts[type] = (counts[type] ?? 0) + 1;
        if (events.length === 32) events.shift();
        events.push({
          type, target: classify(event.target), trusted: event.isTrusted, x: event.clientX, y: event.clientY,
          markedHover: surface.hasAttribute("data-overlay-hover"),
        });
      }, { capture: true, passive: true, signal: controller.signal });
    }
  }

  const observer = window[probeKey];
  const movementCount = () => (observer?.counts.pointermove ?? 0) + (observer?.counts.mousemove ?? 0);
  if (observer && phase !== "before-move") {
    const received = () => phase === "after-move"
      ? movementCount() > (observer.movesAtClick ?? 0)
      : observer.counts.click;
    const deadline = performance.now() + 250;
    while (!received() && performance.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 15));
    }
  }

  const button = findButton();
  const rect = button?.getBoundingClientRect();
  const style = button && getComputedStyle(button);
  const toolbar = button?.closest(".overlay-hover-control");
  // Keep observing the fallback move after a missed click. The next action
  // replaces this bounded listener, and closing the test WebView removes it.
  if (observer && phase === "after-click") observer.movesAtClick = movementCount();
  return {
    markedHover: surface.hasAttribute("data-overlay-hover"),
    cssHover: surface.matches(":hover"),
    hoverCapable: matchMedia("(hover: hover) and (pointer: fine)").matches,
    focused: document.hasFocus(),
    viewport: [innerWidth, innerHeight, devicePixelRatio],
    target: rect ? {
      bounds: [rect.x, rect.y, rect.width, rect.height],
      hit: classify(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2)),
      opacity: toolbar ? getComputedStyle(toolbar).opacity : style.opacity,
      visibility: style.visibility,
      pointerEvents: style.pointerEvents,
      disabled: button.disabled,
    } : null,
    counts: observer?.counts ?? {},
    events: observer?.events ?? [],
  };
}

function evaluate(url, expression, signal) {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(new Error("WebView2 observation timed out"));
      return;
    }
    const socket = new WebSocket(url);
    const timedOut = () => finish(new Error("WebView2 observation timed out"));
    let finished = false;
    function finish(error, value) {
      if (finished) return;
      finished = true;
      signal.removeEventListener("abort", timedOut);
      socket.close();
      if (error) reject(error);
      else resolve(value);
    }
    signal.addEventListener("abort", timedOut, { once: true });
    socket.addEventListener("open", () => socket.send(JSON.stringify({
      id: 1, method: "Runtime.evaluate", params: { expression, awaitPromise: true, returnByValue: true },
    })), { once: true });
    socket.addEventListener("message", (event) => {
      let reply;
      try { reply = JSON.parse(event.data); }
      catch {
        finish(new Error("WebView2 response could not be decoded"));
        return;
      }
      if (reply.id !== 1) return;
      if (reply.error || !reply.result?.result || reply.result.exceptionDetails) finish(new Error("WebView2 evaluation failed"));
      else finish(null, reply.result.result.value);
    });
    socket.addEventListener("error", () => finish(new Error("WebView2 connection failed")), { once: true });
  });
}

setTimeout(() => process.exit(1), 5_000).unref();

try {
  const [portArg, phase, label, screenX, screenY, cursorX, cursorY] = process.argv.slice(2);
  const port = Number(portArg);
  if (!Number.isInteger(port) || port < 1 || port > 65_535 ||
      !["before-move", "after-move", "after-click"].includes(phase)) throw new Error("Invalid diagnostic arguments");
  const signal = AbortSignal.timeout(4_000);
  const response = await fetch(`http://127.0.0.1:${port}/json/list`, { signal, redirect: "error" });
  if (!response.ok) throw new Error("WebView2 target lookup failed");
  const targets = await response.json();
  const expression = `(${overlaySnapshot.toString()})(${JSON.stringify(phase)}, ${JSON.stringify(label)})`;
  let observation;
  for (const target of targets) {
    if (target.type !== "page" || !target.webSocketDebuggerUrl) continue;
    const endpoint = new URL(target.webSocketDebuggerUrl);
    if (endpoint.protocol !== "ws:" || endpoint.hostname !== "127.0.0.1" || Number(endpoint.port) !== port) continue;
    observation = await evaluate(endpoint, expression, signal);
    if (observation) break;
  }
  if (!observation) throw new Error("Overlay renderer unavailable");
  const screen = { target: [Number(screenX), Number(screenY)], pointer: [Number(cursorX), Number(cursorY)] };
  console.log(`mimi-overlay-dom ${JSON.stringify({ phase, screen, ...observation })}`);
} catch {
  console.error("Windows overlay DOM diagnostic could not read the local renderer.");
  process.exitCode = 1;
}
