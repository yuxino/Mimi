// Run against the actual React fixture with a managed ego-browser Page.
// Synthetic IPC state is layout evidence, not native/provider acceptance.
export default async function verifyProfileMenuLayout(page, baseUrl = "http://127.0.0.1:1420") {
await page.goto(new URL("/scripts/fixtures/settings-layout.html", baseUrl).href);
await page.waitForFunction(()=>window.layoutReady===true);
const rows=[];
for(const surface of ["overlay","tray"]) for(const width of [320,420]) {
  await page.cdp("Emulation.setDeviceMetricsOverride",{width,height:1000,deviceScaleFactor:1,mobile:false});
  for(const language of ["zh","zh-TW","en","ja","de","fr","ko"]) for(const theme of ["dark","light"]) for(const long of [false,true]) {
    await page.cdp("Emulation.setEmulatedMedia",{features:[{name:"prefers-color-scheme",value:theme}]});
    rows.push(await page.evaluate(async next=>{
      const name=next.long?"Long configuration name / 長い設定名 / 很长的配置名称 ".repeat(3).trim():"哈哈";
      await window.applyLayoutCase({...next,platform:"macos",state:"idle",provider:"alibabaCloud",profileName:name,sourceLanguage:"ko",targetLanguage:"zh",profileOverrides:{languagePreset:{sourceLanguage:"ko",targetLanguage:"zh"}}});
      await document.fonts.ready;
      await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
      const button=[...document.querySelectorAll('[role="combobox"]')].find(node=>node.textContent.trim()===name);
      if(!button) throw new Error("configuration trigger missing "+JSON.stringify(next));
      button.click();
      await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
      const menu=document.querySelector('.mimi-select__menu'), option=menu?.querySelector('[aria-selected="true"]'), label=option?.querySelector('.mimi-select__label'), description=option?.querySelector('.mimi-select__description');
      if(!label||!description) throw new Error("configuration details missing");
      const lr=label.getBoundingClientRect(),dr=description.getBoundingClientRect(),mr=menu.getBoundingClientRect(),issues=[];
      if(dr.top<lr.bottom-1) issues.push("language pair beside name");
      if(!next.long&&label.scrollWidth>label.clientWidth+1) issues.push("short name clipped");
      if(description.scrollWidth>description.clientWidth+1) issues.push("language pair clipped");
      if(mr.left<7||mr.right>innerWidth-7) issues.push("menu overflow");
      if(button.textContent.trim()!==name) issues.push("closed trigger includes description");
      return {...next,pair:description.textContent,menuWidth:mr.width,descriptionWidth:dr.width,descriptionScroll:description.scrollWidth,issues};
    },{surface,width,language,theme,long}));
  }
}
return { checked: rows.length, failures: rows.filter(row => row.issues.length) };
}
