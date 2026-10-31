'use strict';

// Optional real-browser regression (not part of dependency-free node:test):
// PLAYWRIGHT_PATH=/path/to/playwright CHROME_PATH=/path/to/chrome node console.browser.cjs
// Uses actual built Kuikly parent + source official React/xterm/adapter. All API
// and console traffic is intercepted; no physical device receives a command.
const { chromium } = require(process.env.PLAYWRIGHT_PATH || 'playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const em = path.resolve(__dirname, '..');
const web = path.resolve(em, '../web');
const device = { device_id: 'test-dot2', label: '测试音箱', approved: true, online: true,
  version: '2.18.1', board: 'dot2', base_os: 'emos', session_id: 'smoke-session',
  settings: {}, capabilities: ['mic', 'speaker', 'ble_connect'], ip: '192.0.2.20' };

(async () => {
  const browser = await chromium.launch({ headless: true,
    ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  // Remote typography is not a functional dependency; keep the regression offline/deterministic.
  await page.route('https://fonts.googleapis.com/**', route => route.abort());
  await page.route('https://fonts.gstatic.com/**', route => route.abort());
  const errors = [];
  const sent = [];
  if (process.env.EM_FOCUS_DEBUG) {
    page.on('console', message => { if (message.text().startsWith('FOCUS')) console.log(message.text()); });
    await page.addInitScript(() => {
      for (const method of ['focus', 'blur']) {
        const original = HTMLElement.prototype[method];
        HTMLElement.prototype[method] = function (...args) {
          console.log('FOCUS ' + method, location.pathname, this.tagName, this.id, this.className, new Error().stack);
          return original.apply(this, args);
        };
      }
      for (const event of ['focusin', 'focusout', 'mousedown', 'pointerdown']) document.addEventListener(event, e => {
        console.log('FOCUS event', event, location.pathname, e.target.tagName, e.target.id, e.target.className, e.defaultPrevented);
      }, true);
    });
  }
  page.on('pageerror', error => errors.push(error.message));
  await page.routeWebSocket('**/*', socket => {
    if (!socket.url().includes('/console')) return;
    socket.send(JSON.stringify({ type: 'ready', mode: 'pty' }));
    socket.onMessage(data => {
      sent.push(JSON.parse(String(data)));
      socket.send(JSON.stringify({ type: 'output', text: 'mock command accepted\r\n' }));
    });
  });
  await page.route('http://echomuse.test/**', async route => {
    const u = new URL(route.request().url());
    if (u.pathname.startsWith('/api/')) {
      let body = {};
      if (u.pathname === '/api/echomuse/devices') body = { enabled: true, devices: [device] };
      else if (u.pathname === '/api/echomuse/devices/test-dot2') body = device;
      else if (u.pathname.endsWith('/runtime')) body = { device_id: device.device_id, session_id: device.session_id, online: true, stats: {} };
      else if (u.pathname === '/api/files') body = { path: 'oww-models', entries: [] };
      else if (u.pathname === '/api/config') body = { config: {}, revision: 1 };
      return route.fulfill({ json: body });
    }
    const official = u.pathname.startsWith('/em/');
    const rel = official ? u.pathname.slice(4) : u.pathname.slice(1);
    try { await route.fulfill({ path: path.join(official ? em : web, rel || 'index.html') }); }
    catch { await route.fulfill({ status: 404, body: 'not found' }); }
  });
  try {
    // Native Kuikly console is covered by native-console.browser.cjs; this retains upstream compatibility coverage.
    const modes = ['standalone', 'iframe'];
    for (const mode of modes) {
      sent.length = 0;
      let scope;
      if (mode === 'standalone') {
        await page.goto('http://echomuse.test/em/?device=test-dot2');
        await page.locator('.em-tabs button').filter({ hasText: /^console$/ }).click();
        scope = page;
      } else {
        await page.goto('http://echomuse.test/em/?embed=1&device=test-dot2&tab=shell');
        scope = page;
      }
      const terminal = scope.locator('.xterm-screen');
      await terminal.waitFor({ timeout: 60000 });
      if (mode !== 'standalone') {
        assert.equal(await scope.locator('.em-modal-head:visible').count(), 0, 'embedded header must be absent');
        assert.equal(await scope.locator('.em-modal-body:visible').count(), 1);
      }
      await terminal.click();
      // Real key events, not term.write/onData injection: catches focus traps.
      await page.keyboard.type('echo keyboard-ok');
      await page.keyboard.press('Enter');
      await page.waitForTimeout(250);
      const active = await scope.locator('body').evaluate(() => ({
        tag: document.activeElement && document.activeElement.tagName,
        className: document.activeElement && document.activeElement.className,
        focused: document.hasFocus(),
      }));
      console.log(mode, JSON.stringify({ sent, active }));
      assert.ok(sent.some(frame => frame.type === 'input' && frame.data === 'echo keyboard-ok'), mode + ': actual keyboard command must reach console adapter');
    }
    for (const [tab, official] of [['status', 'status'], ['activity', 'activity'], ['config', 'config'], ['shell', 'console'], ['updates', 'updates'], ['logs', 'logs']]) {
      await page.goto('http://echomuse.test/em/?embed=1&device=test-dot2&tab=' + tab);
      await page.locator('.em-modal-body[aria-label="测试音箱 ' + official + '"]').waitFor();
      assert.equal(await page.locator('.em-modal-head:visible').count(), 0);
      assert.equal(await page.locator('.em-tabs:visible').count(), 0);
    }
    device.approved = false;
    await page.goto('http://echomuse.test/em/?embed=1&device=test-dot2&tab=status');
    await page.getByRole('button', { name: 'Approve & Add to Fleet' }).waitFor();
    assert.equal(await page.locator('.em-modal-head:visible').count(), 0);
    assert.equal(await page.getByRole('textbox', { name: 'Label', exact: true }).isVisible(), true);
    assert.deepEqual(errors, []);
  } catch (error) {
    console.error('browser diagnostics', JSON.stringify({ errors, url: page.url(),
      frames: page.frames().map(frame => frame.url()), text: (await page.locator('body').innerText()).slice(-2500) }));
    throw error;
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
