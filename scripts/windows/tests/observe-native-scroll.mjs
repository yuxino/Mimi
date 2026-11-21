import { writeFile } from "node:fs/promises";
import { resolve, relative, isAbsolute } from "node:path";

const [action, evidence] = process.argv.slice(2);
const port = Number(process.env.MIMI_QA_WEBVIEW_PORT);
const mode = process.env.MIMI_QA_SCHEDULE;
if (process.env.RUNNER_ENVIRONMENT !== "github-hosted" || process.platform !== "win32" ||
    process.env.MIMI_UI_TEST !== "1" || process.env.MIMI_UI_TEST_CAPTION_HISTORY !== "1" ||
    !Number.isInteger(port) || port < 1024 || port > 65535 ||
    !["observe", "down", "last-scroll"].includes(mode) || !["install", "dump"].includes(action)) {
  throw Error("Expected an isolated hosted Windows subtitle fixture");
}
const inside = relative(resolve(process.env.RUNNER_TEMP), resolve(evidence));
if (isAbsolute(inside) || inside.startsWith("..")) throw Error("Expected the test evidence directory");

async function attach(target) {
  const url = new URL(target.webSocketDebuggerUrl);
  if (url.protocol !== "ws:" || !["localhost","127.0.0.1"].includes(url.hostname) || Number(url.port)!==port) throw Error("Unexpected local test endpoint");
  const socket = new WebSocket(url);
  await new Promise((ok,fail)=>{socket.addEventListener("open",ok,{once:true});socket.addEventListener("error",fail,{once:true});});
  let id=0;
  const evaluate = expression => new Promise((ok,fail)=>{
    const n=++id;
    const onMessage=event=>{
      const data=JSON.parse(event.data); if(data.id!==n)return;
      clearTimeout(timeout);socket.removeEventListener("message",onMessage);
      if(data.error || data.result?.exceptionDetails)fail(Error("The synthetic page rejected the observation"));
      else ok(data.result.result.value);
    };
    const timeout=setTimeout(()=>{socket.removeEventListener("message",onMessage);fail(Error("The test page did not respond"));},4000);
    socket.addEventListener("message",onMessage);
    socket.send(JSON.stringify({id:n,method:"Runtime.evaluate",params:{expression,returnByValue:true}}));
  });
  return {socket,evaluate};
}
function observe(schedule) {
  const host=document.querySelector(".overlay-timeline");
  if(!host || window.__mimiReleaseObservation) return false;
  const rows=[];let dragging=false,stalled=false;
  const record=(kind,event)=>{
    const tail=host.scrollHeight-host.clientHeight;
    const top=host.scrollTop;
    if(rows.length<450)rows.push({kind,ms:Math.round(performance.now()),top,tail,
      footer:!!document.querySelector(".overlay-return-to-live"),target:event?.target===host,
      buttons:event && "buttons" in event?event.buttons:null,
      x:event && "clientX" in event?event.clientX:null,y:event && "clientY" in event?event.clientY:null});
  };
  function stall() {stalled=true;record("processing-start");const until=performance.now()+180;while(performance.now()<until){}record("processing-end");}
  for(const type of ["pointerdown","pointerup","pointercancel","mousedown","mouseup","scroll","scrollend","blur"]) {
    window.addEventListener(type,event=>{
      if((type==="scroll" || type==="scrollend") && event.target!==host)return;
      record(type,event);
      if(type==="pointerdown" && event.target===host && event.button===0 && event.clientX>=host.getBoundingClientRect().right-7) {
        dragging=true;if(schedule==="down"&&!stalled)stall();
      }
      if(!stalled && dragging && schedule==="last-scroll" && type==="scroll"){
        const remaining=host.scrollHeight-host.clientHeight-host.scrollTop;
        if(remaining>2 && remaining<70)stall();
      }
      if(type==="pointerup" || type==="pointercancel")dragging=false;
    },true);
  }
  window.__mimiReleaseObservation=()=>{record("snapshot");return {schedule,stalled,rows};};
  return true;
}
const targets=await fetch("http://127.0.0.1:"+port+"/json/list",{signal:AbortSignal.timeout(5000)}).then(r=>r.json());
let found=0;
for(const target of targets){
  if(target.type!=="page")continue;
  let url;try{url=new URL(target.url)}catch{continue}
  if(!["http:","https:"].includes(url.protocol)||url.hostname!=="tauri.localhost")continue;
  const {socket,evaluate}=await attach(target);
  try{
    if(!await evaluate('window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label==="overlay" && !!document.querySelector(".overlay-timeline")'))continue;
    found++;
    if(action==="install"){
      if(!await evaluate("("+observe.toString()+")("+JSON.stringify(mode)+")"))throw Error("Could not attach to one fresh subtitle viewport");
    } else {
      const report=await evaluate("window.__mimiReleaseObservation?.()");
      if(!report?.rows?.length||report.rows.length>450||report.schedule!==mode)throw Error("Missing bounded observation");
      await writeFile(resolve(evidence,"webview-events.json"),JSON.stringify(report));
      console.log("Recorded",report.rows.length,"input/scroll observations; controlled delay",report.stalled);
    }
  }finally{socket.close();}
}
if(found!==1)throw Error("Expected exactly one mounted synthetic subtitle viewport, found "+found);
