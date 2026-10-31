'use strict';
// Actual Kuikly bundles in Chrome; intercepted devices/WS only. No command reaches hardware.
const { chromium } = require(process.env.PLAYWRIGHT_PATH || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const em = path.resolve(__dirname, '..'), web = path.resolve(em, '../web');
const device = { device_id:'test-dot2', label:'测试音箱', approved:true, online:true,
  version:'2.18.1', board:'dot2', base_os:'emos', session_id:'console-session',
  settings:{}, capabilities:['mic','speaker','ble_connect'], ip:'192.0.2.20' };
(async () => {
 const browser = await chromium.launch({headless:true,...(process.env.CHROME_PATH?{executablePath:process.env.CHROME_PATH}:{})});
 const page = await browser.newPage({viewport:{width:1440,height:1000}});
 const errors=[], sent=[], sessions=[];
 let readyMode='pty', raw=true, input='';
 page.on('pageerror', e=>errors.push(e.message));
 await page.routeWebSocket('**/*', socket=>{
  if (!socket.url().includes('/console')) return;
  const session={socket,closed:false}; sessions.push(session);
  socket.onClose(()=>{session.closed=true;});
  socket.send(JSON.stringify({type:'ready',mode:readyMode,raw_input:raw,resize:raw}));
  socket.send(JSON.stringify({type:'output',text:'\x1b[32mroot@dot2:/ # \x1b[0m'}));
  socket.onMessage(message=>{
   const frame=JSON.parse(String(message)); sent.push(frame);
   if(frame.type==='input') {
    if(frame.raw) {
     if(frame.data==='\r') {socket.send(JSON.stringify({type:'output',text:'\r\nnative-command-ok\r\nroot@dot2:/ # '})); input='';}
     else {input+=frame.data;socket.send(JSON.stringify({type:'output',text:frame.data}));}
    } else socket.send(JSON.stringify({type:'output',text:'legacy-command-ok\r\nroot@dot2:/ # '}));
   }
  });
 });
 await page.route('http://echomuse.test/**',async route=>{
  const u=new URL(route.request().url());
  if(u.pathname.startsWith('/api/')) {
   let body={};
   if(u.pathname==='/api/echomuse/devices') body={enabled:true,devices:[device]};
   else if(u.pathname==='/api/echomuse/devices/test-dot2') body=device;
   else if(u.pathname.endsWith('/runtime')) body={device_id:device.device_id,session_id:device.session_id,online:true,stats:{}};
   else if(u.pathname==='/api/files') body={path:'oww-models',entries:[]};
   else if(u.pathname==='/api/config') body={config:{},revision:1};
   return route.fulfill({json:body});
  }
  const official=u.pathname.startsWith('/em/'), rel=official?u.pathname.slice(4):u.pathname.slice(1);
  try {await route.fulfill({path:path.join(official?em:web,rel||'index.html')});}
  catch {await route.fulfill({status:404,body:'missing'});}
 });
 const terminalInput=()=>page.getByRole('textbox',{name:'EchoMuse 控制台键盘输入'});
 const shellTab=()=>page.getByText('控制台',{exact:true}).last();
 const output=()=>page.getByText('root@dot2:/ # ',{exact:false}).first();
 const wait=async predicate=>{for(let i=0;i<80;i++){if(await predicate())return;await page.waitForTimeout(100);}throw Error('condition timed out');};
 try {
  await page.goto('http://echomuse.test/?page_name=admin');
  await page.getByText('EchoMuse',{exact:true}).first().click({force:true});
  await page.getByText('测试音箱',{exact:true}).first().click({force:true});
  await page.locator('iframe').waitFor();
  await shellTab().click({force:true});
  await terminalInput().waitFor({state:'attached'});
  await wait(()=>sessions.length===1&&sent.some(f=>f.type==='resize'));
  assert.equal(await page.locator('iframe').count(),0,'native console must not contain an official iframe');
  await terminalInput().evaluate(el=>{window.__consoleEvents=[];for(const kind of ['focus','blur','keydown','input'])el.addEventListener(kind,e=>window.__consoleEvents.push({kind,key:e.key,value:el.value}));});
  await output().click({timeout:10000});
  await wait(()=>terminalInput().evaluate(el=>document.activeElement===el));
  if(process.env.EM_FOCUS_DEBUG) console.log('focus',await terminalInput().evaluate(el=>({active:document.activeElement===el,disabled:el.disabled,parents:el.parentElement.parentElement.outerHTML.slice(0,1200)})));
  await page.keyboard.type('echo native-ok'); await page.keyboard.press('Enter');
  await wait(()=>sent.some(f=>f.type==='input'&&f.raw&&f.data==='\r'));
  assert.equal(sent.filter(f=>f.type==='input'&&f.raw).map(f=>f.data).join(''),'echo native-ok\r');
  await page.getByText('native-command-ok',{exact:false}).first().waitFor();
  for(const key of ['Tab','ArrowUp','ArrowDown','Control+c','Control+d','Escape','Backspace']) await page.keyboard.press(key);
  await wait(()=>sent.some(f=>f.raw&&f.data==='\x7f'));
  for(const data of ['\t','\x1b[A','\x1b[B','\x03','\x04','\x1b','\x7f']) assert.ok(sent.some(f=>f.raw&&f.data===data),'raw key '+JSON.stringify(data));
  // Real composition and clipboard events exercise the native input bridge.
  await terminalInput().evaluate(el=>{
   el.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}));
   el.value='中文'; el.dispatchEvent(new InputEvent('input',{bubbles:true,isComposing:true,data:'中文'}));
   el.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:'中文'}));
   el.dispatchEvent(new InputEvent('input',{bubbles:true,data:'中文',inputType:'insertText'}));
  });
  await wait(()=>sent.some(f=>f.raw&&f.data==='中文'));
  assert.equal(sent.filter(f=>f.raw&&f.data==='中文').length,1,'IME commit exactly once');
  sessions[0].socket.send(JSON.stringify({type:'output',text:'\x1b[?2004h'}));
  await page.waitForTimeout(100);
  await terminalInput().evaluate(el=>{const dt=new DataTransfer();dt.setData('text/plain','echo 粘贴');el.dispatchEvent(new ClipboardEvent('paste',{bubbles:true,clipboardData:dt}));});
  await wait(()=>sent.some(f=>f.raw&&f.data.includes('粘贴')));
  assert.ok(sent.some(f=>f.raw&&f.data==='\x1b[200~echo 粘贴\x1b[201~'),'bracketed paste preserved');
  // Cursor positioning/erase must replace cells rather than append an ANSI-stripped log.
  sessions[0].socket.send(JSON.stringify({type:'output',text:'\x1b[2J\x1b[Habcdefgh\rXY\x1b[K\r\nnative-screen-ready'}));
  await page.getByText('native-screen-ready',{exact:false}).first().waitFor();
  assert.equal(await page.getByText('XY      ',{exact:false}).count()>0,true);
  await page.screenshot({path:path.resolve(web,'../../../build/echomuse-native-console-smoke.png')});
  const oldSize=sent.filter(f=>f.type==='resize').at(-1);
  await page.setViewportSize({width:900,height:760});
  await wait(()=>sent.filter(f=>f.type==='resize').at(-1)?.cols!==oldSize.cols);
  // A nested confirmation must suppress command input without closing/reopening the shell.
  await page.getByText('删除',{exact:true}).click({force:true});
  await page.getByText('删除设备记录？',{exact:true}).waitFor();
  await wait(()=>terminalInput().isDisabled());
  const before=sent.filter(f=>f.type==='input').length;
  await page.keyboard.type('should-not-send');
  assert.equal(sent.filter(f=>f.type==='input').length,before);
  await page.getByText('取消',{exact:true}).last().click({force:true});
  await wait(async()=>!(await terminalInput().isDisabled()));
  await page.getByText('删除设备记录？',{exact:true}).waitFor({state:'detached'});
  await page.getByText('状态',{exact:true}).last().click({force:true});
  await wait(()=>sessions[0].closed); assert.equal(await terminalInput().count(),0);
  await shellTab().click({force:true}); await wait(()=>sessions.length===2);
  sessions[1].socket.send(JSON.stringify({type:'closed',reason:'device-shell-lost'}));
  await page.getByText('重新连接',{exact:true}).waitFor();
  await page.waitForTimeout(300);assert.equal(sessions.length,2,'never auto-reconnect root shell');
  await page.getByText('重新连接',{exact:true}).click({force:true}); await wait(()=>sessions.length===3);
  await page.getByText('关闭',{exact:true}).last().click({force:true}); await wait(()=>sessions[2].closed);
  await wait(async()=>(await page.locator('div[style*="z-index: 1000"]').count())===0);
  // Old server / pipe firmware must still accept ordinary commands, never raw/resize frames.
  readyMode='pipe';raw=false;sent.length=0;
  await page.getByText('测试音箱',{exact:true}).first().click({force:true});
  await page.locator('iframe').waitFor(); await shellTab().click({force:true});
  await terminalInput().waitFor({state:'attached'}); await page.waitForTimeout(200); await terminalInput().focus();
  await page.keyboard.type('echo legacy');await page.keyboard.press('Enter');
  await wait(()=>sent.some(f=>f.type==='input'&&f.data==='echo legacy'&&!f.raw));
  assert.equal(sent.some(f=>f.raw||f.type==='resize'),false,'pipe must not receive raw/resize');
  await page.getByText('关闭',{exact:true}).last().click({force:true});
  assert.deepEqual(errors,[]);
  console.log(JSON.stringify({result:'PASS',sessions:sessions.length,checks:'native keys/IME/paste/VT/resize/modal/dispose/reconnect/pipe',errors}));
 } catch(error) {
  console.error('diagnostics',JSON.stringify({errors,sent,events:await page.evaluate(()=>window.__consoleEvents),active:await page.evaluate(()=>document.activeElement?.outerHTML?.slice(0,300)),url:page.url(),text:(await page.locator('body').innerText()).slice(-3500)}));
  throw error;
 } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
