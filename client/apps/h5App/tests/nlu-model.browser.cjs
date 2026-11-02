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
  const makeArtifact = () => ({ build_id: '11111111-1111-4111-8111-111111111111', source_revision: revision,
    model_sha256: 'a'.repeat(64), labels: ['reminder_daily', 'new_help', 'other'], output_dir: '/data/models/nlu-builds/test',
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
      artifact = makeArtifact();
      body = { started: true, state: 'running', build_id: artifact.build_id, source_revision: artifact.source_revision };
    } else if (u.pathname === '/api/nlu/model/build') body = artifact
      ? { state: 'success', build_id: artifact.build_id, source_revision: artifact.source_revision, artifact, log_tail: '' }
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
  const save = async () => {
    const before = writes.length;
    await click('保存配置');
    // Simple-mode hidden field guard remains the original config-save pipeline.
    const confirm = button('确认保存');
    try { await confirm.waitFor({ state: 'visible', timeout: 1500 }); await confirm.click(); } catch (e) {
      if (writes.length === before) throw e;
    }
    await page.waitForFunction(n => window.__nluWrites >= n, before + 1);
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
    await click('保存配置');
    assert.equal(writes.length, 0, 'invalid JSON cannot save through original config pipeline');
    assert.equal(await planInput.inputValue(), '{', 'invalid JSON input is preserved');
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
    await click('构建新模型');
    await page.getByText(/候选模型已生成，尚未启用/).waitFor();
    assert.equal(builds.length, 1); assert.equal(builds[0].expected_revision, 'r2');
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
    await page.waitForFunction(() => window.__nluWrites === 2);
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
