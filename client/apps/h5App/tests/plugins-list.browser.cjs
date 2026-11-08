'use strict';
// 插件列表浏览器回归：pure-base 空底座 + 真机回包形状（lifecycle/requires/dependents/registration.ui）。
// API 拦截只证 UI 门控与加载态契约，不代表真实插件运行或服务端推理。
// 运行：PLAYWRIGHT_PATH=/path/to/playwright-core CHROME_PATH=/path/to/chrome node plugins-list.browser.cjs
const {chromium}=require(process.env.PLAYWRIGHT_PATH||'playwright');
const assert=require('node:assert/strict');
const fs=require('node:fs/promises');
const path=require('node:path');
const web=path.resolve(__dirname,'../web');
const out=path.resolve(__dirname,'../../../build/plugins-browser');

const weatherPlugin={
 id:'weather.official',title:'天气（国家气象中心）',description:'国家气象中心实时天气查询',
 version:'0.1.0',api_version:'0.1',official:true,
 phase:{state:'active'},lifecycle:'active',implements:['skill.weather'],permissions:['http-out:www.nmc.cn'],
 requires:[],dependents:[],
 registration:{
  menu:[{slot:'nav.main',id:'weather',label:'天气',group:'插件',order:60,icon:'settings'}],
  config_fields:[{key:'source',label:'数据源',kind:'str',default_hint:'https://www.nmc.cn',help:'NMC 公开接口根地址',hot:'live',secret:false,required:false,enum_values:[]}],
  actions:[{id:'forecast',label:'查天气',danger:false,help:'输入城市查询实时天气'}],
  ui:{a2ui:'0.1',pages:[{id:'weather-test',kind:'test',title:'天气查询测试',bindings:{actions:['forecast']},
   tree:{type:'card',title:'连通性测试',child:{type:'column',children:[
    {type:'input',id:'city',label:'城市',placeholder:'例：杭州'},{type:'actions',actions:['forecast']},{type:'result'}]}}}]},
 },
};
const pluginsBody=()=>({
 ok:true,
 capabilities:[
  {id:'asr',display:'语音识别',required:true,phase:'active',enabled:true,active_impl:'asr.sensevoice',implementations:[{id:'asr.sensevoice',display:'SenseVoice INT8（本地离线）',enabled:true,phase:'active'}],last_error:null},
  {id:'vad',display:'语音分段',required:true,phase:'active',enabled:true,active_impl:'vad.silero',implementations:[{id:'vad.silero',display:'Silero VAD',enabled:true,phase:'active'}],last_error:null},
  {id:'tts',display:'语音合成',required:true,phase:'active',enabled:true,active_impl:'tts.kokoro',implementations:[{id:'tts.kokoro',display:'Kokoro INT8',enabled:true,phase:'active'}],last_error:null},
  {id:'llm',display:'对话生成',required:true,phase:'active',enabled:true,active_impl:'llm.openai',implementations:[{id:'llm.openai',display:'OpenAI 兼容',enabled:true,phase:'active'}],last_error:null},
 ],
 device_plugins:[],
 config:{path:'/data/data.db',persistent:true},
 ui:{mode:'pure-base',static_business:false},
 wasm:{plugins:[weatherPlugin],extensions:{'skill.weather':{state:'resolved',implementation:'weather.official',preferred:''}}},
});

(async()=>{
 const browser=await chromium.launch({headless:true,...(process.env.CHROME_PATH?{executablePath:process.env.CHROME_PATH}:{})});
 const page=await browser.newPage({viewport:{width:1440,height:1000}});
 const errors=[];
 let pluginsDelay=1500; // 首次拉取延迟：覆盖「快照晚于首帧」的加载窗口
 page.on('pageerror',e=>errors.push(e.message));
 // 管理台 JSON 走 /api/admin/ws 的 WS RPC 复用（getJson/adminTransport），必须按帧协议应答：
 // 客户端 {"type":"rpc","id","method","path"} → 服务端 {"type":"rpc_result","id","status","body"}。
 await page.routeWebSocket('**/api/admin/ws*',ws=>{
  ws.send(JSON.stringify({type:'hello',version:0,server_status:{}}));
  ws.onMessage(msg=>{
   let frame;try{frame=JSON.parse(msg.text());}catch{return;}
   if(frame.type==='ping'){ws.send(JSON.stringify({type:'pong'}));return;}
   if(frame.type!=='rpc')return;
   const respond=()=>ws.send(JSON.stringify({type:'rpc_result',id:frame.id,status:200,body:pluginsBody()}));
   if(frame.path==='/api/plugins'&&pluginsDelay>0){
    const d=pluginsDelay;pluginsDelay=0;setTimeout(respond,d);
   }else{
    respond();
   }
  });
 });
 await page.route('http://plugins.test/**',async route=>{
  const request=route.request(),u=new URL(request.url());
  if(u.pathname.startsWith('/api/')){
   return route.fulfill({status:200,json:{}});
  }
  const resolved=path.resolve(web,'.'+u.pathname);
  if(!resolved.startsWith(web+path.sep)&&resolved!==web)return route.fulfill({status:403,body:'blocked'});
  try{await route.fulfill({path:u.pathname==='/'?path.join(web,'index.html'):resolved});}catch{return route.fulfill({status:404,body:'missing'});}
 });
 try{
  await fs.mkdir(out,{recursive:true});
  await page.goto('http://plugins.test/?page_name=admin');

  // ① 快照在途（前 1500ms）：静态「插件」导航可见；不得误报平台不可用；清单卡显示加载中。
  await page.getByText('正在获取插件清单…',{exact:true}).first().waitFor({timeout:8000});
  assert.equal(await page.getByText('服务端未启用 WASM 插件平台',{exact:true}).count(),0,'loading 不得误报平台不可用');
  assert.equal(await page.getByText('暂无插件',{exact:true}).count(),0,'loading 不得提前宣判空清单');
  const navCount=await page.getByText('插件',{exact:true}).count();
  assert.ok(navCount>=1,'静态插件导航在 wasm 解析前必须可见');
  await page.screenshot({path:path.join(out,'loading.png')});

  // ② 回包落定：插件行渲染（标题 + 动作按钮），空态/不可用文案消失，动态导航出现。
  await page.getByText('天气（国家气象中心）',{exact:true}).first().waitFor({timeout:10000});
  assert.equal(await page.getByText('服务端未启用 WASM 插件平台',{exact:true}).count(),0);
  assert.equal(await page.getByText('正在获取插件清单…',{exact:true}).count(),0);
  assert.ok(await page.getByText('已装载插件',{exact:true}).first().isVisible(),'已装载插件卡可见');
  assert.ok(await page.getByText('查天气',{exact:true}).first().isVisible(),'注册动作按钮可见');
  assert.ok(await page.getByText('天气',{exact:true}).count()>=1,'动态插件导航入口出现');
  await page.screenshot({path:path.join(out,'loaded.png')});

  // ③ 进入静态插件管理页（配置组的「插件」导航项）：清单与上传入口同页成立。
  await page.getByText('插件',{exact:true}).first().click();
  await page.waitForTimeout(300);
  assert.ok(await page.getByText('天气（国家气象中心）',{exact:true}).first().isVisible(),'导航往返后清单仍在');
  assert.equal(errors.length,0,'无页面错误: '+errors.join(' | '));
  console.log('plugins-list browser regression: PASS');
 }catch(e){
  await page.screenshot({path:path.join(out,'failure.png')}).catch(()=>{});
  console.error('FAIL',e.message);
  process.exitCode=1;
 }finally{
  await browser.close();
 }
})();
