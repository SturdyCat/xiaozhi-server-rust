'use strict';
// Exercise actual published Kuikly bundles; every device operation is intercepted.
const {chromium}=require(process.env.PLAYWRIGHT_PATH||'playwright');
const assert=require('node:assert/strict');
const path=require('node:path');
const web=path.resolve(__dirname,'../../web');
(async()=>{
 const browser=await chromium.launch({headless:true,...(process.env.CHROME_PATH?{executablePath:process.env.CHROME_PATH}:{})});
 const page=await browser.newPage({viewport:{width:1440,height:1000}});
 const errors=[],writes=[],commands=[];
 const device={device_id:'test-dot2',label:'测试音箱',approved:true,online:true,version:'2.18.1',board:'dot2',base_os:'emos',
  session_id:'native-session',settings:{sendspinName:'原名'},capabilities:['mic','speaker'],ip:'192.0.2.20'};
 const pending={...device,device_id:'pending-dot2',label:'待批准音箱',approved:false,settings:{}};
 let conflict=false,logsFail=false;
 page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(()=>{window.__nativeClicks=[];for(const type of ['pointerdown','pointerup','click'])document.addEventListener(type,e=>window.__nativeClicks.push({type,x:e.clientX,y:e.clientY,target:e.target.outerHTML.slice(0,200)}),true)});
 const sockets=[];await page.routeWebSocket('**/*',socket=>{sockets.push(socket)});
 await page.route('http://native-management.test/**',async route=>{
  const request=route.request(),u=new URL(request.url());
  if(u.pathname.startsWith('/api/')){
   let body={},status=200;
   const current=u.pathname.includes('/pending-dot2/')?pending:device;
   if(u.pathname==='/api/echomuse/devices') body={enabled:true,devices:[device,pending]};
   else if(u.pathname==='/api/files')body={path:'oww-models',entries:[]};
   else if(u.pathname==='/api/config')body={config:{},revision:1};
   else if(u.pathname.endsWith('/runtime'))body={device_id:current.device_id,session_id:current.session_id,online:true,
    volume_state:{level:60},mute_state:{muted:false},listen_state:{state:'local'},stats:{wifiSsid:'测试网络',cpuPct:13}};
   else if(u.pathname.endsWith('/oww/state'))body={device_id:device.device_id,vad:{state:'installed'},ort:{state:'missing'},free_mb:100};
   else if(u.pathname.endsWith('/approve')){assert.equal(u.pathname,'/api/echomuse/devices/pending-dot2/approve');pending.approved=true;pending.label=request.postDataJSON().label;body={approved:true};}
   else if(u.pathname.endsWith('/settings')){
    const delta=request.postDataJSON();writes.push(delta);
    if(conflict){status=409;body={error:'test configuration conflict'};}
    else{Object.assign(device.settings,delta);body={queued:true,persisted:true};}
   }else if(u.pathname.endsWith('/command')){commands.push(request.postDataJSON());status=202;body={queued:true};}
   else if(u.pathname.endsWith('/system/logs')){
    status=logsFail?502:200;body=logsFail?{error:'test log read failure'}:{output:'Firmware\nraw-device-log 原文\n',source:u.searchParams.get('source'),exit_code:0};
   }
   return route.fulfill({status,json:body});
  }
  try{await route.fulfill({path:path.join(web,u.pathname.slice(1)||'index.html')});}
  catch{await route.fulfill({status:404,body:'missing'});}
 });
 const wait=async fn=>{for(let i=0;i<100;i++){if(await fn())return;await page.waitForTimeout(80);}throw Error('condition timed out');};
 const click=async text=>{const role=page.getByRole('button',{name:text,exact:true});const el=await role.count()?role.last():['关闭设备详情','刷新设备列表','刷新资产状态','读取网络结果'].includes(text)?page.getByLabel(text,{exact:true}):page.getByText(text,{exact:true}).last();await el.scrollIntoViewIfNeeded();await page.waitForTimeout(150);const rect=await el.boundingBox();if(rect&&rect.y>680&&text!=='保存配置'){await page.mouse.move(700,500);await page.mouse.wheel(0,rect.y-500);}await page.waitForTimeout(350);await wait(()=>el.evaluate(node=>{const r=node.getBoundingClientRect(),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);return hit===node||node.contains(hit)}));await page.waitForTimeout(120);const target=await el.boundingBox();assert.ok(target,`visible control: ${text}`);await page.mouse.click(target.x+target.width/2,target.y+target.height/2);};
 const field=async label=>{
  const labelNode=page.getByText(label,{exact:true}).last();
  await labelNode.waitFor({state:'attached'});
  await labelNode.evaluate((node,label)=>{
   let parent=node;
   while(parent&&!parent.querySelector('input,textarea'))parent=parent.parentElement;
   const input=parent?.querySelector('input,textarea');
   if(!input)throw Error('missing field '+label);
   input.setAttribute('data-native-test-field',label);
  },label);
  return page.locator(`[data-native-test-field="${label}"]`);
 };
 try{
  await page.goto('http://native-management.test/?page_name=admin');
  await page.getByText('设备管理',{exact:true}).first().click({force:true});await click('测试音箱');
  await page.getByText('输出音量',{exact:true}).waitFor();await page.waitForTimeout(350);
  assert.equal(await page.locator('iframe').count(),0);
  await page.screenshot({path:path.resolve(web,'../../../build/echomuse-native-status.png')});
  assert.equal(await page.getByText('GATT 工具',{exact:true}).count(),0,'official tabs exclude custom tools');
   assert.equal(await page.getByText('WiFi 确认',{exact:true}).count(),0,'no duplicate wifi tab');
   assert.equal(await page.getByText('VAD 资产',{exact:true}).count(),0,'no duplicate asset tab');
   await click('活动');await page.getByText('语音轮次记录暂不可用',{exact:true}).waitFor();
  await click('配置');await page.getByText('保存配置',{exact:true}).waitFor({state:'attached'});
  await page.getByText('125',{exact:true}).scrollIntoViewIfNeeded();await page.waitForTimeout(250);
   await page.screenshot({path:path.resolve(web,'../../../build/echomuse-native-eq.png')});
   const name=await field('播放器名称');await name.fill('改名一');
  await click('保存配置');await wait(()=>writes.length===1);
  await page.getByText('配置已保存并排队下发，实际生效请在设备上核对',{exact:true}).first().waitFor();
  assert.deepEqual(writes[0],{sendspinName:'改名一'},'save only the changed key');
  await name.fill('原名');await click('保存配置');await wait(()=>writes.length===2);
  assert.deepEqual(writes[1],{sendspinName:'原名'},'saving back to the original value still sends a delta');
  await page.waitForTimeout(150);
  conflict=true;await name.fill('冲突后草稿');await click('保存配置');await wait(()=>writes.length===3);
  await page.getByText(/配置冲突，未保存的输入已保留/).first().waitFor();
  assert.equal(await name.inputValue(),'冲突后草稿');conflict=false;
  await click('扫描周边网络');await wait(()=>commands.length===1);
  assert.deepEqual(commands[0],{action:'wifi_scan'});
  await (await field('网络名称（SSID）')).fill('新网络');
  await (await field('网络密码')).fill('test-password');await click('切换到新网络');
  await wait(()=>commands.length===2);
  assert.deepEqual(commands[1],{action:'wifi',ssid:'新网络',psk:'test-password'});
  await click('更新');await page.getByLabel('刷新资产状态',{exact:true}).waitFor();
  await page.getByText('已安装',{exact:true}).waitFor();
  assert.equal(await page.locator('iframe').count(),0);
  await click('日志');await click('读取日志');
  await page.getByText('raw-device-log 原文',{exact:false}).waitFor();
  assert.ok((await page.locator('body').innerText()).includes('Firmware'),'raw log is not translated');
  logsFail=true;await click('读取日志');await page.getByText(/设备命令执行失败/).first().waitFor();
  await page.getByText('raw-device-log 原文',{exact:false}).waitFor();
  await click('清空');await page.getByText('尚未读取日志。',{exact:true}).waitFor({state:'attached',timeout:2000}).catch(()=>{});
  assert.equal(await page.getByText('raw-device-log 原文',{exact:false}).count(),0);
  await page.setViewportSize({width:900,height:650});await click('更新');
  await page.getByLabel('刷新资产状态',{exact:true}).scrollIntoViewIfNeeded();
  await page.screenshot({path:path.resolve(web,'../../../build/echomuse-native-updates-compact.png')});
  assert.equal(await page.locator('iframe').count(),0);
  await page.setViewportSize({width:1440,height:1000});await click('关闭设备详情');await click('扩展工具');await click('测试音箱');
   await page.getByText('GATT 工具',{exact:true}).waitFor();
   assert.equal(await page.getByText('状态',{exact:true}).count(),0,'custom workspace excludes official tabs');
   await click('原始诊断');await click('读取设备报告');
   await wait(async()=>(await page.locator('body').innerText()).includes('"volume_state"'));
   await click('设备兼容');await (await field('私听动画')).fill('[]');await click('保存兼容设置');
   await wait(()=>writes.length===4);assert.deepEqual(writes[3],{listeningAnim:[]},'unique compatibility setting uses the one delta pipeline');
   assert.equal(await page.getByText('WiFi 确认',{exact:true}).count(),0);
   assert.equal(await page.getByText('VAD 资产',{exact:true}).count(),0);
   await click('关闭设备详情');await click('返回设备管理');await click('待批准音箱');
  await page.getByText('新设备 · 等待批准',{exact:true}).waitFor();
  assert.equal(await page.locator('iframe').count(),0);
  await (await field('设备名称')).fill('已批准音箱');await click('批准并加入设备');
  await page.getByText('输出音量',{exact:true}).waitFor();await page.waitForTimeout(350);
  assert.equal(pending.approved,true);assert.equal(pending.label,'已批准音箱');assert.deepEqual(errors,[]);
  console.log(JSON.stringify({result:'PASS',checks:'five native tabs/config-delta/save-back/409-draft/wifi-scan-switch/raw-logs/error-clear/compact-updates',writes,commands,errors}));
 }catch(e){await page.screenshot({path:path.resolve(web,'../../../build/echomuse-native-management-failure.png')});console.error('clicks',await page.evaluate(()=>window.__nativeClicks.slice(-9)));console.error('fields',await page.locator('input,textarea').evaluateAll(nodes=>nodes.map(n=>({value:n.value,field:n.getAttribute('data-native-test-field'),rect:n.getBoundingClientRect().toJSON()}))));console.error('diagnostics',JSON.stringify({errors,writes,commands,text:(await page.locator('body').innerText()).slice(-5000)}));throw e;}
 finally{await Promise.all(sockets.map(async socket=>{try{await socket.close()}catch{}}));await page.close();await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1});
