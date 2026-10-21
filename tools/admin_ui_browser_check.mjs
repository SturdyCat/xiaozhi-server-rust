#!/usr/bin/env node
/** Real-Chrome UI-only smoke check. No packages, build, production server or model execution.
 * Usage: node tools/admin_ui_browser_check.mjs [--probe] [--out=client/build/ui-validation/name]
 * Requires a freshly published client/apps/h5App/web artifact for final assertions.
 * Fixture responses are explicitly synthetic, never model/hardware validation.
 */
import http from 'node:http';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile, stat } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const built = path.join(root, 'client/apps/h5App/web');
const allowedOutput = path.join(root, 'client/build/ui-validation');
const probe = process.argv.includes('--probe');
const outArg = process.argv.find(a => a.startsWith('--out='))?.slice(6);
const output = path.resolve(root, outArg || `client/build/ui-validation/${probe ? 'probe' : 'fresh'}-${new Date().toISOString().replace(/[:.]/g, '-')}`);
if (!output.startsWith(allowedOutput + path.sep)) throw new Error('Outputs must remain under client/build/ui-validation/');
const chromePath = process.env.CHROME_PATH || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const token = 'fixture-only-ui-smoke-token-not-a-secret';
const revision = 'fixture-saved-revision-001';
const stages = ['VAD','ASR','NLU','LLM','TTS'];
const dots = ['流程开始','VAD → ASR','ASR → NLU','NLU → LLM','LLM → TTS','流程结束'];
const nodes = [
  {label:'终端与音频入口',dot:0,action:'输入能力验证',actions:['输入能力验证','文本流程验证','音频流程验证']},
  {label:'唤醒与问候',dot:0,action:'问候配置验证'},
  {label:'VAD · 语音分段',stage:'VAD',action:'样本分段'},
  {label:'ASR · 语音识别',stage:'ASR',action:'样本识别'},
  {label:'会话指令',dot:2,action:'指令匹配验证'},
  {label:'NLU · 意图与技能',stage:'NLU',action:'文本意图测试'},
  {label:'人格与提示词',dot:3,action:'提示词预览'},
  {label:'记忆召回',dot:3,action:'召回验证'},
  {label:'LLM · 对话生成',stage:'LLM',action:'节点调用测试'},
  {label:'TTS · 语音合成',stage:'TTS',action:'服务端合成测试'},
  {label:'终端输出适配',dot:5,action:'输出能力验证'},
  {label:'轮次结束与记忆写回',dot:5,action:'写回计划验证'},
];
const actionKinds = {'input.profile':'none','wakeup.preview':'none','vad.sample':'sample_or_pcm','asr.sample':'sample_or_pcm','asr.recording':'native','command.match':'text','nlu.classify':'text','soul.preview':'text','memory.recall':'text','llm.chat':'text','tts.synthesis':'text','tts.playback':'native','output.profile':'none','memory.writeback.dry_run':'text','pipeline.text':'text','pipeline.audio':'sample_or_pcm'};
const report = { scope: 'Real Chrome rendering of actual published Kuikly H5 bundles with deterministic, synthetic HTTP fixtures ONLY', mode: probe ? 'probe' : 'full', startedAt: new Date().toISOString(), output, checks: [], screenshots: [], requests: [], browserExceptions: [], consoleErrors: [], expectedFixtureWebsocketUnavailable: [], resourceFailures: [], limitations: ['No model inference, hardware, microphone, native IME, native playback, or real audio transport validated.', 'Fixture HTTP server is read-only; config POST always returns 409 without persisting.', 'Not interacting with the DSH GUI; only the script-owned read-only fixture origin is navigated.'], artifacts: {} };
const plans = [];
const delays = new Set();
let server, chrome, cdp, origin;
let navigationSequence = 0;
function check(name, pass, evidence) { report.checks.push({name, pass: !!pass, evidence}); console.log(`${pass ? 'PASS' : 'FAIL'} ${name}`); }
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn, timeout = 12000, description = 'condition') { const start = Date.now(); while (Date.now() - start < timeout) { const value = await fn(); if (value) return value; await delay(100); } throw new Error(`Timed out: ${description}`); }
function json(res, value, status = 200, wait = 0) { const send = () => { if (!res.destroyed) { res.writeHead(status, {'Content-Type':'application/json; charset=utf-8','X-UI-Fixture':'synthetic-read-only-no-model-validation','Cache-Control':'no-store'}); res.end(JSON.stringify(value)); } }; if (wait) { const id = setTimeout(() => { delays.delete(id); send(); }, wait); delays.add(id); } else send(); }
function result(action, text) { return {fixture:true, request_id:`fixture-request-${report.requests.length}`, action_id:action, source:'saved', revision, state:'ok', stage:action, elapsed_ms:123, first_token_ms:21, first_pcm_ms:34, engine:'FIXTURE ONLY — no model executed', text, measurement:'fixture_synthetic', context_mode:action.startsWith('pipeline.')?'pipeline_preview':'isolated', stages:[]}; }
async function startFixture() {
  const index = await readFile(path.join(built, 'index.html'), 'utf8');
  report.artifacts.entryScripts = [...index.matchAll(/<script[^>]*src="([^"]+)"/g)].map(m => m[1]);
  for (const name of ['index.html', 'nativevue2.js', 'h5App.js']) { const bytes = await readFile(path.join(built, name)); const info = await stat(path.join(built, name)); report.artifacts[name] = {bytes:bytes.length, sha256:createHash('sha256').update(bytes).digest('hex'), modifiedAt:info.mtime.toISOString()}; }
  server = http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url, 'http://127.0.0.1');
      const chunks = []; for await (const chunk of req) chunks.push(chunk);
      const raw = Buffer.concat(chunks).toString(); let body; try { body = raw ? JSON.parse(raw) : undefined; } catch { body = raw; }
      const record = {method:req.method, path:url.pathname, query:url.search, authorization:req.headers.authorization || '', body, at:Date.now()}; report.requests.push(record);
      if (url.pathname.startsWith('/api/')) {
        if (req.method === 'POST' && url.pathname === '/api/config') return json(res, {fixture:true,ok:false,message:'FIXTURE HTTP409: configuration changed; keep local draft',revision:'fixture-newer'},409,600);
        if (req.method === 'POST' && url.pathname === '/api/test/run') {
          const planIndex = plans.findIndex(p => !p.action || p.action === body?.action_id);
          const plan = planIndex >= 0 ? plans.splice(planIndex,1)[0] : {};
          const value = result(body?.action_id, plan.text || 'FIXTURE_ASYNC_RESULT — synthetic response, not inference');
          record.fixturePlan = plan;
          return json(res, plan.status === 409 ? {...value,state:'error',error:'FIXTURE409_TEST_REVISION',error_code:'revision_conflict'} : value, plan.status || 200, plan.delay || 1200);
        }
        if (req.method !== 'GET') return json(res,{fixture:true,error:'Fixture refuses non-allowlisted mutation'},405);
        const fixtures = {
          '/api/config': {fixture:true,revision,server:{port:8000,worker_threads:2,admin_dir:'FIXTURE ONLY'},vad:{model:'FIXTURE-VAD-MODEL',threshold:0.5,min_silence_duration:0.25,min_speech_duration:0.25},asr:{provider:'cpu',model:'FIXTURE-ASR-MODEL',tokens:'FIXTURE-TOKENS',language:'auto'},tts:{engine:'kokoro'},llm:{enabled:true,model:'FIXTURE-LLM-MODEL',api_base:'https://fixture.invalid/v1',has_api_key:false},playback:{mode:'interactive',announcement_start_ms:5000,announcement_start_max_ms:10000}},
          '/api/config/meta': {fixture:true,config_path:'FIXTURE ONLY — no file persisted',persistent:false,hint:'Synthetic browser fixture'},
          '/api/config/schema': {fixture:true,app_sections:[],capabilities:[]},
          '/api/config/presets': {fixture:true,presets:[]},
          '/api/plugins': {fixture:true,capabilities:[]}, '/api/health/engines':{fixture:true,capabilities:[{capability:'vad',display:'FIXTURE VAD 健康修复入口',required:true,enabled:true,phase:'failed',active_impl:'fixture.vad',last_error:'FIXTURE ONLY — synthetic deviation; no model or filesystem probe performed',probe:{state:'fail',detail:'FIXTURE ONLY — synthetic read-only browser contract'},config_section:'vad'}]},
          '/api/mcp/tools':{fixture:true,devices:[]}, '/api/ota/firmware':{fixture:true,entries:[]},
          '/api/tts/voices':{fixture:true,voices:[]}, '/api/stats':{fixture:true,devices:{},sessions:{},turns:{}},
          '/api/devices':{fixture:true,devices:[]},
          '/api/echomuse/devices':{fixture:true,enabled:true,devices:[
            {device_id:'fixture:echo-approved',label:'FIXTURE-客厅音箱',approved:true,online:true,version:'1.2.3',board:'fixture-dot',base_os:'emos'},
            {device_id:'fixture:echo-pending',label:'',approved:false,online:false,version:'',board:'',base_os:''},
          ]},
          '/api/sessions':{fixture:true,sessions:[]}, '/api/conversations':{fixture:true,conversations:[]},
          '/api/usage':{fixture:true,global:{usage:{}},sessions:[]}, '/api/metrics':{fixture:true,sessions:{},turns:{},tts:{},errors:{}},
          '/api/soul/presets':{fixture:true,presets:[]}, '/api/memory/status':{fixture:true,enabled:false,scopes:[]},
          '/api/test/capabilities':{fixture:true,source:'saved',revision,actions:Object.entries(actionKinds).map(([action_id,input_kind])=>({action_id,input_kind,available:input_kind!=='native',readiness:input_kind==='native'?'unavailable':'configured',reason:input_kind==='native'?'FIXTURE no native adapter':'FIXTURE only; not real inference'})),samples:[{id:'fixture-sample-a',label:'FIXTURE 合成契约样本 A（无音频）',transcript:'仅用于 UI 契约，不提供音频',license:'Fixture only, no media'},{id:'fixture-sample-b',label:'FIXTURE 合成契约样本 B（无音频）',transcript:'第二个 UI 样本',license:'Fixture only, no media'}]},
        };
        if (url.pathname in fixtures) return json(res,fixtures[url.pathname]);
        return json(res,{fixture:true,error:'No fixture contract for this API'},404);
      }
      if (req.method !== 'GET') return json(res,{fixture:true,error:'Read only'},405);
      if (url.pathname === '/favicon.ico') { res.writeHead(204,{'X-UI-Fixture':'no-fixture-favicon'}); return res.end(); }
      const relative = decodeURIComponent(url.pathname === '/' ? 'index.html' : url.pathname.slice(1));
      const target = path.resolve(built,relative);
      if (!target.startsWith(built + path.sep)) return json(res,{error:'Outside built artifact'},403);
      try { const bytes = await readFile(target); const mime = {'.html':'text/html','.js':'text/javascript','.png':'image/png','.svg':'image/svg+xml','.json':'application/json','.css':'text/css','.ico':'image/x-icon'}[path.extname(target)] || 'application/octet-stream'; res.writeHead(200,{'Content-Type':mime,'Cache-Control':'no-store','X-UI-Fixture':'actual-published-static-asset'}); res.end(bytes); } catch { json(res,{fixture:true,error:'Missing built asset',path:url.pathname},404); }
    } catch (error) { json(res,{fixture:true,error:String(error)},500); }
  });
  server.on('upgrade',(req,socket)=>{ report.expectedFixtureWebsocketUnavailable.push({url:req.url,reason:'No fake voice/admin websocket server; fixture intentionally unavailable'}); socket.end('HTTP/1.1 503 Fixture websocket unavailable\r\nConnection: close\r\n\r\n'); });
  await new Promise(resolve => server.listen(0,'127.0.0.1',resolve));
  origin = `http://127.0.0.1:${server.address().port}`; report.fixtureOrigin = origin;
}
class CDP {
  constructor(ws) { this.ws=ws;this.seq=0;this.pending=new Map();this.listeners=new Map();ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){const p=this.pending.get(m.id);if(p){this.pending.delete(m.id);clearTimeout(p.timer);m.error?p.reject(new Error(JSON.stringify(m.error))):p.resolve(m.result);}} else for(const f of this.listeners.get(m.method)||[])f(m.params);}); }
  static async connect(url) { const ws=new WebSocket(url);await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});return new CDP(ws); }
  on(name,fn){const rows=this.listeners.get(name)||[];rows.push(fn);this.listeners.set(name,rows);}
  call(method,params={}) {return new Promise((resolve,reject)=>{const id=++this.seq;const timer=setTimeout(()=>{this.pending.delete(id);reject(new Error(`CDP timeout ${method}`));},20000);this.pending.set(id,{resolve,reject,timer});this.ws.send(JSON.stringify({id,method,params}));});}
  async eval(expression) {const r=await this.call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails));return r.result.value;}
  close(){for(const p of this.pending.values()){clearTimeout(p.timer);p.reject(new Error('CDP closed'));}this.pending.clear();this.ws.close();}
}
async function startChrome(){
  const profile=path.join(output,'chrome-profile');await mkdir(profile,{recursive:true});
  chrome=spawn(chromePath,['--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--remote-debugging-port=0',`--user-data-dir=${profile}`,'about:blank'],{stdio:['ignore','ignore','pipe']});
  let stderr='';
  const endpoint=await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(new Error('Chrome did not publish DevTools URL')),15000);chrome.once('error',e=>{clearTimeout(timer);reject(e);});chrome.stderr.on('data',chunk=>{stderr+=chunk;const match=stderr.match(/DevTools listening on (ws:\/\/[^\s]+)/);if(match){clearTimeout(timer);resolve(match[1]);}});chrome.once('exit',code=>{clearTimeout(timer);reject(new Error(`Own Chrome exited ${code}: ${stderr}`));});});
  const debug=new URL(endpoint);const targets=await (await fetch(`http://${debug.host}/json/list`)).json();const target=targets.find(t=>t.type==='page');
  cdp=await CDP.connect(target.webSocketDebuggerUrl);
  cdp.on('Runtime.exceptionThrown',p=>report.browserExceptions.push(p.exceptionDetails));
  cdp.on('Runtime.consoleAPICalled',p=>{if(p.type==='error')report.consoleErrors.push(p.args.map(a=>a.value||a.description).join(' '));});
  const requestUrls = new Map();
  cdp.on('Network.requestWillBeSent',p=>requestUrls.set(p.requestId,p.request.url));
  cdp.on('Network.responseReceived',p=>{if(p.response.status>=400)report.resourceFailures.push({url:p.response.url,status:p.response.status,type:p.type});});
  cdp.on('Network.loadingFailed',p=>report.resourceFailures.push({url:requestUrls.get(p.requestId)||p.requestId,error:p.errorText,type:p.type}));
  await Promise.all(['Page.enable','Runtime.enable','Network.enable'].map(method=>cdp.call(method)));
  await cdp.call('Network.setBlockedURLs',{urls:['http://127.0.0.1:55849/*','http://localhost:55849/*','http://127.0.0.1:58950/*','http://localhost:58950/*','https://*']});
  report.ownChromePid=chrome.pid;
  report.runtime={node:process.version,platform:process.platform,chrome:await cdp.call('Browser.getVersion')};
}
const bodyText=()=>cdp.eval('document.body.innerText');
async function viewport(width,height=1000){await cdp.call('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:false});await delay(200);}
async function navigate(page='config'){const url=`${origin}/?page_name=${page}&token=${token}&ui_fixture_navigation=${++navigationSequence}`;await cdp.call('Page.navigate',{url});await until(async()=>await cdp.eval(`document.URL===${JSON.stringify(url)}&&document.readyState==='complete'&&(document.querySelectorAll('input').length>1||document.body.innerText.includes('概览'))`),15000,'fresh Kuikly document rendered');await delay(300);}
async function screenshot(name){const shot=await cdp.call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});const filename=path.join(output,`${name}.png`);await writeFile(filename,Buffer.from(shot.data,'base64'));report.screenshots.push(filename);return filename;}
// Locate rendered leaf text, not private Kotlin state; input and clicks remain browser events.
function findTextCode(text,exact=true,order='top',interactive=false){return `(()=>{const text=${JSON.stringify(text)};const clickable=e=>{for(let p=e;p&&p!==document.body;p=p.parentElement)if(p.style.cursor==='pointer'||p.matches('button,[role="button"]'))return true;return false;};const candidates=[...document.querySelectorAll('#root *')].filter(e=>{const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&getComputedStyle(e).visibility==='visible'&&!e.closest('#measure_iframe')&&${interactive?'clickable(e)&&':''}${exact?'e.textContent===text':'e.textContent.includes(text)'}&&![...e.children].some(c=>${exact?'c.textContent===text':'c.textContent.includes(text)'});});return candidates.sort((a,b)=>${order==='right'?'b.getBoundingClientRect().right-a.getBoundingClientRect().right':order==='left'?'a.getBoundingClientRect().left-b.getBoundingClientRect().left':'a.getBoundingClientRect().top-b.getBoundingClientRect().top'})[0];})()`;}
async function clickPoint(found,description){if(!found)throw new Error(`Rendered target not found: ${description}`);const width=await cdp.eval('innerWidth'),height=await cdp.eval('innerHeight');if(found.x<0||found.x>width||found.y<0||found.y>height)throw new Error(`Target outside viewport: ${description} ${JSON.stringify(found)}`);await cdp.call('Input.dispatchMouseEvent',{type:'mousePressed',x:found.x,y:found.y,button:'left',clickCount:1});await cdp.call('Input.dispatchMouseEvent',{type:'mouseReleased',x:found.x,y:found.y,button:'left',clickCount:1});await delay(150);return found;}
async function clickText(text,{exact=true,scroll=true,order='top'}={}){const found=await cdp.eval(`(()=>{const e=${findTextCode(text,exact,order,true)};if(!e)return null;${scroll?'e.scrollIntoView({block:"nearest",inline:"nearest"});':''}const r=e.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2,w:r.width,h:r.height};})()`);return clickPoint(found,text);}
// Resolve targets from actual DOM boxes. Never infer a dot from a hard-coded pitch or viewport.
const workflowCode = `(()=>{const names=${JSON.stringify(stages)};const leaf=name=>[...document.querySelectorAll('#root *')].find(e=>e.textContent===name&&![...e.children].some(c=>c.textContent===name));const text=names.map(leaf);if(text.some(e=>!e))return null;const clickable=e=>{let p=e;while(p&&p!==document.body){if(p.style.cursor==='pointer'||p.matches('button,[role="button"]'))return p;p=p.parentElement;}return e;};const core=text.map(clickable);let row=core[0];while(row&&!core.every(e=>row.contains(e)))row=row.parentElement;if(!row)return null;const labelled=${JSON.stringify(dots)}.map(name=>[...row.querySelectorAll('*')].find(e=>[e.getAttribute('aria-label'),e.getAttribute('accessibility'),e.getAttribute('accessibilityLabel')].some(value=>value===('流程连接点：'+name)||value?.startsWith('流程连接点：'+name+'，'))));const raw=[...row.querySelectorAll('*')].filter(e=>(e.style.cursor==='pointer'||e.matches('button,[role="button"]'))&&!core.some(c=>e===c||e.contains(c)||c.contains(e)));const points=labelled.every(Boolean)?labelled:raw.filter(e=>!raw.some(other=>other!==e&&other.contains(e))).sort((a,b)=>a.getBoundingClientRect().left-b.getBoundingClientRect().left);return {row,core,points,labels:labelled.every(Boolean),text};})()`;
async function workflowBounds(){return cdp.eval(`(()=>{const w=${workflowCode};if(!w)return null;const rect=e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom,text:e.textContent,aria:e.getAttribute('aria-label')};};let scroller=w.row;while(scroller&&!/auto|scroll/.test(getComputedStyle(scroller).overflowX))scroller=scroller.parentElement;const verticalClips=[];for(let p=w.row.parentElement;p&&p!==document.body;p=p.parentElement){const s=getComputedStyle(p);if(/hidden|auto|scroll/.test(s.overflowY)){const r=p.getBoundingClientRect();verticalClips.push({top:r.top+p.clientTop,bottom:r.top+p.clientTop+p.clientHeight,clientHeight:p.clientHeight,outerHeight:r.height,overflowY:s.overflowY});}}return {core:w.core.map((e,i)=>({...rect(e),text:w.text[i].textContent})),dots:w.points.map(rect),accessibleLabels:w.labels,row:rect(w.row),verticalClips,scroll:scroller?{clientWidth:scroller.clientWidth,clientHeight:scroller.clientHeight,scrollWidth:scroller.scrollWidth,scrollLeft:scroller.scrollLeft,bounds:rect(scroller),overflowX:getComputedStyle(scroller).overflowX}:null};})()`);}
async function selectPosition({stage,dot}){const found=await cdp.eval(`(()=>{const w=${workflowCode};if(!w||w.points.length!==6)return null;const e=${stage?`w.core[${stages.indexOf(stage)}]`:`w.points[${dot}]`};if(!e)return null;e.scrollIntoView({block:'nearest',inline:'nearest'});const r=e.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2,w:r.width,h:r.height};})()`);return clickPoint(found,stage||dots[dot]);}
async function workspaceTab(tab){await clickText(tab,{order:'left'});await delay(120);}
async function selectNode(label,{tab='测试'}={}){const node=nodes.find(n=>n.label===label);if(!node)throw new Error('Unknown fixture node '+label);await selectPosition(node);await workspaceTab('配置');if(!node.stage)await clickText(label,{order:'left'});if(tab==='测试'){await workspaceTab('测试');await clickText(node.action,{order:'left'});}await delay(150);}
const runAction=text=>clickText(text,{order:'right'});
async function fillInput(value,currentValue){return cdp.eval(`(()=>{const inputs=[...document.querySelectorAll('input,textarea')];const e=inputs.find(e=>e.value===${JSON.stringify(currentValue)});if(!e)return {ok:false,inputs:inputs.map(e=>({value:e.value,placeholder:e.placeholder}))};e.scrollIntoView({block:'center'});e.focus();const proto=e instanceof HTMLTextAreaElement?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;Object.getOwnPropertyDescriptor(proto,'value').set.call(e,${JSON.stringify(value)});e.dispatchEvent(new InputEvent('input',{bubbles:true,inputType:'insertText',data:${JSON.stringify(value)}}));e.dispatchEvent(new Event('change',{bubbles:true}));return {ok:true,value:e.value};})()`);}
async function layout(){return cdp.eval(`(()=>{const w=innerWidth;const items=[...document.querySelectorAll('#root *')].filter(e=>{const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&r.bottom>0&&r.top<innerHeight&&(getComputedStyle(e).cursor==='pointer'||e.matches('input,textarea,button'));});const rect=e=>{const r=e.getBoundingClientRect();return {tag:e.tagName,text:e.textContent.slice(0,90),x:r.x,y:r.y,width:r.width,height:r.height};};const clipped=items.filter(e=>{const r=e.getBoundingClientRect();if(r.left>=-1&&r.right<=w+1)return false;let p=e.parentElement;while(p){const s=getComputedStyle(p);if(/auto|scroll/.test(s.overflowX)&&p.scrollWidth>p.clientWidth+1)return false;p=p.parentElement;}return true;}).map(rect);return {width:w,bodyWidth:document.body.scrollWidth,documentWidth:document.documentElement.scrollWidth,clippedControls:clipped,visibleControls:items.length,text:document.body.innerText};})()`);}
async function dump(name){await writeFile(path.join(output,`${name}.json`),JSON.stringify(await cdp.eval(`({text:document.body.innerText,inputs:[...document.querySelectorAll('input,textarea')].map(e=>({value:e.value,placeholder:e.placeholder})),images:[...document.images].map(e=>({src:e.src,complete:e.complete,naturalWidth:e.naturalWidth})),html:document.body.innerHTML})`),null,2));}
async function iconCheck(){
  const canvases=await until(async()=>{const rows=await cdp.eval(`Array.from(document.querySelectorAll('#root canvas')).map(e=>{const r=e.getBoundingClientRect();return {width:Math.round(r.width),height:Math.round(r.height)};})`);return rows.length?rows:null;},12000,'embedded vector icon canvases');
  const images=await cdp.eval(`[...document.images].map(e=>e.src).filter(s=>s.includes('/icons/'))`);
  const painted=await cdp.eval(`(()=>{for(const e of document.querySelectorAll('#root canvas')){const w=e.width,h=e.height;if(!w||!h||w*h>262144)continue;try{const data=e.getContext('2d').getImageData(0,0,w,h).data;for(let i=3;i<data.length;i+=4)if(data[i]>0)return true;}catch{}}return false;})()`);
  check('Embedded vector icons render as painted Canvas without Image or bitmap',canvases.length>0&&painted&&images.length===0,{canvasCount:canvases.length,painted,bitmapIconImages:images});
  const svgRequests=report.requests.filter(r=>r.path.endsWith('.svg'));
  const business=await readFile(path.join(built,'nativevue2.js'),'utf8');
  check('Icons ship as embedded geometry with no icon resources or HTTP loads',svgRequests.length===0&&!business.includes('assets://common/icons'),{svgRequests,scope:'Admin icons are Canvas-drawn embedded pathData (AdminIcons.kt); no resource files on any host'});
}
// EchoMuse device cards: delete is the encapsulated icon-only appButton with the embedded vector
// Delete icon (never the legacy 🗑 emoji glyph); in-card menu items lead with vector icons too.
async function echoMuseIconCheck(){
  await viewport(1440);
  await clickText('EchoMuse',{order:'left'});
  await until(async()=>(await bodyText()).includes('FIXTURE-客厅音箱'),12000,'EchoMuse device cards rendered');
  await delay(200);
  const text=await bodyText();
  const canvases=await cdp.eval(`Array.from(document.querySelectorAll('#root canvas')).filter(e=>{const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&r.width<=48&&r.height<=48;}).length`);
  const images=await cdp.eval(`[...document.images].map(e=>e.src).filter(s=>s.includes('/icons/'))`);
  check('EchoMuse delete uses the encapsulated vector icon button, emoji glyph gone',!text.includes('🗑')&&canvases>=10&&images.length===0,{canvasCount:canvases,emojiDelete:text.includes('🗑'),bitmapIconImages:images});
  const menuIcons=await cdp.eval(`(()=>{const leafs=name=>[...document.querySelectorAll('#root *')].filter(e=>e.textContent===name&&![...e.children].some(c=>c.textContent===name));const wifi=leafs('WiFi 配置')[0];if(!wifi)return null;let column=wifi.parentElement;while(column&&!(column.textContent.includes('蓝牙代理')&&column.textContent.includes('唤醒词设置')))column=column.parentElement;if(!column)return null;const labels=['概览','WiFi 配置','唤醒词设置','设备管理','蓝牙代理'];return labels.map(name=>{const rows=leafs(name).filter(e=>column.contains(e));const row=rows[rows.length-1];if(!row)return {name,present:false};return {name,present:true,iconCanvas:[...row.parentElement.children].some(c=>c.tagName==='CANVAS')};});})()`);
  check('EchoMuse in-card menu items lead with a vector icon',!!menuIcons&&menuIcons.length===5&&menuIcons.every(m=>m.present&&m.iconCanvas),menuIcons);
  const clicked=await cdp.eval(`(()=>{const e=[...document.querySelectorAll('#root *')].find(e=>['aria-label','accessibility','accessibilityLabel'].some(a=>(e.getAttribute(a)||'').includes('删除该设备记录')));if(!e)return null;let t=e;while(t.parentElement&&t.style.cursor!=='pointer'&&!t.matches('button,[role="button"]'))t=t.parentElement;const r=t.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()`);
  await clickPoint(clicked,'删除该设备记录 icon-only button');
  const confirmText=await bodyText();
  check('EchoMuse icon-only delete opens the inline confirm row',confirmText.includes('确认删除')&&confirmText.includes('取消'),{confirmRow:confirmText.includes('确认删除')});
  await clickText('取消',{order:'right'});
  await screenshot('echomuse-icon-buttons');
}
async function actionBounds(label){return cdp.eval(`(()=>{const leaf=${findTextCode(label,true,'right')};if(!leaf)return null;let button=leaf;while(button.parentElement&&button.style.cursor!=='pointer'&&!button.matches('button,[role="button"]'))button=button.parentElement;const r=button.getBoundingClientRect();let row=button.parentElement;while(row&&row.getBoundingClientRect().width<=r.width+1)row=row.parentElement;const rect=e=>{const q=e.getBoundingClientRect();return {x:q.x,y:q.y,width:q.width,height:q.height,right:q.right,bottom:q.bottom};};return {button:rect(button),row:row?rect(row):null};})()`);}
async function workspaceBounds(menuLabel,contentLabel){return cdp.eval(`(()=>{const menu=${findTextCode(menuLabel,true,'left')},content=${findTextCode(contentLabel,true,'right')};if(!menu||!content)return null;const rect=e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom};};let sidebar=menu;const admin=new URL(location.href).searchParams.get('page_name')==='admin';const workspaceWidth=innerWidth-(admin&&innerWidth>=768?241:0);const expected=workspaceWidth<700?92:192;while(sidebar&&Math.abs(sidebar.getBoundingClientRect().width-expected)>1)sidebar=sidebar.parentElement;return {menu:rect(menu),content:rect(content),sidebar:sidebar?rect(sidebar):null,expectedSidebar:expected,workspaceWidth,admin,oldDropdown:document.body.innerText.includes('流程节点与扩展')};})()`);}
async function workflowCheck(context){const w=await workflowBounds();check(`${context}: five core stages and six actual DOM dot hit targets`,!!w&&w.core.length===5&&w.dots.length===6&&w.core.every((r,i)=>r.text===stages[i])&&w.dots.every(r=>r.width>=40&&r.height>=40),w);if(!w)return;const ordered=[w.dots[0],...w.core.flatMap((core,i)=>[core,w.dots[i+1]])];check(`${context}: workflow stays horizontal in canonical order`,ordered.every((r,i)=>Math.abs((r.y+r.height/2)-(ordered[0].y+ordered[0].height/2))<2&&(!i||r.x>=ordered[i-1].right-1)),w);const clipTop=Math.max(0,...w.verticalClips.map(c=>c.top)),clipBottom=Math.min(Infinity,...w.verticalClips.map(c=>c.bottom));check(`${context}: stage and dot targets fit actual ancestor scrollport height`,ordered.every(r=>r.y>=clipTop-1&&r.bottom<=clipBottom+1),{clipTop,clipBottom,...w});check(`${context}: global settings remains visible at top right`,await cdp.eval(`(()=>{const e=${findTextCode('全局设置')};if(!e)return false;const r=e.getBoundingClientRect();return r.x>innerWidth/2&&r.right<=innerWidth+1&&r.top<100;})()`),w);}
async function workflowScrollCheck(width){const before=await workflowBounds();if(!before?.scroll||before.scroll.scrollWidth<=before.scroll.clientWidth+1)return;const r=before.scroll.bounds;await cdp.call('Input.dispatchMouseEvent',{type:'mouseWheel',x:r.x+before.scroll.clientWidth/2,y:r.y+before.scroll.clientHeight/2,deltaX:120,deltaY:0});await delay(250);const after=await workflowBounds();check(`Workflow ${width}: native horizontal wheel scroll survives hidden indicator`,after.scroll.scrollLeft>before.scroll.scrollLeft+1,{before:before.scroll,after:after.scroll});}
// Desktop contract (>=1280): the track sizes to the visible content area (sidebar-aware), so the
// workflow scroller must have nothing to scroll; only narrower viewports keep horizontal scrolling.
async function workflowFitCheck(context,width){const w=await workflowBounds();const s=w?.scroll;check(`${context} ${width}: workflow adapts to content width without horizontal scroll`,!!s&&s.scrollWidth<=s.clientWidth+1&&s.scrollLeft<=1&&w.row.width<=s.clientWidth+1&&w.row.right<=s.bounds.x+s.clientWidth+1,{scroll:s,row:w.row,viewport:width});}
async function workflowNavigationCheck(width){const before=report.requests.filter(r=>r.path==='/api/test/run').length;await workspaceTab('配置');for(const stage of stages){await selectPosition({stage});const node=nodes.find(n=>n.stage===stage);check(`Workflow ${width}: ${stage} opens its matching node`,(await bodyText()).includes(node.label));}for(const dot of dots.keys()){await selectPosition({dot});const features=nodes.filter(n=>n.dot===dot);const text=await bodyText();check(`Workflow ${width}: dot ${dots[dot]} opens attached features or explicit empty state`,features.length?features.every(n=>text.includes(n.label)):text.includes('暂无扩展'),{features:features.map(n=>n.label),text});}check(`Workflow ${width}: navigation alone never executes a test`,report.requests.filter(r=>r.path==='/api/test/run').length===before);}
async function titleActionCheck(context){const evidence=await cdp.eval(`(()=>{const title=${findTextCode('节点管理')};if(!title)return null;const rect=r=>({x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom});const range=document.createRange();range.selectNodeContents(title);const glyph=rect(range.getBoundingClientRect());const buttons=['保存配置','✕ 保存失败','保存中…','重新加载'].flatMap(label=>{let e=[...document.querySelectorAll('#root *')].find(p=>p.textContent===label&&![...p.children].some(c=>c.textContent===label));if(!e)return [];while(e.parentElement&&e.style.cursor!=='pointer')e=e.parentElement;return [{label,...rect(e.getBoundingClientRect())}];});return {title:rect(title.getBoundingClientRect()),glyph,buttons};})()`);const overlap=(a,b)=>Math.min(a.right,b.right)-Math.max(a.x,b.x)>1&&Math.min(a.bottom,b.bottom)-Math.max(a.y,b.y)>1;check(`${context}: full title text and header actions do not overlap`,!!evidence&&evidence.glyph.width>0&&evidence.buttons.length>0&&evidence.buttons.every((b,i)=>!overlap(evidence.glyph,b)&&evidence.buttons.slice(i+1).every(other=>!overlap(b,other))),evidence);}
async function workspaceCheck(width,{admin=false}={}){await selectNode('ASR · 语音识别',{tab:'配置'});const tabs=await cdp.eval(`['配置','测试'].map(label=>{let e=[...document.querySelectorAll('#root *')].filter(p=>getComputedStyle(p).visibility==='visible'&&!p.closest('#measure_iframe')&&p.textContent===label&&![...p.children].some(c=>c.textContent===label)).find(p=>{for(let a=p;a&&a!==document.body;a=a.parentElement)if(a.style.cursor==='pointer')return true;return false;});while(e&&e.style.cursor!=='pointer')e=e.parentElement;if(!e)return null;const r=e.getBoundingClientRect();const clips=[];for(let p=e.parentElement;p&&p!==document.body;p=p.parentElement){if(/hidden|auto|scroll/.test(getComputedStyle(p).overflowY)){const q=p.getBoundingClientRect();clips.push({top:q.top+p.clientTop,bottom:q.top+p.clientTop+p.clientHeight,clientHeight:p.clientHeight,outerHeight:q.height});}}return {label,x:r.x,y:r.y,width:r.width,height:r.height,clips};})`);check(`Workspace tabs ${width}: exactly two 112×44 side-by-side targets`,tabs.length===2&&tabs.every(Boolean)&&tabs.every(t=>Math.abs(t.width-112)<=1&&Math.abs(t.height-44)<=1)&&Math.abs(tabs[0].y-tabs[1].y)<1&&tabs[1].x>=tabs[0].x+tabs[0].width-1,{tabs});check(`Workspace tabs ${width}: full 44px targets fit actual ancestor scrollports`,tabs.every(t=>t&&t.clips.every(c=>t.y>=c.top-1&&t.y+t.height<=c.bottom+1)),{tabs});const pill=await cdp.eval(`(()=>{return [...document.querySelectorAll('#root *')].filter(e=>{const s=getComputedStyle(e);const r=e.getBoundingClientRect();return r.width>0&&r.height>0&&!e.closest('#measure_iframe')&&s.backgroundColor==='rgb(7, 193, 96)'&&Math.abs(r.height-3)<=1&&Math.abs(r.width-44)<=1;}).map(e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};});})()`);check(`Workspace tabs ${width}: official indicator pill slides under the selected label`,pill.length===1&&pill[0].y>tabs[0].y+20&&pill[0].y+pill[0].height<=tabs[0].y+tabs[0].height+1&&pill[0].x>=tabs[0].x&&pill[0].x+pill[0].width<=tabs[0].x+tabs[0].width+1,{pill,selectedTab:tabs[0]});await titleActionCheck(`${admin?'Admin':'Config'} ${width}`);let bounds=await workspaceBounds('模型与参数','ASR · 语音识别');
  check(`Configuration tab ${width}: persistent left sidebar and right content`,!!bounds?.sidebar&&bounds.content.x>=bounds.sidebar.right-1&&!bounds.oldDropdown,bounds);const save=await actionBounds('保存配置');check(`Primary save action ${width}: 48px high and within right edge`,!!save&&Math.abs(save.button.height-48)<=1&&save.button.right<=width+1&&save.button.x>=width/2,save);
  await clickText('运行状态',{order:'left'});check(`Configuration runtime submenu ${width} is reachable`,(await bodyText()).includes('ASR · 语音识别 · 运行状态')&&!(await cdp.eval(`[...document.querySelectorAll('input')].some(e=>e.value==='FIXTURE-ASR-MODEL')`)));
  await selectNode('ASR · 语音识别');bounds=await workspaceBounds('样本识别','刷新测试能力');check(`Test tab ${width}: persistent left menu and selected action on right`,!!bounds?.sidebar&&bounds.content.x>=bounds.sidebar.right-1&&!bounds.oldDropdown,bounds);
  const action=await actionBounds('样本识别');check(`Test execution action ${width}: nonzero button right aligned inside content`,!!action?.row&&action.button.width>=60&&Math.abs(action.button.height-44)<=1&&Math.abs(action.row.right-action.button.right)<=2,action);
  const testLayout=await layout();check(`Test workspace ${width}: no document overflow or cropped controls`,testLayout.bodyWidth<=width+1&&testLayout.documentWidth<=width+1&&testLayout.clippedControls.length===0,testLayout);await screenshot(`${admin?'admin-':''}workspace-test-${width}`);
}
async function runChecks(){
  await viewport(1440);await navigate();await dump('initial-dom');await screenshot('initial-1440');
  await iconCheck();
  if(probe){report.initialLayout=await layout();for(const width of [320,375,768,900]){await viewport(width);await navigate();const evidence=await layout();check(`Initial probe layout ${width}`,evidence.clippedControls.length===0&&evidence.documentWidth<=width+1,evidence);await workflowCheck(`Initial probe ${width}`);await screenshot(`initial-${width}`);}await viewport(375);await navigate();await selectNode('ASR · 语音识别');await screenshot('initial-narrow-asr');await dump('initial-narrow-dom');return;}
  await workspaceTab('测试');report.sharedPanelPresent=(await bodyText()).includes('节点测试 · 已保存配置');
  check('Fresh artifact includes shared HTTP NodeTestPanel',report.sharedPanelPresent,{artifact:report.artifacts['nativevue2.js']});
  if(!report.sharedPanelPresent)throw new Error('Shared panel absent: await Lead fresh published artifact, not a completed UI pass');
  for(const width of [320,375,768,900,1280,1440]){await viewport(width);await navigate();const evidence=await layout();check(`Layout ${width}: no document overflow or cropped visible controls`,evidence.bodyWidth<=width+1&&evidence.documentWidth<=width+1&&evidence.clippedControls.length===0,evidence);check(`Web native hardware hidden at ${width}`,!['本机录音识别','本机播放诊断'].some(t=>evidence.text.includes(t)),{width});await workflowCheck(`Config ${width}`);if(width>=1280)await workflowFitCheck('Config',width);await screenshot(`config-${width}`);await workflowScrollCheck(width);await workflowNavigationCheck(width);await workspaceCheck(width);}
  await viewport(1440);await navigate();await viewport(375);const resized=await layout();check('Wide→narrow resize keeps workflow and persistent sidebar without dropdown',!resized.text.includes('流程节点与扩展')&&resized.clippedControls.length===0,resized);await workflowCheck('Wide→narrow resize');await workspaceCheck(375);await screenshot('wide-to-narrow-375');
  await viewport(1440);await navigate();
  for(const node of nodes){await selectNode(node.label);const text=await bodyText();const nativeHidden=!text.includes('本机录音识别')&&!text.includes('本机播放诊断');check(`Flow node accessible: ${node.label}`,text.includes(node.action)&&text.includes('节点测试 · 已保存配置')&&nativeHidden,{label:node.label,expectedAction:node.action,nativeHidden});}
  for(const dot of [1,4]){const before=report.requests.filter(r=>r.path==='/api/test/run').length;await selectPosition({dot});await workspaceTab('配置');const config=await bodyText();await workspaceTab('测试');const test=await bodyText();check(`Empty attachment point ${dots[dot]} is explicit and never runs a request`,config.includes('暂无扩展')&&test.includes('暂无扩展')&&report.requests.filter(r=>r.path==='/api/test/run').length===before,{config,test});}
  await selectNode('终端与音频入口');await clickText('文本流程验证',{order:'left'});const inputTab=await cdp.eval(`(()=>{const w=${findTextCode('刷新测试能力',true,'right')};if(!w)return null;let p=w;while(p.parentElement&&!p.textContent.includes('节点测试 · 已保存配置'))p=p.parentElement;return p.textContent;})()`);check('Test submenu renders only the selected action in right panel',!!inputTab&&inputTab.includes('文本流程验证')&&!inputTab.includes('输入能力验证')&&!inputTab.includes('音频流程验证'),{text:inputTab});
  await selectNode('LLM · 对话生成');await until(async()=>(await bodyText()).includes('节点调用测试'));
  check('LLM Web hardware actions hidden',!(await bodyText()).includes('本机播放诊断'));
  const inputValue='FIXTURE_INPUT_DRAFT_NOT_CONFIG';check('Test text browser input accepted',(await fillInput(inputValue,'你好，请简短介绍一下你自己。')).ok);await delay(150);
  const postBefore=report.requests.filter(r=>r.method==='POST'&&r.path==='/api/config').length;
  const runBefore=report.requests.filter(r=>r.path==='/api/test/run').length;
  plans.push({action:'llm.chat',delay:2200,text:'FIXTURE_ASYNC_UPDATED_TEXT'});
  await runAction('节点调用测试');await until(async()=>(await bodyText()).includes('测试中…'));
  const animationA=await cdp.eval(`({animations:document.getAnimations().map(a=>({state:a.playState,time:a.currentTime,frames:a.effect?.getKeyframes()})),transforms:[...document.querySelectorAll('canvas')].map(e=>getComputedStyle(e).transform)})`);await delay(230);const animationB=await cdp.eval(`({animations:document.getAnimations().map(a=>({state:a.playState,time:a.currentTime})),transforms:[...document.querySelectorAll('canvas')].map(e=>getComputedStyle(e).transform)})`);
  check('Loading indicator really rotates',animationA.animations.some((a,i)=>a.state==='running'&&animationB.animations[i]?.time!==a.time)||JSON.stringify(animationA.transforms)!==JSON.stringify(animationB.transforms),{before:animationA,after:animationB});
  await clickText('测试中…');await clickText('测试中…');await screenshot('loading-llm');
  check('Loading suppresses duplicate clicks',report.requests.filter(r=>r.path==='/api/test/run').length===runBefore+1,{requests:report.requests.filter(r=>r.path==='/api/test/run')});
  await until(async()=>(await bodyText()).includes('FIXTURE_ASYNC_UPDATED_TEXT'));check('Async result text updates reactive DOM',true,{text:await bodyText()});await screenshot('async-result');
  check('Shared test never POSTs configuration',report.requests.filter(r=>r.method==='POST'&&r.path==='/api/config').length===postBefore);
  const run=report.requests.find(r=>r.path==='/api/test/run');check('Shared test sends fixture-only token header and saved revision',run?.authorization===`Bearer ${token}`&&run?.body?.expected_revision===revision&&run?.body?.action_id==='llm.chat'&&run?.body?.text===inputValue,{request:run});check('Fixture-only URL token not rendered in body',!(await bodyText()).includes(token));
  await clickText('刷新测试能力');await until(async()=>(await bodyText()).includes('节点调用测试'));
  plans.push({action:'llm.chat',delay:1600,text:'FIXTURE_STALE_OLD_NODE_RESPONSE'});await runAction('节点调用测试');await until(async()=>(await bodyText()).includes('测试中…'));await selectNode('ASR · 语音识别');await delay(1800);check('Delayed old-node response cannot overwrite current ASR node',!(await bodyText()).includes('FIXTURE_STALE_OLD_NODE_RESPONSE')&&(await bodyText()).includes('样本识别'),{text:await bodyText()});await selectNode('LLM · 对话生成');check('Returning to old node does not restore stale result',!(await bodyText()).includes('FIXTURE_STALE_OLD_NODE_RESPONSE'));
  plans.push({action:'llm.chat',status:409,delay:500});await runAction('节点调用测试');await until(async()=>(await bodyText()).includes('配置快照已变化，请刷新测试能力'));check('Test HTTP409 keeps test input',(await cdp.eval(`[...document.querySelectorAll('input')].some(e=>e.value===${JSON.stringify(inputValue)})`)),{text:await bodyText()});
  await selectNode('VAD · 语音分段',{tab:'配置'});const draft='FIXTURE_LOCAL_UNSAVED_VAD_MODEL';const edit=await fillInput(draft,'FIXTURE-VAD-MODEL');check('Production config input accepts local draft',edit.ok,edit);await delay(150);await clickText('保存配置');await until(async()=>(await bodyText()).includes('重新加载'));check('Config HTTP409 preserves input/draft and offers reload',await cdp.eval(`[...document.querySelectorAll('input')].some(e=>e.value===${JSON.stringify(draft)})`)&&report.requests.filter(r=>r.method==='POST'&&r.path==='/api/config').at(-1)?.body?.vad?.model===draft,{text:await bodyText(),post:report.requests.filter(r=>r.method==='POST'&&r.path==='/api/config').at(-1)});await screenshot('config-409-draft-preserved');
  await viewport(320);await titleActionCheck('Config HTTP409 at 320');const conflictLayout=await layout();check('Conflict state remains within narrow viewport after resize',conflictLayout.clippedControls.length===0&&conflictLayout.documentWidth<=321,conflictLayout);await screenshot('config-409-narrow-320');
  await viewport(1440);await navigate('admin');await screenshot('admin-1440');const admin=await bodyText();check('Admin Web navigation hides native testbench',admin.includes('EchoMuse')&&admin.includes('对话记录')&&!admin.includes('测试台'),{text:admin});await workflowCheck('Admin config content');await workflowFitCheck('Admin config content',1440);await clickText('概览',{order:'left'});check('Admin operations overview renders no workflow outside the config area',!(await bodyText()).includes('语音工作流')&&(await workflowBounds())===null,{text:await bodyText()});await clickText('语音流程与配置',{order:'left'});await delay(300);await selectPosition({stage:'ASR'});check('Config-area workflow opens the matching node workspace from operations',(await bodyText()).includes('ASR · 语音识别')&&(await bodyText()).includes('模型与参数'));await workspaceTab('测试');check('Admin config workflow shares the HTTP node-test workspace',(await bodyText()).includes('节点测试 · 已保存配置'));await screenshot('admin-workflow-asr');const repairRuns=report.requests.filter(r=>r.path==='/api/test/run').length;await clickText('概览',{order:'left'});await until(async()=>(await bodyText()).includes('FIXTURE VAD 健康修复入口'));await clickText('去修复 ›');const repaired=await bodyText();check('Health repair from Testing through operations forces Configuration parameters',repaired.includes('模型与参数')&&!repaired.includes('节点测试 · 已保存配置')&&await cdp.eval(`[...document.querySelectorAll('input')].some(e=>e.value==='FIXTURE-VAD-MODEL')`),{text:repaired});check('Health repair navigation never executes a node test',report.requests.filter(r=>r.path==='/api/test/run').length===repairRuns);await screenshot('admin-health-repair');
  await echoMuseIconCheck();
  for(const width of [800,900,1280,1440]){await viewport(width);await navigate('admin');await workflowCheck(`Admin ${width}`);if(width>=1280)await workflowFitCheck('Admin',width);await workspaceCheck(width,{admin:true});const evidence=await layout();check(`Admin ${width}: compact workspace remains inside the shell`,evidence.clippedControls.length===0&&evidence.documentWidth<=width+1,evidence);await screenshot(`admin-workspace-${width}`);}
  await viewport(375);await navigate('admin');await screenshot('admin-375');const adminLayout=await layout();check('Admin compact shell has no cropped visible controls',adminLayout.clippedControls.length===0&&adminLayout.documentWidth<=376,adminLayout);await workflowCheck('Admin compact config shell');
  await dump('final-dom');
}
try {await mkdir(output,{recursive:true});await startFixture();await startChrome();await runChecks();}
catch(error){report.fatal=error.stack||String(error);console.error(report.fatal);if(cdp){try{await dump('failure-dom');await screenshot('failure');}catch{}}}
finally{
  if(cdp){try{cdp.close();}catch{}}
  if(chrome&&chrome.exitCode===null&&chrome.signalCode===null){chrome.kill('SIGTERM');await Promise.race([new Promise(resolve=>chrome.once('exit',resolve)),delay(3000)]);if(chrome.exitCode===null&&chrome.signalCode===null){chrome.kill('SIGKILL');await new Promise(resolve=>chrome.once('exit',resolve));}}
  for(const timer of delays)clearTimeout(timer);
  if(server){server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
  report.finishedAt=new Date().toISOString();report.processCleanup={ownChromeExited:!chrome||chrome.exitCode!==null||chrome.signalCode!==null,fixtureServerClosed:!server||!server.listening};
  report.expectedFixtureHttpConflicts = report.resourceFailures.filter(r=>r.status===409 && (r.url===origin+'/api/config'||r.url===origin+'/api/test/run'));
  const unexpectedResources=report.resourceFailures.filter(r=>!String(r.url).includes('/api/admin/ws')&&!String(r.url).includes('favicon.ico')&&!report.expectedFixtureHttpConflicts.includes(r));
  check('No uncaught browser exceptions',report.browserExceptions.length===0,{exceptions:report.browserExceptions});
  check('No unexpected resource failures',unexpectedResources.length===0,{failures:unexpectedResources,expectedFixtureWebsocketUnavailable:report.expectedFixtureWebsocketUnavailable});
  report.passed=!report.fatal&&report.checks.every(c=>c.pass);await writeFile(path.join(output,'report.json'),JSON.stringify(report,null,2));console.log(`REPORT ${path.join(output,'report.json')}`);console.log(`RESULT ${report.passed?'PASS':'FAIL'} (${probe?'initial probe, not final assertions':'full fresh-artifact smoke'})`);if(!report.passed)process.exitCode=1;
}
