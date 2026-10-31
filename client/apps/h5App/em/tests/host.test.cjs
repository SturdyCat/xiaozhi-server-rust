'use strict';

// Dependency-free host/adapter regression tests. The tiny hook driver executes
// host.js, not a second implementation of its routing/render logic. Official
// components remain leaves; browser layout/xterm still need a browser smoke test.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { createHash } = require('node:crypto');
const base = path.resolve(__dirname, '..');
const host = fs.readFileSync(path.join(base, 'static/host.js'), 'utf8');
const adapter = fs.readFileSync(path.join(base, 'static/adapter.js'), 'utf8');

function boot(search, options = {}) {
  let cursor = 0;
  let tree;
  let component;
  const slots = [];
  const effects = [];
  const intervals = new Map();
  const requests = [];
  const classes = new Set();
  const tabbarClasses = new Set();
  let tabButtons = [];
  const clickedTabs = [];
  const observers = [];
  const wrapperNode = {
    querySelectorAll: selector => selector === '.em-tabs button' ? tabButtons : [],
  };
  const record = { device_id: options.id || 'device-one', approved: true, online: true, ...options.record };
  const root = { classList: { add: value => classes.add(value) } };
  let failure = options.failure;
  let serial = 0;
  const React = {
    Fragment: 'Fragment',
    createElement: (type, props, ...children) => {
      props = props || {};
      if (props.ref && type === 'div' && String(props.className || '').indexOf('em-embedded-detail') === 0) props.ref.current = wrapperNode;
      return { type, props, children: children.flat() };
    },
    useState(initial) {
      const i = cursor++;
      if (!(i in slots)) slots[i] = typeof initial === 'function' ? initial() : initial;
      return [slots[i], next => { slots[i] = typeof next === 'function' ? next(slots[i]) : next; }];
    },
    useRef(initial) {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { current: initial };
      return slots[i];
    },
    useEffect(callback, deps) {
      const i = cursor++;
      const old = slots[i];
      if (!old || deps.some((dep, n) => dep !== old.deps[n])) {
        effects.push(() => {
          if (old && old.cleanup) old.cleanup();
          slots[i] = { deps, cleanup: callback() };
        });
      }
    }
  };
  function render() {
    cursor = 0;
    tree = component();
    while (effects.length) effects.shift()();
    return tree;
  }
  const context = vm.createContext({
    URL, URLSearchParams, Response, React, console,
    location: new URL('http://example.test/em/' + search),
    document: { getElementById: () => root },
    MutationObserver: function (callback) {
      this.observe = () => observers.push(callback);
      this.disconnect = () => {};
    },
    API: {}, Card: 'OfficialCard', Detail: 'OfficialDetail',
    WebSocket: function () {},
    setInterval: (fn, ms) => { const id = ++serial; intervals.set(id, { fn, ms }); return id; },
    clearInterval: id => intervals.delete(id),
    fetch: async (url, init = {}) => {
      requests.push({ url, init });
      if (failure) return new Response(JSON.stringify({ error: failure }), { status: 404 });
      const body = String(url).endsWith('/runtime') ? {}
        : url === '/api/echomuse/devices' ? { devices: [record] } : record;
      return new Response(JSON.stringify(body));
    },
    ReactDOM: { createRoot: () => ({ render: element => { component = element.type; render(); } }) }
  });
  context.window = context;
  vm.runInContext(adapter, context, { filename: 'adapter.js' });
  vm.runInContext(host, context, { filename: 'host.js' });
  return {
    requests, intervals, classes, record, context,
    setTabButtons(labels) {
      tabButtons = labels.map(label => ({
        textContent: label,
        click() { this.clicked = true; clickedTabs.push(label); },
        parentElement: { classList: { add: value => tabbarClasses.add(value) } },
      }));
      observers.forEach(fn => fn());
    },
    tabbarClasses,
    clickedTabs,
    get tree() { return tree; },
    set failure(value) { failure = value; },
    render,
    async settle() { await new Promise(resolve => setImmediate(resolve)); return render(); },
    async poll() { for (const { fn } of intervals.values()) fn(); return this.settle(); }
  };
}

