'use strict';
// Real Kuikly bundles and pointer gestures; only device HTTP/WS traffic is mocked.
const {chromium}=require(process.env.PLAYWRIGHT_PATH||'playwright');
const assert=require('node:assert/strict');
const path=require('node:path');
const web=path.resolve(__dirname,'../../web');
(async()=>{
 const browser=await chromium.launch({headless:true,...(process.env.CHROME_PATH?{executablePath:process.env.CHROME_PATH}:{})});
 const page=await browser.newPage({viewport:{width:1440,height:1000}});
 const errors=[],writes=[];
 const device={device_id:'volume-dot2',label:'音量测试音箱',approved:true,online:true,version:'2.18.1',board:'dot2',base_os:'emos',session_id:'volume-session',settings:{},capabilities:['mic','speaker'],ip:'192.0.2.25'};
 let reported=60,runtimeReads=0,holdNext=false,held=null;
 page.on('pageerror',error=>errors.push(error.message));
 const sockets=[];await page.routeWebSocket('**/*',socket=>{sockets.push(socket)});
 await page.route('http://native-volume.test/**',async route=>{
  const request=route.request(),url=new URL(request.url());
  if(!url.pathname.startsWith('/api/')){
   try{return await route.fulfill({path:path.join(web,url.pathname.slice(1)||'index.html')});}
   catch{return route.fulfill({status:404,body:'missing'});}
  }
  let body={};
  if(url.pathname==='/api/echomuse/devices')body={enabled:true,devices:[device]};
  else if(url.pathname==='/api/files')body={path:'oww-models',entries:[]};
  else if(url.pathname==='/api/config')body={config:{},revision:1};
  else if(url.pathname.endsWith('/runtime')){
   runtimeReads++;
   body={device_id:device.device_id,session_id:device.session_id,online:device.online,volume_state:reported==null?null:{level:reported},listen_state:{state:'local'},stats:{cpuPct:12,memUsedMb:100,memTotalMb:512}};
   if(holdNext){holdNext=false;await new Promise(resolve=>{held={release:resolve,body};});held=null;}
  }else if(url.pathname.endsWith('/volume')){writes.push(request.postDataJSON());body={queued:true};}
  else if(url.pathname.endsWith('/oww/state'))body={device_id:device.device_id};
  return route.fulfill({status:200,json:body});
 });
 const wait=async (fn,label)=>{for(let i=0;i<120;i++){if(await fn())return;await page.waitForTimeout(80);}throw Error('timed out: '+label);};
 const click=async text=>{
  const name=({'关闭':'关闭设备详情','刷新':'刷新设备列表'})[text]||text;
   const role=page.getByRole('button',{name,exact:true});
   const node=text==='音量测试音箱'?page.getByLabel(/^音量测试音箱,/):name!==text?page.getByLabel(name,{exact:true}):await role.count()?role.last():page.getByText(text,{exact:true}).last();await node.scrollIntoViewIfNeeded();await page.waitForTimeout(200);
  await wait(()=>node.evaluate(n=>{const r=n.getBoundingClientRect(),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);return hit===n||n.contains(hit);}),text+' unblocked');
  await node.click();await page.waitForTimeout(150);
 };
 const slider=()=>page.getByLabel(/^设备音量(?:,|$)/);
 const value=async text=>{await page.getByText(text,{exact:true}).first().waitFor();};
 const bounds=async()=>{const r=await slider().boundingBox();assert.ok(r&&r.width>100,'volume slider has a real hit area');return r;};
 const startDrag=async (from,to)=>{const r=await bounds();await page.mouse.move(r.x+r.width*from,r.y+r.height/2);await page.mouse.down();await page.mouse.move(r.x+r.width*to+(to===1?40:to===0?-40:0),r.y+r.height/2,{steps:12});await page.waitForTimeout(120);};
 const activeWidth=()=>slider().evaluate(root=>Math.min(...[...root.querySelectorAll('div')].map(n=>n.getBoundingClientRect()).filter(r=>r.width>=10&&r.width<=48&&r.height>=40&&r.height<=50).map(r=>r.x)));
 const open=async()=>{await click('音量测试音箱');await slider().waitFor();await page.waitForTimeout(250);};
 try{
  await page.goto('http://native-volume.test/?page_name=admin');await page.getByText('设备管理',{exact:true}).first().click({force:true});await open();await value('47%');
  assert.equal(await page.getByText('下发音量',{exact:true}).count(),0,'no numeric submit button');
  reported=95;await value('75%');assert.ok(runtimeReads>=2,'device changes are polled');
  await startDrag(.75,.85);const during=await activeWidth();assert.ok(Number.isFinite(during));
  reported=10;const reads=runtimeReads;await wait(()=>runtimeReads>reads,'report arrives during drag');await page.waitForTimeout(150);
  assert.equal(writes.length,0,'dragging never submits');
  assert.ok(Math.abs(await activeWidth()-during)<2,'a device report cannot overwrite the dragging thumb');
  await page.mouse.up();await wait(()=>writes.length===1,'single drag-end submit');
  assert.ok(Number.isInteger(writes[0].level)&&writes[0].level>=0&&writes[0].level<=127);
  await value('8%');assert.equal(writes.length,1,'queued target is not fabricated as device state');
  reported=44;await value('35%');holdNext=true;await wait(()=>held!==null,'hold a pre-submit runtime response');
  await startDrag(.35,.65);reported=100;await page.mouse.up();await wait(()=>writes.length===2,'second drag-end submit');
  await value('79%');held.release();await page.waitForTimeout(300);await value('79%');
  assert.equal(await page.getByText('35%',{exact:true}).count(),0,'pre-submit runtime response is discarded');
  await startDrag(.79,0);await page.mouse.up();await wait(()=>writes.length===3,'minimum volume');assert.equal(writes[2].level,0);
  await page.waitForTimeout(200);await startDrag(.79,1);await page.mouse.up();await wait(()=>writes.length===4,'maximum volume');assert.equal(writes[3].level,127);
  await click('活动');const beforeLeave=runtimeReads;await page.waitForTimeout(2400);
  assert.equal(runtimeReads,beforeLeave,'leaving status stops its runtime polling');
  await click('状态');await value('79%');await click('关闭');
  device.online=false;await click('刷新');await open();await value('79%');
  const offlineWrites=writes.length;await startDrag(.79,.3);await page.mouse.up();await page.waitForTimeout(250);assert.equal(writes.length,offlineWrites,'offline slider cannot submit');
  await click('关闭');device.online=true;device.capabilities=['mic'];await click('刷新');await open();
  await startDrag(.79,.4);await page.mouse.up();await page.waitForTimeout(250);assert.equal(writes.length,offlineWrites,'speaker capability is required');
  await click('关闭');device.capabilities=['speaker'];reported=128;await click('刷新');await open();await value('未上报');
  await startDrag(.2,.5);await page.mouse.up();await page.waitForTimeout(250);assert.equal(writes.length,offlineWrites,'invalid device volume is unknown, not a controllable zero');
  await click('关闭');device.approved=false;await click('刷新');await click('音量测试音箱');await page.getByText('新设备 · 等待批准',{exact:true}).waitFor();
  assert.equal(await slider().count(),0,'unapproved devices have no volume control');
  assert.deepEqual(errors,[]);console.log(JSON.stringify({result:'PASS',checks:'device-report-sync/drag-isolation/end-only-submit/zero-max/stale-runtime/tab-poll-dispose/offline-speaker-approved-invalid-gating',writes,runtimeReads,errors}));
 }catch(error){await page.screenshot({path:path.resolve(web,'../../../build/echomuse-native-volume-failure.png')});console.error({writes,runtimeReads,errors,text:await page.locator('body').innerText()});throw error;}
 finally{held?.release();await Promise.all(sockets.map(async socket=>{try{await socket.close()}catch{}}));await page.close();await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
