// @vitest-environment jsdom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SessionAudioPlayer, type SessionAudioPlayerProps } from "./SessionAudioPlayer";

const labels = {
  play: "Play recording", pause: "Pause recording", loading: "Loading recording",
  retry: "Retry playback", seek: "Recording position", playFailed: "Could not play this recording.",
  ready: "Recording ready", label: "Session recording",
};
let host: HTMLDivElement;
let root: Root;
let props: SessionAudioPlayerProps;
const onLoad = vi.fn();
let play: ReturnType<typeof vi.spyOn>;
let pause: ReturnType<typeof vi.spyOn>;
let load: ReturnType<typeof vi.spyOn>;
const audio = () => host.querySelector("audio")!;
const button = () => host.querySelector("button")!;
const range = () => host.querySelector<HTMLInputElement>("input[type=range]")!;
const time = () => host.querySelector("output")!.textContent;

async function render(patch: Partial<SessionAudioPlayerProps> = {}, key = "session:system") {
  props = { ...props, ...patch };
  await act(async () => root.render(<SessionAudioPlayer key={key} {...props} />));
}
async function renderStrict(patch: Partial<SessionAudioPlayerProps> = {}) {
  props = { ...props, ...patch };
  await act(async () => root.render(<StrictMode><SessionAudioPlayer {...props} /></StrictMode>));
}
async function click() { await act(async () => button().click()); }
async function event(name: string, element = audio()) {
  await act(async () => element.dispatchEvent(new Event(name, { bubbles: true })));
}
async function metadata(duration = 125, currentTime = 0) {
  Object.defineProperties(audio(), {
    readyState: { configurable: true, value: 1 },
    duration: { configurable: true, value: duration },
    currentTime: { configurable: true, writable: true, value: currentTime },
  });
  await event("loadedmetadata");
}
function deferred() {
  let resolve!: () => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  onLoad.mockReset();
  play = vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue();
  pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  load = vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(() => {});
  props = { src: null, loading: false, readError: false, disabled: false, onLoad, labels };
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("SessionAudioPlayer", () => {
  it("retains the loaded source through StrictMode ref replay and plays only on request", async () => {
    await renderStrict({ src: "blob:strict-restored" });
    expect(audio().getAttribute("src")).toBe("blob:strict-restored");
    expect(play).not.toHaveBeenCalled();
    await click();
    expect(play).toHaveBeenCalledOnce();
    expect(play.mock.contexts[0]).toBe(audio());
    expect(audio().getAttribute("src")).toBe("blob:strict-restored");
  });

  it("preserves a newly read source through StrictMode ref replay before playing it", async () => {
    await renderStrict();
    await click();
    expect(onLoad).toHaveBeenCalledOnce();
    await renderStrict({ loading: true });
    await renderStrict({ loading: false, src: "blob:strict-new" });
    expect(audio().getAttribute("src")).toBe("blob:strict-new");
    expect(play).toHaveBeenCalledOnce();
    expect(play.mock.contexts[0]).toBe(audio());
    await metadata(8);
    await event("playing");
    expect(time()).toBe("00:00 / 00:08");
    expect(button().getAttribute("aria-label")).toBe(labels.pause);
    const element = audio();
    pause.mockClear();
    load.mockClear();
    await act(async () => root.render(null));
    expect(pause).toHaveBeenCalledOnce();
    expect(pause.mock.contexts[0]).toBe(element);
    expect(element.getAttribute("src")).toBeNull();
    expect(load).toHaveBeenCalledOnce();
    expect(load.mock.contexts[0]).toBe(element);
  });

  it("keeps one primary action and unknown time without starting a file read", async () => {
    await render({ sourceControl: <span>System audio</span> });
    expect(button().getAttribute("aria-label")).toBe(labels.play);
    expect(host.querySelectorAll("button")).toHaveLength(1);
    expect(host.textContent).toContain("System audio");
    expect(time()).toBe("00:00 / —:—");
    expect(range().disabled).toBe(true);
    expect(onLoad).not.toHaveBeenCalled();
    expect(play).not.toHaveBeenCalled();
  });

  it("loads once and starts playback before metadata when preload remains at readyState zero", async () => {
    await render();
    await act(async () => { button().click(); button().click(); });
    expect(onLoad).toHaveBeenCalledOnce();
    await render({ loading: true });
    expect(button().disabled).toBe(true);
    expect(button().getAttribute("aria-label")).toBe(labels.loading);
    await render({ src: "blob:system", loading: false });
    expect(audio().readyState).toBe(0);
    expect(play).toHaveBeenCalledOnce();
    expect(play.mock.contexts[0]).toBe(audio());
    expect(time()).toBe("00:00 / —:—");
    expect(range().disabled).toBe(true);
    await metadata();
    expect(play).toHaveBeenCalledOnce();
    expect(button().getAttribute("aria-label")).toBe(labels.play);
    await event("playing");
    expect(button().getAttribute("aria-label")).toBe(labels.pause);
    await metadata();
    expect(play).toHaveBeenCalledOnce();
  });

  it("starts a loaded source on direct click even when its metadata has not arrived", async () => {
    await render({ src: "blob:preload-deferred" });
    expect(audio().readyState).toBe(0);
    expect(play).not.toHaveBeenCalled();
    await click();
    expect(play).toHaveBeenCalledOnce();
    expect(time()).toBe("00:00 / —:—");
    expect(range().disabled).toBe(true);
  });

  it("does not autoplay a restored source and follows actual play/pause events", async () => {
    await render({ src: "blob:restored" });
    await metadata();
    expect(play).not.toHaveBeenCalled();
    await click();
    expect(play).toHaveBeenCalledOnce();
    await event("playing");
    expect(host.textContent).not.toContain(labels.ready);
    pause.mockClear();
    await click();
    expect(pause).toHaveBeenCalledOnce();
    await event("pause");
    expect(button().getAttribute("aria-label")).toBe(labels.play);
    expect(host.textContent).toContain(labels.ready);
    pause.mockClear();
    await event("playing");
    expect(pause).toHaveBeenCalledOnce();
    expect(button().getAttribute("aria-label")).toBe(labels.play);
    expect(onLoad).not.toHaveBeenCalled();
  });

  it.each(["resolve", "reject"])("times out a hanging play, ignores its late %s and playing event, and permits retry", async outcome => {
    vi.useFakeTimers();
    const first = deferred();
    play.mockReturnValueOnce(first.promise);
    await render({ src: "blob:slow" });
    await click();
    expect(button().disabled).toBe(true);
    expect(button().getAttribute("aria-label")).toBe(labels.loading);
    await act(async () => vi.advanceTimersByTime(9_999));
    expect(button().disabled).toBe(true);
    pause.mockClear();
    await act(async () => vi.advanceTimersByTime(1));
    expect(pause).toHaveBeenCalledOnce();
    expect(pause.mock.contexts[0]).toBe(audio());
    expect(button().disabled).toBe(false);
    expect(button().getAttribute("aria-label")).toBe(labels.retry);
    expect(host.textContent).toContain(labels.playFailed);
    expect(vi.getTimerCount()).toBe(0);
    await act(async () => {
      if (outcome === "resolve") first.resolve();
      else first.reject(new Error("expired play"));
    });
    pause.mockClear();
    await event("playing");
    expect(pause).toHaveBeenCalledOnce();
    expect(button().getAttribute("aria-label")).toBe(labels.retry);
    await click();
    expect(play).toHaveBeenCalledTimes(2);
    expect(onLoad).not.toHaveBeenCalled();
    await metadata(8);
    await event("playing");
    expect(button().getAttribute("aria-label")).toBe(labels.pause);
    expect(host.textContent).not.toContain(labels.playFailed);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("accepts an actual playing event even if its play promise remains pending", async () => {
    vi.useFakeTimers();
    const first = deferred();
    play.mockReturnValueOnce(first.promise);
    await render({ src: "blob:playing" });
    await click();
    await event("playing");
    expect(button().disabled).toBe(false);
    expect(button().getAttribute("aria-label")).toBe(labels.pause);
    expect(vi.getTimerCount()).toBe(0);
    pause.mockClear();
    await act(async () => vi.advanceTimersByTime(10_000));
    await act(async () => first.reject(new Error("obsolete promise")));
    expect(pause).not.toHaveBeenCalled();
    expect(button().getAttribute("aria-label")).toBe(labels.pause);
    expect(host.textContent).not.toContain(labels.playFailed);
  });

  it.each(["source", "unmount", "disable"])("clears a pending play deadline on %s", async boundary => {
    vi.useFakeTimers();
    const first = deferred();
    play.mockReturnValueOnce(first.promise);
    await render({ src: "blob:previous" });
    await click();
    expect(vi.getTimerCount()).toBe(1);
    const previous = audio();
    if (boundary === "source") await render({ src: "blob:replacement" });
    else if (boundary === "disable") await render({ disabled: true });
    else await act(async () => root.render(null));
    expect(vi.getTimerCount()).toBe(0);
    pause.mockClear();
    await act(async () => vi.advanceTimersByTime(10_000));
    await act(async () => first.reject(new Error("cancelled play")));
    expect(pause).not.toHaveBeenCalled();
    expect(host.textContent).not.toContain(labels.playFailed);
    if (boundary === "source") {
      await event("playing", previous);
      expect(button().getAttribute("aria-label")).toBe(labels.play);
    } else if (boundary === "disable") {
      await event("playing");
      expect(pause).toHaveBeenCalledOnce();
      expect(button().disabled).toBe(true);
      expect(button().getAttribute("aria-label")).toBe(labels.play);
    } else {
      expect(host.childElementCount).toBe(0);
    }
  });

  it("retries failed file reads using the same action and preserves the new intent", async () => {
    await render();
    await click();
    await render({ readError: true });
    expect(button().getAttribute("aria-label")).toBe(labels.retry);
    expect(host.textContent).toContain(labels.playFailed);
    await click();
    expect(onLoad).toHaveBeenCalledTimes(2);
    await render({ readError: false, loading: true });
    await render({ loading: false, src: "blob:retry" });
    await metadata();
    expect(play).toHaveBeenCalledOnce();
    expect(host.textContent).not.toContain(labels.playFailed);
  });

  it.each(["reject", "throw"])("handles a play %s and retries without re-reading the file", async outcome => {
    await render({ src: "blob:ready" });
    await metadata();
    if (outcome === "reject") play.mockRejectedValueOnce(new DOMException("synthetic policy", "NotAllowedError"));
    else play.mockImplementationOnce(() => { throw new Error("synthetic media failure"); });
    await click();
    expect(button().disabled).toBe(false);
    expect(button().getAttribute("aria-label")).toBe(labels.retry);
    expect(host.textContent).toContain(labels.playFailed);
    await click();
    expect(play).toHaveBeenCalledTimes(2);
    await event("playing");
    expect(button().getAttribute("aria-label")).toBe(labels.pause);
    expect(host.textContent).not.toContain(labels.playFailed);
    expect(onLoad).not.toHaveBeenCalled();
  });

  it("serializes pending play promises and discards late rejection after a source replacement", async () => {
    const first = deferred();
    play.mockReturnValueOnce(first.promise);
    await render({ src: "blob:first" });
    await metadata();
    await act(async () => { button().click(); button().click(); });
    expect(play).toHaveBeenCalledOnce();
    await render({ src: "blob:second" });
    await metadata();
    await act(async () => first.reject(new Error("old playback")));
    expect(host.textContent).not.toContain(labels.playFailed);
    expect(button().getAttribute("aria-label")).toBe(labels.play);
    await click();
    expect(play).toHaveBeenCalledTimes(2);
  });

  it("does not carry an unloaded request into a separately selected source", async () => {
    await render();
    await click();
    await render({ loading: true }, "session:microphone");
    await render({ loading: false, src: "blob:microphone" }, "session:microphone");
    await metadata();
    expect(play).not.toHaveBeenCalled();
  });

  it("cancels pending intent and rejects late play events while disabled", async () => {
    await render();
    await click();
    await render({ disabled: true, src: "blob:stopped" });
    await metadata();
    expect(play).not.toHaveBeenCalled();
    expect(button().disabled).toBe(true);
    await event("playing");
    expect(button().getAttribute("aria-label")).toBe(labels.play);
    await render({ disabled: false });
    expect(play).not.toHaveBeenCalled();
    await click();
    expect(play).toHaveBeenCalledOnce();
  });

  it("ignores a late play rejection after disable and allows a later explicit retry", async () => {
    const pendingPlay = deferred();
    play.mockReturnValueOnce(pendingPlay.promise);
    await render({ src: "blob:ready" });
    await metadata();
    await click();
    await render({ disabled: true });
    await act(async () => pendingPlay.reject(new Error("superseded play")));
    expect(host.textContent).not.toContain(labels.playFailed);
    await render({ disabled: false });
    await click();
    expect(play).toHaveBeenCalledTimes(2);
  });

  it("uses media duration and time updates, and seeking does not start playback", async () => {
    await render({ src: "blob:ready" });
    await metadata(125.8, 5.3);
    expect(time()).toBe("00:05 / 02:05");
    expect(range().disabled).toBe(false);
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(range(), "87.4");
      range().dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(audio().currentTime).toBe(87.4);
    expect(time()).toBe("01:27 / 02:05");
    expect(range().getAttribute("aria-valuetext")).toBe("01:27 / 02:05");
    expect(play).not.toHaveBeenCalled();
    audio().currentTime = 200;
    await event("timeupdate");
    expect(range().value).toBe("125.8");
  });

  it.each([NaN, Infinity, 0])("keeps seeking disabled for an unknown or empty duration (%s)", async duration => {
    await render({ src: "blob:unknown" });
    await metadata(duration, 20);
    expect(time()).toBe("00:00 / —:—");
    expect(range().disabled).toBe(true);
  });

  it("returns to play at the end and restarts the same recording at zero", async () => {
    await render({ src: "blob:ready" });
    await metadata(10);
    await click();
    await event("playing");
    audio().currentTime = 10;
    await event("ended");
    expect(time()).toBe("00:10 / 00:10");
    expect(button().getAttribute("aria-label")).toBe(labels.play);
    await click();
    expect(audio().currentTime).toBe(0);
    expect(play).toHaveBeenCalledTimes(2);
  });

  it("surfaces media decoding errors without claiming playback", async () => {
    await render({ src: "blob:broken" });
    await event("error");
    expect(button().getAttribute("aria-label")).toBe(labels.retry);
    expect(host.textContent).toContain(labels.playFailed);
    expect(button().disabled).toBe(false);
  });

  it.each(["source", "unmount"])("stops and clears the actual audio element on %s", async boundary => {
    await render({ src: "blob:buffered" });
    await metadata();
    const element = audio();
    pause.mockClear();
    load.mockClear();
    if (boundary === "source") await render({ src: "blob:next" });
    else await act(async () => root.render(null));
    expect(pause).toHaveBeenCalledOnce();
    expect(pause.mock.contexts[0]).toBe(element);
    expect(element.getAttribute("src")).toBeNull();
    expect(load).toHaveBeenCalledOnce();
    expect(load.mock.contexts[0]).toBe(element);
  });

  it("handles a late rejection after unmount without creating new playback", async () => {
    const pendingPlay = deferred();
    play.mockReturnValueOnce(pendingPlay.promise);
    await render({ src: "blob:ready" });
    await metadata();
    await click();
    await act(async () => root.render(null));
    await act(async () => pendingPlay.reject(new Error("detached play")));
    expect(host.childElementCount).toBe(0);
    expect(play).toHaveBeenCalledOnce();
  });
});
