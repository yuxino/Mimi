// Playwright CLI run-code harness; requires npm run dev at 127.0.0.1:1420.
// Measures actual CSS geometry at equivalent 100/125/150% scale, not Windows hardware DPI.
async (page) => {
  const result = await page.evaluate(async () => {
    const ReactModule = await import('/node_modules/.vite/deps/react.js'); const React = ReactModule.default || ReactModule;
    const ReactDom = await import('/node_modules/.vite/deps/react-dom_client.js'); const { createRoot } = ReactDom.default || ReactDom;
    const { LanguageStatusCapsule } = await import('/src/windows/overlay-control/LanguageStatusCapsule.tsx');
    const { useStore } = await import('/src/lib/store.ts');
    const { setStoredUiLanguage } = await import('/src/lib/i18n.ts');
    const { languageStatus } = await import('/src/windows/overlay/overlayModel.ts');
    const host = document.createElement('div');
    host.style.cssText = 'position:absolute;top:0;left:0;background:#222;z-index:9999';
    document.body.append(host);
    const root = createRoot(host);
    const failures = [];
    let checked = 0;
    for (const language of ['en', 'zh', 'zh-TW', 'ja', 'de', 'fr', 'ko']) {
      setStoredUiLanguage(language);
      const fixtures = [];
      for (const zoom of [1, 1.25, 1.5]) for (const phase of ['error', 'idle', 'paused', 'translating', 'listening']) for (const mode of ['lowLatency', 'highQuality', 'turbo']) for (const pair of ['auto','zh','en','ja','ko'].flatMap(source=>['zh','en','ja','original'].map(target=>[source,target]))) {
        const settings = {...useStore.getState().settings, sourceLanguage: pair[0], targetLanguage: pair[1], translationMode: mode};
        const id = [language, zoom, phase, mode, ...pair].join('/');
        fixtures.push(React.createElement('div', {key:id,'data-case':id,style:{width:280,zoom,marginBottom:4}}, React.createElement(LanguageStatusCapsule, {phase,settings,status:languageStatus(settings,null),effectiveMode:mode,isPaused:phase==='paused',isWaitingForFinalTranslation:phase==='translating',expanded:false,onToggle:()=>{}})));
      }
      root.render(React.createElement(React.Fragment, null, ...fixtures));
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      for (const fixture of host.children) {
        const button = fixture.querySelector('button');
        const bounds = button.getBoundingClientRect();
        const children = [...button.querySelectorAll('span,strong,svg')];
        const clipped = children.some(child => { const r=child.getBoundingClientRect(); return r.right>bounds.right+0.5 || r.left<bounds.left-0.5; });
        if(clipped || button.scrollWidth > button.clientWidth + 1) failures.push({case:fixture.dataset.case,text:button.textContent,width:button.clientWidth,scroll:button.scrollWidth});
        checked++;
      }
    }
    root.unmount(); host.remove();
    return {checked,failures};
  });
  console.log(JSON.stringify(result));
  if (result.failures.length) throw new Error(JSON.stringify(result.failures));
  return result;
}
