'use strict';
// Published native Kuikly UI + mocked management APIs. This does NOT prove model accuracy/inference.
const { chromium } = require(process.env.PLAYWRIGHT_PATH || 'playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const web = path.resolve(__dirname, '../web');
const out = path.resolve(__dirname, '../../../build/nlu-model-browser');

(async () => {
  const browser = await chromium.launch({ headless: true,
    ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  const errors = [], writes = [], builds = [], tests = [];
  let revision = 'r1', conflict = false, measurement = 'heuristic', catalogReads = 0;
  const existingPlan = { conditions: [{ kind: 'slot_present', slot: 'task' }], steps: [
    { api: 'tasks.create', args: { text: '{{slots.task}}', daily: '{{slots.daily_time}}', timezone: 'Asia/Shanghai' } },
    { api: 'replies.say', args: { text: '{{steps.0.speech}}' } } ] };
  const editedPlan = { conditions: [{ kind: 'slot_equals', slot: 'task', value: '喝水' }], steps: [
    { api: 'tasks.create', args: { text: '{{slots.task}}', yearly: { month: 2, day: 29, time: '20:30' }, timezone: 'Asia/Shanghai' } },
    { api: 'devices.call', args: { tool: 'set_volume', arguments: { volume: 30, nested: { values: [1, true, { text: '{{slots.task}}' }] } } } },
    { api: 'replies.say', args: { text: '{{steps.0.speech}}' } } ] };
  const planPlaceholder = '{"conditions":[],"steps":[{"api":"replies.say","args":{"text":"你好"}}]}';
  let config = { revision, nlu: { enabled: true, intents: [{ name: 'reminder_daily',
    templates: ['每天{daily_time}提醒我{task}', '每日{daily_time}提醒我{task}'],
    action: { kind: 'plan', reply: '', plan: existingPlan } }],
    slots: [{ name: 'daily_time', values: [], optional: false }, { name: 'task', values: [], optional: false }], bert_enabled: false,
    bert_model: '/old/model.onnx', bert_vocab: '/old/tokenizer.json', bert_labels: '/old/labels.txt',
    bert_threshold: 0.75, bert_num_threads: 1, bert_max_len: 64, bert_label_map: {} } };
  let artifact = null;
  // 服务端只有唯一底座：状态里不再有 `bases` 臂表，构建请求也不再带 `base_model`。
  const BASE = 'minirbt-h288';
  // base_model 由构建产物回显：服务端只在 manifest 里记录训练用的底座。
  const makeArtifact = () => ({ build_id: '11111111-1111-4111-8111-111111111111', source_revision: revision,
    model_sha256: 'a'.repeat(64), labels: ['reminder_daily', 'new_help', 'other'], output_dir: '/data/models/nlu-builds/test',
    base_model: BASE,
    expected_config: { bert_enabled: true, bert_model: '/data/models/nlu-builds/test/model.onnx',
      bert_vocab: '/data/models/nlu-builds/test/tokenizer.json', bert_labels: '/data/models/nlu-builds/test/labels.txt',
      bert_threshold: 0.75, bert_num_threads: 1, bert_max_len: 64, bert_label_map: {} } });
  page.on('pageerror', e => errors.push(e.message));
  await page.routeWebSocket('**/*', () => {});
  await page.route('http://nlu.test/**', async route => {
    const req = route.request(), u = new URL(req.url());
    if (!u.pathname.startsWith('/api/')) {
      const file = path.resolve(web, '.' + u.pathname);
      if (file !== web && !file.startsWith(web + path.sep)) return route.fulfill({ status: 403, body: 'blocked' });
      try { return await route.fulfill({ path: u.pathname === '/' ? path.join(web, 'index.html') : file }); }
      catch { return route.fulfill({ status: 404, body: 'missing' }); }
    }
    let body = {}, status = 200;
    if (u.pathname === '/api/config' && req.method() === 'POST') {
      const patch = req.postDataJSON(); writes.push(patch);
      if (conflict) { status = 409; body = { message: '配置已被其他窗口修改，请重新加载后再保存' }; }
      else {
        assert.equal(patch.expected_revision, revision);
        revision = 'r' + (Number(revision.slice(1)) + 1);
        config = { ...config, ...patch, revision };
        body = { ok: true, revision, hot: 'next_session', message: '已持久化' };
      }
    } else if (u.pathname === '/api/config') body = { ...config, revision };
    else if (u.pathname === '/api/skills/catalog') {
      catalogReads++;
      body = { state: 'ok', source: 'saved', revision, recognition: 'model_only', executed: false,
        effective: { enabled: true, intents: config.nlu.intents, slots: config.nlu.slots },
        intents: config.nlu.intents,
        capabilities: [{ api: 'devices.call', description: '调用已声明的设备工具', execution: 'device', args_schema: { type: 'object' } }] };
    }
    else if (u.pathname === '/api/nlu/model/build' && req.method() === 'POST') {
      const request = req.postDataJSON(); builds.push(request);
      assert.equal(request.expected_revision, revision);
      // 服务端 `BuildRequest` 是 deny_unknown_fields：客户端多传一个键直接 400。
      assert.ok(!Object.hasOwn(request, 'base_model'), 'build request must not carry base_model');
      artifact = makeArtifact();
      body = { started: true, state: 'running', build_id: artifact.build_id, source_revision: artifact.source_revision,
        base_model: artifact.base_model };
    } else if (u.pathname === '/api/nlu/model/build') body = artifact
      ? { state: 'success', build_id: artifact.build_id, source_revision: artifact.source_revision,
          base_model: artifact.base_model, artifact, log_tail: '' }
      : { state: 'idle', artifact: null, log_tail: '' };
    else if (u.pathname === '/api/nlu/model/test') {
      const request = req.postDataJSON(); tests.push(request);
      assert.equal(request.build_id, artifact.build_id);
      body = { state: 'ok', build_id: artifact.build_id, source_revision: artifact.source_revision,
        model_sha256: artifact.model_sha256, raw_label: 'new_help', intent: 'new_help',
        confidence: 0.91, threshold: 0.75, accepted: true, reject_reason: null,
        expected_intent: request.expected_intent, expected_match: true,
        tokenize_ms: 3, infer_ms: 8, executed: false, measurement };
    } else if (u.pathname === '/api/config/schema') body = { advanced_fields: [], capabilities: [] };
    else if (u.pathname === '/api/plugins') body = { capabilities: [] };
    else if (u.pathname === '/api/test/capabilities') body = { revision, tests: [], samples: [] };
    else if (u.pathname === '/api/workflow') body = { version: 1, revision, plugins: [], condition_kinds: [], config: null };
    else if (u.pathname === '/api/files') body = { path: '', entries: [] };
    return route.fulfill({ status, json: body });
  });
  // Native Kuikly text is public DOM; some renderer versions omit aria labels/roles.
  const button = text => page.getByText(text, { exact: true }).last();
  // Let the native scroll bridge update hit-testing after programmatic DOM scrolling.
  const click = async text => { const b = button(text); await b.scrollIntoViewIfNeeded(); await page.waitForTimeout(250); await b.click(); await page.waitForTimeout(250); };
  const attempt = async text => { const b = button(text); await b.scrollIntoViewIfNeeded(); await page.waitForTimeout(250); await b.click({ force: true }); await page.waitForTimeout(250); };
  // GearUI Select 的弹层里有一层全屏承载 div（overlay 层自身），Playwright 的可点性检查
  // 判它"intercepts pointer events"，点不动；真实用户点击没有这个问题（事件由 overlay 层
  // 自己路由到选项行）。底座不再可选，本文件不再需要按坐标点击弹层选项的辅助函数。
  const saveButton = () => page.getByText('保存配置', { exact: true }).last();
  // 「保存按钮可点」的唯一判据：文案回到「保存配置」。保存期间是「保存中…」，此时点击
  // 在 Kotlin 侧被 `if (saving) return@with` 静默丢弃。所有"点保存"的地方都先过这一句，
  // 否则"没点上"与"点上了被拦"在 `writes.length` 上完全一样（负向断言会假绿）。
  const waitSaveIdle = () => saveButton().waitFor({ state: 'visible', timeout: 15000 });
  // 页面顶部的保存按钮（`ConfigPage.kt:247`）：只有它传了 `onResult` → 会弹 toast。
  // 卡片里那个「保存配置」点下去什么都不会弹，需要"点击确实进了 save()"的证据时用它。
  const clickSaveHeader = async () => {
    const b = page.getByText('保存配置', { exact: true }).first();
    await b.scrollIntoViewIfNeeded();
    await page.waitForTimeout(250);
    await b.click();
    await page.waitForTimeout(250);
  };
  // 这一轮保存是否**真的发出去了**：`writes` 是 mock（Node 侧）收到的 POST，是唯一真值；
  // 页面上的 `__nluWrites` 只是它每 100ms 的镜像。两者都查，免得把"已发出、镜像还没刷新"
  // 误判成没发。
  const waitWrite = (n, timeout) => page
    .waitForFunction(want => window.__nluWrites >= want, n, { timeout })
    .then(() => true)
    .catch(() => writes.length >= n);
  // 每次 save() 都把"点了几次"打出来，**成功也打**：否则"20/20 全绿"和"每轮都重试 2 次"
  // 在日志里长得一模一样——真实 flake 率是 0% 还是 80% 就永远看不出来。
  // 判读口径：**稀疏随机**（20 轮里两三轮回 retry 1 次）= flake 被抑制；
  // **每轮恒定 retry**（例如每轮都恰好 1 次）= harness 与产品之间的必然时序错配，
  // 那时该改的是"等对状态"，不是加重试。
  let saveCallSeq = 0;
  const save = async () => {
    const callNo = ++saveCallSeq;
    // 入口先等上一轮结束：保存期间按钮文案是「保存中…」，此时点击**既不报错也不发请求**
    //（Kotlin 侧 `if (saving) return@with`），随后"等弹层 2s + 等写入 5s"双双超时，
    // 报出"既没有确认弹层，也没有发出请求"。QA 复现的 flake 走的正是这条路径，
    // 所以自愈点放在**每个入口**，而不是在每个旁路各补一次。
    await waitSaveIdle();
    const before = writes.length;
    // 点一次可能被吞：症状与"按钮还在 loading"完全一样——**既没有确认弹层，也没有发出请求**，
    // 浏览器里也没有 JS 异常。已知的**真静默** return 只有一条：`ConfigFormActions.kt:162`
    // 的 `if (saving) return@with`（不设 statusMsg/saveResult/onResult，零反馈）；
    // 其后的 NLU 校验分支（`:164-168`）三处反馈齐全，不是静默。除此之外 Kuikly 的
    // `disabled` 也不会在 DOM 上留原生属性，点了同样只是什么都不发生。
    // 所以不能只点一次：最多 3 次，每次之间靠"按钮回到可点文案"这个真实状态推进，
    // 不用固定 sleep 掩盖；真的发不出去时照样报错，并带上按钮的可交互状态便于定位成因。
    const attemptMax = 3;
    let attemptNo = 0;
    for (attemptNo = 1; ; attemptNo++) {
      await waitSaveIdle();
      await click('保存配置');
      // Simple-mode hidden field guard remains the original config-save pipeline.
      // 确认弹层是**条件出现**的（只在简洁模式藏了高级改动时才有）；没有弹层不代表失败，
      // 判据是"请求确实发出去了"。这里等"写入计数前进"而不是死等弹层，
      // 免得在 409/慢一拍的情况下把合法路径误判成失败（曾出现过的 flake）。
      const confirm = button('确认保存');
      let confirmed = false;
      try {
        await confirm.waitFor({ state: 'visible', timeout: 2000 });
        await confirm.click();
        confirmed = true;
      } catch (e) {
        confirmed = false;
      }
      if (confirmed) {
        await waitWrite(before + 1, 15000);
        break;
      }
      if (!(await waitWrite(before + 1, 5000)) && attemptNo < attemptMax) {
        // 没有弹层也没有请求：成因至少有两种可能（重渲染窗口吞掉这次点击 / 按钮此刻业务禁用），
        // 目前**没有把哪一种确认为机制**，所以只重试一次。⚠️ 实测 60 轮里 ≈10% 的
        // save#2 走到这条 continue（残留反例，见 T02 验收记录），终端分支的 suspect=
        // 四态覆盖不到它们——所以在 continue 前把三态证据先打出来，下次 20 连跑
        // 必然出现几次，靠它自证"吞一次第二次就发出"这条路径的真实形态。
        const el0 = saveButton();
        const [idle0, busy0] = await Promise.all([
          page.getByText('保存配置', { exact: true }).count(),
          page.getByText('保存中…', { exact: true }).count()]);
        const box0 = await el0.boundingBox().catch(() => null);
        const opacity0 = await el0.evaluate(n => (n && getComputedStyle(n).opacity) || null)
          .catch(() => null);
        console.log(`[swallow] save#${callNo} attempt ${attemptNo} box=${JSON.stringify(box0)} `
          + `opacity=${opacity0} 「保存配置」${idle0} 个、「保存中…」${busy0} 个`);
        continue;
      }
      if (writes.length === before) {
        const [idle, busy] = await Promise.all([
          page.getByText('保存配置', { exact: true }).count(),
          page.getByText('保存中…', { exact: true }).count()]);
        // 光有计数分不清"存在但业务禁用"与"存在且可点"：Kuikly 用**视觉变灰**表达禁用，
        // DOM 上没有原生 `disabled`，所以 Playwright 的可点性检查照样通过、点击也不抛。
        // 这里把 box + opacity + busy 一起打出来，四态可判（`suspect=` 是**按这三项推断**的，
        // 不是结论）：
        //   box === null                      → 元素不存在/已卸载
        //   busy > 0                          → `form.saving`：时序问题，等复位是对的
        //   opacity 偏低 + busy === 0          → 业务禁用且不是在保存中，即 `!unsaved`
        //                                        （**测试逻辑问题**：fill() 没让表单变 dirty，
        //                                          重试 100 次也发不出去）
        //   opacity 正常 + busy === 0          → 确实可点却没产生请求（保存管线/渲染）
        const el = saveButton();
        const box = await el.boundingBox().catch(() => null);
        const opacity = await el.evaluate(n => (n && getComputedStyle(n).opacity) || null)
          .catch(() => null);
        const dim = opacity !== null && Number(opacity) < 1;
        const suspect = box === null
          ? '元素不存在/已卸载'
          : busy > 0
            ? 'form.saving（时序，等复位）'
            : dim
              ? '业务禁用且非 saving，即 !unsaved（测试没让表单变 dirty）'
              : '可点却无请求（保存管线/渲染）';
        throw new Error(`保存既没有确认弹层，也没有发出请求（点了 ${attemptNo} 次；`
          + `box=${JSON.stringify(box)} opacity=${opacity} `
          + `「保存配置」${idle} 个、「保存中…」${busy} 个；`
          + `writes=${writes.length}/${before}；suspect=${suspect}）`);
      }
      break;
    }
    // 成功也要打：否则"20/20 全绿"和"每轮都重试 2 次"在日志里完全一样，
    // 真实 flake 率是 0% 还是 80% 永远看不出来。
    console.log(`[retry] save#${callNo} clicked ${attemptNo} time(s)`);
    // 等这一轮保存**真正落地**：保存期间按钮文案是「保存中…」，回到「保存配置」才算结束。
    // 不等的后果：下一次 save() 点在仍处于 loading 的按钮上，既不出弹层也不发请求
    //（5 次 1 挂的根因；`__nluWrites` 在 mock 收到请求时就 +1，早于界面复位）。
    await waitSaveIdle();
  };
  // Expose counts only to synchronize fixture requests, never production behavior.
  await page.exposeFunction('__nluWriteCount', () => writes.length);
  await page.addInitScript(() => { setInterval(async () => { window.__nluWrites = await window.__nluWriteCount(); }, 100); });
  try {
    await fs.mkdir(out, { recursive: true });
    await page.goto('http://nlu.test/?page_name=admin');
    await page.getByText('VAD · 语音分段', { exact: true }).first().waitFor();
    const labeledAxis = page.getByLabel('打开NLU节点', { exact: true });
    await (await labeledAxis.count() ? labeledAxis : page.getByText('NLU', { exact: true }).first()).click();
    await page.getByText('NLU · 意图与技能', { exact: true }).first().waitFor();
    await button('添加技能').waitFor(); // Available in simple mode, not hidden under Advanced.
    assert.equal(await page.getByText('执行阈值', { exact: true }).count(), 0, 'no heuristic threshold control');
    assert.equal(await page.getByText('领先余量', { exact: true }).count(), 0, 'no heuristic margin control');
    await page.getByText(/没有启用可用模型就不能识别技能/).waitFor();
    await click('读取技能清单');
    await page.getByText(/reminder_daily · 能力组合/).waitFor();
    assert.equal(catalogReads, 1); assert.equal(writes.length, 0);
    const planInput = page.getByPlaceholder(planPlaceholder, { exact: true }).first();
    assert.deepEqual(JSON.parse(await planInput.inputValue()), existingPlan, 'saved plan loads intact, not as reply');
    await planInput.fill('{');
    await waitSaveIdle();
    // 点击必须"生效且被拦"：非法 JSON 在 `ConfigFormActions.kt:163` 的 `nlu.validationError()`
    // 里就被拦下，**根本走不到发请求**。所以 `writes.length == 0` 在"点上了被拦"和
    // "没点上（点击丢了 / 按钮此刻业务禁用）"两个世界里都成立，必须另外拿到
    // 一条只有"真的进了 save()"才会出现的痕迹。
    //
    // 痕迹只有 toast，而 **toast 只有页面顶部的保存按钮才有**（`ConfigPage.kt:256` 传了
    // `onResult` → `page.showToast`）。卡片里那个「保存配置」(`ConfigNluModelCard.kt:40`)
    // 没传 `onResult`，点它什么都不会弹，所以这里必须点顶部那个。
    await clickSaveHeader();
    // 再精确到文案：`save()` 拦下时弹的是 `nlu.validationError()`，它带
    // `技能「name」：` 前缀（`ConfigNluState.kt:77`）；而卡片里那条即时校验用的
    // `draft.validationError()`（`ConfigNluCard.kt:132`）**不带前缀**，它在 `fill('{')`
    // 之后就已经出现在屏幕上了。只匹配裸文案会把两者混成一个，等于断言了输入框自己的
    // 静态提示——这正是这条负向断言原本假绿的原因。
    await page.getByText(/技能「.+」：能力组合不是合法 JSON/).first()
      .waitFor({ timeout: 3000 });
    assert.equal(writes.length, 0, 'invalid JSON cannot save through original config pipeline');
    assert.equal(await planInput.inputValue(), '{', 'invalid JSON input is preserved');
    // toast 是覆盖层：不等它退场，紧接着的「添加技能」就可能点在它上面被吞掉
    //（症状是后面只发出 1 个技能，`writes[0].nlu.intents[1]` 直接 undefined）。
    await page.getByText(/技能「.+」：能力组合不是合法 JSON/).first()
      .waitFor({ state: 'hidden', timeout: 15000 });
    await planInput.fill(JSON.stringify(editedPlan));
    await click('添加技能');
    await page.getByPlaceholder('light_control', { exact: true }).last().fill('new_help');
    await page.getByPlaceholder('把{room}的灯{action}', { exact: true }).last().fill('告诉我怎么使用\n我想了解操作方法\n介绍一下功能');
    await attempt('构建新模型');
    assert.equal(builds.length, 0, 'unsaved draft must not start training');
    await save();
    assert.equal(writes.length, 1); assert.equal(config.nlu.bert_enabled, false);
    assert.equal(writes[0].nlu.intents[0].action.kind, 'plan');
    assert.equal(writes[0].nlu.intents[0].action.reply, '', 'result speech must not add a second static reply');
    assert.deepEqual(writes[0].nlu.intents[0].action.plan, editedPlan, 'calendar parameters and result references round-trip in unique save');
    assert.equal(writes[0].nlu.intents[0].action.plan.steps[2].args.text, '{{steps.0.speech}}');
    assert.deepEqual(writes[0].nlu.intents[0].action.plan.steps[0].args.yearly, { month: 2, day: 29, time: '20:30' });
    assert.equal(writes[0].nlu.intents[1].name, 'new_help');
    assert.ok(!Object.hasOwn(writes[0].nlu, 'model_threshold'));
    assert.ok(!Object.hasOwn(writes[0].nlu, 'model_margin'));
    // 底座维度：服务端只有唯一底座，界面没有可切换的臂，构建请求也不带 base_model。
    await click('刷新状态');
    await page.getByText(/当前固定使用预训练底座 minirbt-h288/).waitFor();
    assert.equal(await page.getByText('预训练底座', { exact: true }).count(), 0, 'no base model selector');
    await click('构建新模型');
    await page.getByText(/候选模型已生成，尚未启用/).waitFor();
    assert.equal(builds.length, 1); assert.equal(builds[0].expected_revision, 'r2');
    assert.ok(!Object.hasOwn(builds[0], 'base_model'), 'build request must not carry base_model');
    console.log(`build request: ${JSON.stringify(builds[0])}`);
    await page.getByText(/训练底座：minirbt-h288/).waitFor();
    assert.equal(writes.length, 1, 'build success never saves/activates');
    assert.equal(config.nlu.bert_enabled, false);
    await attempt('采用候选并保存启用');
    assert.equal(writes.length, 1, 'untested candidate must not activate');
    await page.getByPlaceholder('输入没有照抄训练模板的口语表达', { exact: true }).fill('可以讲讲你支持的功能吗');
    await page.getByPlaceholder('填写上面新增的技能名', { exact: true }).fill('new_help');
    await click('测试候选模型');
    await page.getByText('候选版本或推理证据不一致，不能采用。', { exact: true }).waitFor();
    await attempt('采用候选并保存启用');
    assert.equal(writes.length, 1, 'heuristic evidence must fail closed; candidate test never mutates configuration');
    measurement = 'real_onnx_inference';
    await click('测试候选模型');
    await page.getByText(/这一个正例达到阈值且与预期一致/).waitFor();
    assert.equal(writes.length, 1);
    await page.getByPlaceholder('输入没有照抄训练模板的口语表达', { exact: true }).fill('另一个未测试句子');
    await attempt('采用候选并保存启用');
    assert.equal(writes.length, 1, 'editing text invalidates positive evidence and prevents activation');
    await click('测试候选模型');
    await page.getByText(/这一个正例达到阈值且与预期一致/).waitFor();
    await click('采用候选并保存启用');
    const confirm = button('确认保存');
    try { await confirm.waitFor({ state: 'visible', timeout: 1500 }); await confirm.click(); } catch (e) {
      if (writes.length !== 2) throw e;
    }
    await page.waitForFunction(n => window.__nluWrites >= n, 2, { timeout: 15000 });
    // `__nluWrites` 在 mock 收到请求时就 +1，早于界面复位——只等它会让下一步点在仍处于
    // loading 的按钮上（同 flake 根因）。这里补等界面真的退回可点状态。
    await page.getByText('保存配置', { exact: true }).last()
      .waitFor({ state: 'visible', timeout: 15000 });
    assert.equal(writes[1].expected_revision, 'r2');
    assert.equal(writes[1].nlu.bert_enabled, true);
    assert.equal(writes[1].nlu.bert_model, artifact.expected_config.bert_model);
    assert.equal(writes[1].nlu.bert_vocab, artifact.expected_config.bert_vocab);
    assert.equal(writes[1].nlu.bert_labels, artifact.expected_config.bert_labels);
    assert.equal(writes[1].nlu.bert_threshold, 0.75);
    assert.deepEqual(writes[1].nlu.intents[0].action.plan, editedPlan, 'candidate adoption cannot erase a skill plan');
    assert.equal(tests.length, 3);
    conflict = true;
    await page.getByPlaceholder('好的，已为你打开灯。', { exact: true }).fill('保留的未保存应答');
    await save();
    await page.getByText('重新加载', { exact: true }).first().waitFor();
    await attempt('构建新模型');
    assert.equal(builds.length, 1, '409 conflict preserves draft and blocks builds');
    assert.ok(await page.getByPlaceholder('好的，已为你打开灯。', { exact: true }).inputValue() === '保留的未保存应答');
    // D 回归：重进页面后仍只有固定底座，界面不残留可切换的臂表。
    await page.reload();
    await page.getByText('VAD · 语音分段', { exact: true }).first().waitFor();
    const axisAfterReload = page.getByLabel('打开NLU节点', { exact: true });
    await (await axisAfterReload.count() ? axisAfterReload : page.getByText('NLU', { exact: true }).first()).click();
    await page.getByText('NLU · 意图与技能', { exact: true }).first().waitFor();
    await click('刷新状态');
    await page.getByText(/当前固定使用预训练底座 minirbt-h288/).waitFor();
    assert.equal(await page.getByText('预训练底座', { exact: true }).count(), 0, 'no base model selector after reload');
    assert.deepEqual(errors, []);
    await page.screenshot({ path: path.join(out, 'nlu-candidate.png') });
    console.log('PASS NLU UI: simple-mode add, saved revision build, no automatic activation, isolated test, fail-closed evidence, explicit save activation, 409 draft retention. Mocked APIs only; no real model accuracy claim.');
  } catch (e) {
    console.error('NLU browser state', { revision, builds, tests, writes: writes.length, errors });
    await fs.writeFile(path.join(out, 'failure-body.txt'), await page.locator('body').innerText());
    await page.screenshot({ path: path.join(out, 'failure.png') });
    throw e;
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
