// Installed only in the Windows synthetic overlay when the pointer probe is enabled.
(() => {
  if (window.top !== window) return;

  const events = [];
  const actions = new Map([
    ["Pause translating", "pause"],
    ["Resume translating", "resume"],
    ["Open Mimi Settings", "settings"],
    ["Open speech settings", "error-settings"],
  ]);
  const classify = (element) => {
    if (!(element instanceof Element)) return "outside";
    const button = element.closest("button");
    if (button) {
      if (button.closest(".overlay-drag-layer")) return "drag";
      return actions.get(button.getAttribute("aria-label")) ?? "other-button";
    }
    return element.closest(".overlay-surface") ? "surface" : "outside";
  };
  const coordinates = (event, x, y) => Number.isFinite(event[x]) && Number.isFinite(event[y])
    ? [Math.round(event[x]), Math.round(event[y])]
    : null;

  for (const kind of [
    "blur", "pointerover", "pointerenter", "pointerleave", "pointermove", "pointerdown", "pointerup",
    "mouseover", "mouseenter", "mouseleave", "mousemove", "mousedown", "mouseup", "click",
  ]) {
    window.addEventListener(kind, (event) => {
      if (events.length === 32) events.shift();
      events.push({
        kind,
        target: classify(event.target),
        local: coordinates(event, "clientX", "clientY"),
        screen_css: coordinates(event, "screenX", "screenY"),
        trusted: event.isTrusted,
        marked_hover: Boolean(document.querySelector(".overlay-surface[data-overlay-hover]")),
      });
    }, { capture: true, passive: true });
  }

  window.__mimiUiTestOverlayPointer = () => {
    const surface = document.querySelector(".overlay-surface");
    if (!surface) return null;
    const button = surface.querySelector('button[aria-label="Pause translating"], button[aria-label="Resume translating"]');
    const rect = button?.getBoundingClientRect();
    const style = button && getComputedStyle(button);
    const toolbar = button?.closest(".overlay-hover-control");
    return {
      marked_hover: surface.hasAttribute("data-overlay-hover"),
      css_hover: surface.matches(":hover"),
      hover_capable: matchMedia("(hover: hover) and (pointer: fine)").matches,
      focused: document.hasFocus(),
      device_pixel_ratio: devicePixelRatio,
      target: rect ? {
        bounds: [rect.x, rect.y, rect.width, rect.height].map(Math.round),
        hit: classify(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2)),
        opacity_percent: Math.round(Number(toolbar ? getComputedStyle(toolbar).opacity : style.opacity) * 100),
        visible: style.visibility === "visible",
        pointer_events: style.pointerEvents !== "none",
        disabled: button.disabled,
      } : null,
      events: events.splice(0),
    };
  };
})();
