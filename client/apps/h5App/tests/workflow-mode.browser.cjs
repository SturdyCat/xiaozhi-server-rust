'use strict';
// Actual published Kuikly bundles. API interception proves UI state/patch semantics, not inference.
const {chromium}=require(process.env.PLAYWRIGHT_PATH||'playwright');
const assert=require('node:assert/strict');
const fs=require('node:fs/promises');
const path=require('node:path');
const web=path.resolve(__dirname,'../web');
const out=path.resolve(__dirname,'../../../build/workflow-browser');
(async()=>{
 const browser=await chromium.launch({headless:true,...(process.env.CHROME_PATH?{executablePath:process.env.CHROME_PATH}:{})});
 const page=await browser.newPage({viewport:{width:1440,height:1000}});
 const errors=[],writes=[];
 let revision='r1',conflict=false;
 let config={revision,asr:{model:'/models/asr-base.onnx',tokens:'/models/tokens.txt',language:'auto'},vad:{model:'/models/vad.onnx',threshold:0.5},workflow:null};
 const binding=(id,plugin_id,order)=>({id,plugin_id,position_id:'link.nlu.llm',enabled:true,order,condition:{kind:'always'},on_error:'continue'});
 const defaultBindings=[binding('soul','soul.profile',0),binding('recall','memory.recall',10)];
 defaultBindings[0].condition={kind:'confidence',min:0.2}; // UI fixture only; no inference claim.
 const descriptor=(id,label,section)=>({id,label,owner_position:'link.nlu.llm',allowed_positions:['link.nlu.llm'],config_paths:[section],repeatable:false,on_error_options:['continue','stop'],condition_kinds:['always','device_kind','confidence']});
 const metadata=()=>({version:1,revision,mode:'local',hot:'next_session',plugins:[descriptor('soul.profile','人格档案','soul'),descriptor('memory.recall','记忆召回','memory')],condition_kinds:[{kind:'confidence',label:'置信度区间',fields:[{key:'min',label:'最低置信度',type:'number',required:true}]},{kind:'always',label:'始终',fields:[]},{kind:'device_kind',label:'设备类型',fields:[{key:'value',label:'设备类型值',type:'string',required:true,options:['xiaozhi_esp','echomuse']}]}],config:config.workflow||{version:1,bindings:defaultBindings}});
 page.on('pageerror',e=>errors.push(e.message));
 await page.routeWebSocket('**/*',()=>{});
 await page.route('http://workflow.test/**',async route=>{
  const request=route.request(),u=new URL(request.url());
  if(u.pathname.startsWith('/api/')){
   let body={},status=200;
   if(u.pathname==='/api/config'&&request.method()==='POST'){
    const patch=request.postDataJSON();writes.push(patch);
    if(conflict){status=409;body={message:'配置已被其他窗口修改，请重新加载后再保存'};}
    else{assert.equal(patch.expected_revision,revision);revision='r'+(Number(revision.slice(1))+1);config={...config,...patch,revision};body={ok:true,revision,hot:'next_session',message:'已持久化'};}
   }else if(u.pathname==='/api/config')body={...config,revision};
   else if(u.pathname==='/api/workflow')body=metadata();
   else if(u.pathname==='/api/config/schema')body={advanced_fields:['asr.model','asr.tokens'],capabilities:[{common_fields:[{fields_path:['asr'],fields:[{key:'model',advanced:true,hot:'next_session'},{key:'tokens',advanced:true,hot:'next_session'}]}]}]};
   else if(u.pathname==='/api/plugins')body={capabilities:[]};
   else if(u.pathname==='/api/test/capabilities')body={revision,tests:[],samples:[]};
   else if(u.pathname==='/api/files')body={path:'',entries:[]};
   return route.fulfill({status,json:body});
  }
  const resolved=path.resolve(web,'.'+u.pathname);
  if(!resolved.startsWith(web+path.sep)&&resolved!==web)return route.fulfill({status:403,body:'blocked'});
  try{await route.fulfill({path:u.pathname==='/'?path.join(web,'index.html'):resolved});}catch{return route.fulfill({status:404,body:'missing'});}
 });
 const click=async text=>{const role=page.getByRole('button',{name:text,exact:true});const tab=page.getByLabel(text+', 未选择',{exact:true});const target=await role.count()?role.last():await tab.count()?tab.last():page.getByText(text,{exact:true}).last();await target.scrollIntoViewIfNeeded();await target.click();await page.waitForTimeout(250);};
 const axis=async label=>{const el=page.getByLabel('打开'+label+'节点',{exact:true});return await el.count()?el:page.getByText(label,{exact:true}).first();};
 const saved=async()=>{const toast=page.getByText('已保存，新会话生效。已持久化',{exact:true});await toast.waitFor();await toast.waitFor({state:'hidden'});};
 const fillValue=async(value,next)=>{const index=await page.locator('input').evaluateAll((els,v)=>els.findIndex(e=>e.value===v),value);assert.ok(index>=0,'input not found: '+value);await page.locator('input').nth(index).fill(next);await page.waitForTimeout(180);};
 try{
  await fs.mkdir(out,{recursive:true});
  await page.goto('http://workflow.test/?page_name=admin');
  await page.getByText('VAD · 语音分段',{exact:true}).first().waitFor();
  assert.equal(await page.getByText('VAD 模型',{exact:true}).count(),0,'simple default hides model paths');
  const boxes=[];for(const label of ['接入','VAD','ASR','NLU','LLM','TTS','结束'])boxes.push(await(await axis(label)).boundingBox());
  assert.ok(boxes.every(Boolean));
  assert.ok(Math.max(...boxes.map(b=>b.y))-Math.min(...boxes.map(b=>b.y))<4,'one horizontal axis');
  assert.ok(boxes.slice(1).every((b,i)=>b.x>boxes[i].x),'left-to-right workflow');
  await page.screenshot({path:path.join(out,'simple-wide.png')});
  await(await axis('ASR')).click();await page.waitForTimeout(250);
  assert.equal(await page.getByText('ASR 模型',{exact:true}).count(),0);
  await click('高级');
  await fillValue('/models/asr-base.onnx','/models/custom.onnx');
  await click('简洁');assert.equal(writes.length,0,'mode never submits');
  await click('保存配置');await page.getByText('保存包含高级配置的改动',{exact:true}).waitFor();
  await click('保留草稿');assert.equal(writes.length,0,'cancel preserves draft without submit');
  await click('高级');assert.ok(await page.locator('input').evaluateAll(els=>els.some(e=>e.value==='/models/custom.onnx')));
  await click('简洁');await click('保存配置');await click('确认保存');await saved();
  assert.equal(writes.length,1);assert.equal(writes[0].asr.model,'/models/custom.onnx');assert.equal(writes[0].expected_revision,'r1');
  assert.ok(!Object.hasOwn(writes[0],'workflow'),'unmodified workflow omitted');
  await page.getByLabel(/衔接插件/).nth(2).click();await page.waitForTimeout(250);
  await page.getByText('自称',{exact:true}).waitFor();
  assert.equal(await page.getByText('自我定位',{exact:true}).count(),0,'simple soul keeps only preset and identity');
  await click('高级');await page.getByText('自我定位',{exact:true}).waitFor();await click('管理');
  await page.getByText('人格档案',{exact:true}).last().waitFor();
  await fillValue('0.2','0.');
  await click('下移');
  assert.ok(await page.locator('input').evaluateAll(els=>els.some(e=>e.value==='0.')),'numeric intermediate input follows binding identity on reorder');
  await fillValue('0.','0.35');
  await click('简洁');await click('保存配置');await click('确认保存');await saved();
  assert.equal(writes.length,2);assert.equal(writes[1].expected_revision,'r2');
  assert.equal(writes[1].workflow.bindings.find(b=>b.id==='soul').condition.min,0.35,'numeric condition stays a JSON number');
  assert.ok(writes[1].workflow.bindings.find(b=>b.id==='soul').order>writes[1].workflow.bindings.find(b=>b.id==='recall').order);
  conflict=true;await click('高级');await click('下移');await click('保存配置');
  await page.getByText('重新加载',{exact:true}).first().waitFor();
  assert.equal(writes.length,3);assert.equal(writes[2].expected_revision,'r3');
  assert.ok((await page.locator('body').innerText()).includes('未保存：'),'409 retains workflow draft');
  await page.setViewportSize({width:640,height:850});await page.waitForTimeout(500);
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth+2),'narrow page does not overflow document');
  await page.screenshot({path:path.join(out,'advanced-narrow.png')});
  conflict=false;revision='r4';
  config.workflow.bindings=config.workflow.bindings.map(b=>b.id==='soul'?{...b,condition:{kind:'confidence',min:0.8}}:b);
  await click('重新加载');
  await page.waitForFunction(()=>Array.from(document.querySelectorAll('input')).some(e=>e.value==='0.8'));
  assert.ok(!await page.locator('input').evaluateAll(els=>els.some(e=>e.value==='0.35')),'reload reflects the authoritative numeric value');
  await page.reload();await page.getByText('VAD · 语音分段',{exact:true}).first().waitFor();
  assert.ok(await page.getByText('VAD 模型',{exact:true}).count()>0,'mode survives reload locally');
  assert.deepEqual(errors,[]);
  console.log('PASS workflow: simple default, single axis, local mode, hidden-draft confirmation, unique patch/revision, ordering, 409 retention, narrow layout');
 }catch(e){console.error('workflow test state',JSON.stringify({revision,conflict,writes,errors}));await fs.writeFile(path.join(out,'failure-body.txt'),await page.locator('body').innerText());await page.screenshot({path:path.join(out,'failure.png')});throw e;}
 finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