function nodes(tree) {
  return !tree || typeof tree !== 'object' ? []
    : [tree, ...tree.children.flatMap(nodes)];
}
function detail(app) { return nodes(app.tree).find(n => n.type === 'OfficialDetail'); }
function text(tree) {
  return !tree || typeof tree !== 'object' ? String(tree || '') : tree.children.map(text).join('');
}
function assertNoFleet(app) {
  assert.equal(nodes(app.tree).some(n => n.type === 'OfficialCard' || n.type === 'a'), false);
  assert.doesNotMatch(text(app.tree), /Devices ·|返回管理台|No devices yet/);
  assert.equal(app.requests.some(r => r.url === '/api/echomuse/devices'), false);
}

test('invalid embedded tab falls back to official status', async () => {
  const app = boot('?embed=1&device=device-one&tab=bogus');
  await app.settle();
  app.setTabButtons(['status', 'activity', 'config', 'console', 'updates', 'logs']);
  assert.deepEqual(app.clickedTabs, ['status']);
});

for (const [requested, official] of [
  ['status', 'status'], ['activity', 'activity'], ['config', 'config'],
  ['shell', 'console'], ['updates', 'updates'], ['logs', 'logs'],
]) {
  test('embedded tab selects exact official content and hides only its bar: ' + requested, async () => {
    const app = boot('?embed=1&device=device-one&tab=' + requested);
    await app.settle();
    app.setTabButtons(['status', 'activity', 'config', 'console', 'updates', 'logs']);
    assert.deepEqual(app.clickedTabs, [official]);
    assert.equal(app.tabbarClasses.has('em-embedded-tabbar-hidden'), true);
    assert.match(app.tree.props.className, /em-embedded-approved/);
    assertNoFleet(app);
  });
}

test('embedded request decodes device/token once and uses only selected endpoints', async () => {
  const id = 'room /?#+% 雪';
  const token = 't+&=% 雪';
  const app = boot('?embed=1&device=' + encodeURIComponent(id) + '&token=' + encodeURIComponent(token), { id });
  assert.match(text(app.tree), /Reading device/);
  assertNoFleet(app);
  await app.settle();
  assert.equal(app.classes.has('em-embedded'), true);
  const pane = detail(app);
  assert.equal(pane.props.device.device_id, id);
  assert.equal(pane.props.token, token);
  assert.equal(pane.props.isAdmin, true);
  assert.equal(app.context.API.token, token);
  assert.deepEqual(app.requests.map(r => r.url), [
    '/api/echomuse/devices/' + encodeURIComponent(id),
    '/api/echomuse/devices/' + encodeURIComponent(id) + '/runtime'
  ]);
  assert.ok(app.requests.every(r => r.init.headers.Authorization === 'Bearer ' + token));
  assert.deepEqual([...app.intervals.values()].map(i => i.ms), [3000]);
  assertNoFleet(app);
});

for (const search of ['?embed=1', '?embed=1&device=']) {
  test('missing device fails inline without fetching fleet: ' + search, async () => {
    const app = boot(search);
    await app.settle();
    assert.match(text(app.tree), /Missing device parameter/);
    assert.equal(app.tree.props.role, 'alert');
    assert.equal(app.requests.length, 0);
    assert.equal(app.intervals.size, 0);
    assertNoFleet(app);
  });
}

test('selected device errors are visible and polling can recover without fleet', async () => {
  const app = boot('?embed=1&device=device-one', { failure: 'Unknown device' });
  await app.settle();
  assert.match(text(app.tree), /Unable to read device: Unknown device/);
  assert.equal(app.tree.props.role, 'alert');
  assertNoFleet(app);
  app.failure = '';
  await app.poll();
  assert.ok(detail(app));
  assertNoFleet(app);
});

