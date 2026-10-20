// Serve with npm run dev, prime / once, then call with a foreground browser
// Page supporting evaluate/goto/waitForFunction and cdp(method, parameters).
// Uses real browser layout/CSSAnimation objects, without native/provider APIs.
export async function verifyOverlayMotion(page, baseUrl = "http://127.0.0.1:1420") {
  await page.goto(new URL("/scripts/fixtures/overlay-motion.html", baseUrl).href);
  await page.waitForFunction(() => window.motionFixtureReady === true);
  await page.cdp("Page.bringToFront", {});
  await page.cdp("Emulation.setFocusEmulationEnabled", { enabled:true });
  await page.cdp("Emulation.setEmulatedMedia", { features:[{ name:"prefers-reduced-motion", value:"no-preference" }] });
  const failures = [];
  const geometry = await page.evaluate(async () => {
    const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
    const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
    const fixture = window.motionFixture;
    const block = id => ({ id:`history-${id}`, source:`Sentence ${id}`, translation:null, createdAt:id, presentation:"history" });
    fixture.renderTimeline([1,2,3,4].map(block), {}, true);
    await pause(250);
    fixture.scrolls = [];
    fixture.renderTimeline([1,2,3,4,5].map(block));
    await pause(350);
    const scrolls = [...fixture.scrolls];
    const timeline = document.querySelector("#timeline .overlay-timeline");
    const tailReached = Math.abs(timeline.scrollTop - (timeline.scrollHeight - timeline.clientHeight)) <= 2;
    fixture.renderTimeline([{ id:"live", source:"Line one", translation:null, createdAt:null, presentation:"live" }], {}, true);
    await pause(200);
    fixture.renderTimeline([{ id:"live", source:"Line one\nLine two\nLine three\nLine four\nLine five", translation:null, createdAt:null, presentation:"live" }]);
    const frames = [];
    for (let index = 0; index < 14; index++) {
      await frame();
      const lane = document.querySelector("#timeline [aria-label]");
      const inner = lane.firstElementChild;
      frames.push({ y:inner.getBoundingClientRect().top - lane.getBoundingClientRect().top,
        transform:getComputedStyle(inner).transform });
    }
    fixture.renderTimeline([1,2,3,4].map(block), {}, true);
    await pause(200);
    fixture.renderTimeline([1,2,3,4,5].map(block));
    await frame(); await frame();
    const clicked = document.querySelector("#timeline .overlay-timeline");
    clicked.dispatchEvent(new PointerEvent("pointerdown", { bubbles:true, buttons:1 }));
    await frame();
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles:true }));
    await pause(350);
    const clickedGlideFollows = !fixture.readingHistory && Math.abs(clicked.scrollTop - (clicked.scrollHeight - clicked.clientHeight)) <= 2;
    return { scrolls, tailReached, frames, clickedGlideFollows };
  });
  if (!geometry.scrolls.some(move => move.behavior === "smooth" && move.top > move.before + 2)) failures.push("new sentence has no scroll distance left to animate");
  if (geometry.scrolls.some(move => move.behavior === "instant" && Math.abs(move.after - move.before) > 2)) failures.push("measurement snaps an active sentence glide");
  if (!geometry.tailReached) failures.push("sentence glide does not reach the final measured tail");
  if (!geometry.clickedGlideFollows) failures.push("click during a sentence glide enters history or stops following");
  if (geometry.frames.some(frame => frame.y > 1)) failures.push("chunk growth jumps the first line downward");
  if (!geometry.frames.some(frame => frame.y > -45) || geometry.frames.at(-1).y > -47) failures.push("chunk growth does not glide to the bounded tail");

  const preferences = [];
  for (const reduced of [false, true]) {
    await page.cdp("Emulation.setEmulatedMedia", { features:[{ name:"prefers-reduced-motion", value:reduced ? "reduce" : "no-preference" }] });
    for (const explicit of [null, true, false]) {
      const lights = await page.evaluate(async explicit => {
        window.motionFixture.renderLights({ explicit });
        await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        return [...document.querySelectorAll("section .sound-light")].map(light => {
          const wave = light.querySelector(".sound-light__wave");
          const style = getComputedStyle(wave);
          return { surface:light.parentElement.id, duration:style.animationDuration, iterations:style.animationIterationCount,
            running:light.getAnimations({ subtree:true }).filter(animation => animation instanceof CSSAnimation && animation.playState === "running").length };
        });
      }, explicit);
      const enabled = explicit ?? !reduced;
      for (const light of lights) {
        if (enabled && (light.duration !== "3.6s" || light.iterations !== "infinite" || light.running !== 2)) failures.push(`resolved motion missing: ${light.surface}/${reduced}/${explicit}`);
        if (!enabled && light.running !== 0) failures.push(`disabled motion still running: ${light.surface}/${reduced}/${explicit}`);
      }
      preferences.push({ reduced, explicit, lights });
    }
  }
  const clocks = await page.evaluate(async () => {
    const fixture = window.motionFixture;
    const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
    const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
    fixture.renderLights({ explicit:true });
    await frame();
    const light = document.querySelector("#overlay .sound-light");
    const wave = light.querySelector(".sound-light__wave");
    const animation = wave.getAnimations()[0];
    fixture.renderLights({ explicit:true, phase:"translating" });
    await pause(550);
    const outgoingPaused = animation.playState === "paused";
    fixture.renderLights({ explicit:true, phase:"listening" });
    const sameAnimation = wave.getAnimations()[0] === animation;
    const resumed = animation.playState === "running";
    fixture.renderLights({ explicit:true, animationVisible:false });
    const hiddenPaused = light.getAnimations({ subtree:true }).filter(animation => animation instanceof CSSAnimation).every(animation => animation.playState === "paused");
    fixture.renderLights({ explicit:true });
    return { outgoingPaused, sameAnimation, resumed, hiddenPaused };
  });
  if (Object.values(clocks).some(passed => !passed)) failures.push("track pause/resume loses clock or animation identity");
  return { checked:30, failures, geometry, preferences, clocks };
}