test('embedded callbacks patch only selected device and refresh after approval/deletion', async () => {
  const app = boot('?embed=1&device=device-one', { record: { approved: false } });
  await app.settle();
  const before = detail(app);
  before.props.onDeviceConfigChange('other-device', { label: 'wrong' });
  app.render();
  assert.notEqual(detail(app).props.device.label, 'wrong');
  before.props.onDeviceConfigChange('device-one', { label: 'patched' });
  app.render();
  assert.equal(detail(app).props.device.label, 'patched');
  app.record.approved = true;
  before.props.onApprove();
  before.props.onClose();
  await app.settle();
  assert.equal(detail(app).props.device.approved, true);
  assert.notEqual(detail(app).props.key, before.props.key, 'approval remounts the internal approve tab');
  app.failure = 'Device not found';
  detail(app).props.onClose();
  await app.settle();
  assert.equal(detail(app), undefined);
  assert.match(text(app.tree), /Device not found/);
  assertNoFleet(app);
});

for (const mode of ['', '&embed=0', '&embed=true']) {
  test('standalone deep-link retains list/modal and close behavior: ' + (mode || 'default'), async () => {
    const app = boot('?device=device-one' + mode);
    await app.settle();
    assert.equal(app.classes.has('em-embedded'), false);
    assert.ok(app.requests.some(r => r.url === '/api/echomuse/devices'));
    assert.ok(nodes(app.tree).some(n => n.type === 'OfficialCard'));
    assert.ok(detail(app));
    assert.match(text(app.tree), /Devices ·/);
    detail(app).props.onClose();
    app.render();
    app.render();
    assert.equal(detail(app), undefined);
    assert.ok(nodes(app.tree).some(n => n.type === 'OfficialCard'));
  });
}

test('standalone list without device still opens an official detail from its card', async () => {
  const app = boot('');
  await app.settle();
  assert.equal(detail(app), undefined);
  nodes(app.tree).find(n => n.type === 'OfficialCard').props.onClick();
  app.render();
  await app.settle();
  assert.ok(detail(app));
});

test('embedded CSS beats inline/mobile modal styles and hides only detail Close', () => {
  const html = fs.readFileSync(path.join(base, 'index.html'), 'utf8');
  const css = html.slice(html.indexOf('/* Embedded host only:'));
  assert.match(css, /\.em-embedded-detail > div \{[^}]*position: static !important;[^}]*background: transparent !important;[^}]*backdrop-filter: none !important;/);
  assert.match(css, /\.em-embedded-detail > div > \.em-modal \{[^}]*width: 100% !important; height: 100% !important;[^}]*border: 0 !important; border-radius: 0 !important;[^}]*box-shadow: none !important;/);
  assert.match(css, /\.em-embedded-detail \.em-modal-headrow button\[title="Close"\] \{ display: none !important; \}/);
  assert.doesNotMatch(css, /\.em-modal-head\s*\{[^}]*display:\s*none/);
  assert.match(css, /Keep the official device toolbar/);
  assert.match(css, /\.em-embedded-detail \.em-embedded-tabbar-hidden \{ display: none !important; \}/);
  assert.ok(html.indexOf('/* Embedded host only:') > html.indexOf('@media (max-width: 640px)'));
});

test('official source/bundle stay checksum-identical with all six admin tabs', () => {
  const sums = fs.readFileSync(path.join(base, 'README.sha256'), 'utf8').trim().split('\n');
  for (const line of sums) {
    const [, file, expected] = line.trim().split(/\s+/);
    const actual = createHash('sha256').update(fs.readFileSync(path.join(base, 'static', file))).digest('hex');
    assert.equal(actual, expected, file);
  }
  const source = fs.readFileSync(path.join(base, 'static/dashboard.jsx'), 'utf8');
  assert.match(source, /isAdmin \? \['status', 'activity', 'config', 'console', 'updates', 'logs'\]/);
});
