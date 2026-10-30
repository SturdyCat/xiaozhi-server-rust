/*
 * adapter.js —— 把上游 EchoMuse dashboard 的 `/api/devices*` 接口面，映射到本项目
 * Rust 服务端的 `/api/echomuse/**`。
 *
 * 为什么是"外层拦截"而不是改 dashboard.jsx：
 *   - dashboard.jsx 是**逐字拷贝**的上游源码（升版时整体替换），任何手改都会漂移；
 *   - 官方全部网络出口只有一个 `const API`（dashboard.jsx:98-189），内部统一走
 *     `fetch(ingressPath(path))`，所以包一层 window.fetch 就能改掉全部路径与响应形状；
 *   - 控制台 `Shell` 走 WebSocket，另包一层 window.WebSocket 做帧协议转换。
 *
 * 加载顺序：本文件必须在 dashboard.js 之前（它要拦到 API 定义之前的 fetch 引用也无妨，
 * fetch 是在调用时解析的），并且在 host.js 之前（host 用 window.EMAdapter）。
 *
 * 口径：本项目 Rust 侧**零业务改动**，缺的数据一律给出"未采集"的空值，让上游 UI 按
 * 自身规则渲染「—」；不支持的动作返回可读的 501，而不是静默失败。
 */
(function () {
  'use strict';

  var RAW_FETCH = window.fetch.bind(window);
  var RAW_WS = window.WebSocket;
  var EM = '/api/echomuse';
  var TOKEN = new URLSearchParams(location.search).get('token') || '';
  // 本页挂在 /em/ 下，官方 ingressPath() 产出 "./api/x"，会解析成 /em/api/x。
  var EM_PREFIX = /^\/em(?=\/)/;

  // settings 白名单：与 server/src/device/echomuse/admin.rs 的 SETTINGS_KEYS 逐字一致
  // （40 个键，由 admin_tests.rs::settings_keys_are_exactly_the_device_wire_contract 钉死）。
  // 不在表里的键设备端不消费，转发过去会被服务端过滤，这里提前挡掉并告诉用户。
  var SETTINGS_KEYS = [
    'wakeWordEnabled', 'owwOnDevice', 'owwModel', 'owwThreshold', 'wakeSound',
    'wakeSoundLevel', 'wakeMic', 'vadThreshold', 'vadSpeechMs', 'vadSilenceMs',
    'adcMicpga', 'adcDigitalGain', 'micGainDb', 'startupVolume', 'aecEnabled',
    'aecDelayMs', 'aecTailMs', 'aecRefSource', 'agcEnabled', 'beamformingEnabled',
    'beamAngle', 'bargeInEnabled', 'bargeInThreshold', 'responseLevel',
    'volumeButtonSound', 'remoteVolumeArc', 'eqBands', 'eqLoudness',
    'bassGuardEnabled', 'bassGuardDb', 'limiterEnabled', 'limiterThreshold',
    'limiterRelease', 'duckDb', 'bleProxyEnabled', 'bleProxyConnections',
    'sendspinEnabled', 'sendspinName', 'sendspinUnpaired', 'listeningAnim'
  ];
  var SETTINGS_SET = Object.create(null);
  SETTINGS_KEYS.forEach(function (k) { SETTINGS_SET[k] = true; });

  // 上游把"设备本地覆盖哪些段"当作 config_sections；本服务端一直是整份 settings
  // 落盘（无分段继承语义），所以声明全部段由设备覆盖，保证 effectiveConfig() 生效。
  var ALL_SECTIONS = ['playback', 'wakeword', 'microphones', 'ring', 'advanced',
    'bluetooth', 'sendspin'];

  var pendingWifi = Object.create(null);   // device_id → ssid（切网横幅用）
  var lastWifiResult = Object.create(null);

  // ---- 基础工具 ----

  function jsonResponse(status, obj) {
    return new Response(JSON.stringify(obj), {
      status: status,
      headers: { 'Content-Type': 'application/json; charset=utf-8' }
    });
  }

  function notSupported(message) {
    return jsonResponse(501, { error: message, code: 'not_supported' });
  }

  /** 解析请求目标：只接管同源 /api/*；其它（blob、跨源）原样放行。 */
  function resolveTarget(input) {
    var raw = typeof input === 'string' ? input : (input && input.url) || '';
    if (!raw) return null;
    var abs;
    try { abs = new URL(raw, location.href); } catch (e) { return null; }
    if (abs.origin !== location.origin) return null;
    var path = abs.pathname.replace(EM_PREFIX, '');
    if (path.indexOf('/api/') !== 0) return null;
    return { path: path, search: abs.search, url: path + abs.search };
  }

  function authHeaders(extra) {
    var h = extra || {};
    if (TOKEN) h.Authorization = 'Bearer ' + TOKEN;
    return h;
  }

  /** 直连我方端点（不走被拦截的 fetch）。 */
  function emCall(path, init) {
    var opts = init || {};
    opts.headers = authHeaders(opts.headers || {});
    return RAW_FETCH(EM + path, opts).then(function (r) {
      return r.text().then(function (text) {
        var data = null;
        try { data = text ? JSON.parse(text) : null; } catch (e) { data = null; }
        return { ok: r.ok, status: r.status, data: data, text: text };
      });
    });
  }

  function emJson(path) {
    return emCall(path).then(function (r) {
      if (!r.ok) throw new Error((r.data && r.data.error) || ('HTTP ' + r.status));
      return r.data;
    });
  }

  // ---- 设备记录 → 官方 device 形状 ----

  /** 管理台页面是否走 https —— 我方没有设备侧的 TLS 位可读，只能按同源协议推断。 */
  function pageIsTls() {
    try { return location.protocol === 'https:'; } catch (e) { return false; }
  }

  function baseOsOf(record) {
    var v = String(record.base_os || '');
    if (/^emos/i.test(v)) return 'emos';
    if (/^fire/i.test(v)) return 'fireos';
    return null;
  }

  function emosVersionOf(record) {
    var m = /^emos[-_ ]?v?([\d.]+)/i.exec(String(record.base_os || ''));
    return m ? m[1] : null;
  }

  /**
   * 上游 Detail/Card 读的是 `device.*`（dashboard.jsx:1563-2917），这里逐字段搬。
   *
   * 我方 `/runtime` 的 `stats` 键名（cpuPct / cpuTempC / memUsedMb / wifiRssi …）与上游
   * 设备上报的 `device.stats.*` 完全一致，所以 stats 整块直传、不改名。
   * 上游有、我方从不采集的字段（health / rttMs / linkQuality / firmware_previous /
   * emos*）**不填 0、也不编造**，留 undefined 让 UI 自己显示「—」。
   */
  function mapDevice(record, runtime) {
    var rt = runtime || null;
    var listen = rt && rt.listen_state ? rt.listen_state : null;
    var listenState = listen && listen.state ? listen.state : '';
    var wifiResult = rt ? rt.wifi_result : null;
    var deviceId = record.device_id;

    // 切网结果一到就撤掉"切网中"横幅（官方同一判据：以设备上报为准）。
    if (wifiResult && wifiResult !== lastWifiResult[deviceId]) {
      lastWifiResult[deviceId] = wifiResult;
      delete pendingWifi[deviceId];
    }

    var dev = {
      device_id: deviceId,
      label: record.label || '',
      approved: !!record.approved,
      connected: !!record.online,
      connected_secs: record.connected_secs || null,
      ip: record.ip || null,
      first_seen: record.first_seen || null,          // 上游 relTime() 按秒解释
      last_seen: record.last_seen || null,

      firmware_ver: record.version || null,
      firmware_previous: null,
      baseOs: baseOsOf(record),
      emosVersion: emosVersionOf(record),
      emosLatest: null,
      emosOffer: null,
      emosUpdateAvailable: null,
      firmware_update: false,                          // 无 release 流，不宣称可升级

      kernelArch: record.kernel_arch || null,
      kernelRelease: record.kernel_release || null,
      bootReason: record.boot_reason || null,
      sendspinCapable: false,
      bleConnectCapable: false,                        // BLE GATT 在 Kuikly 页里
      owwShadowCapable: true,                          // /oww/state 有真实五态
      listening: listenState === 'stream' || listenState === 'local',
      speaking: false,
      thinking: false,
      muted: !!(rt && rt.mute_state && rt.mute_state.muted),
      listen: listen,
      // 上游 `device.volume` 是 **0..1 的分数**（dashboard.jsx:2248 乘 100 显示），
      // 我方 `volume_state.level` 是 0..100 的整数百分比 —— 不换算会显示成 "8500%"。
      volume: rt && rt.volume_state && typeof rt.volume_state.level === 'number'
        ? rt.volume_state.level / 100
        : null,

      stats: rt ? rt.stats : null,
      wifi: {
        pending: pendingWifi[deviceId] || null,
        last_result: wifiResult || null,
        scan: rt ? rt.wifi_scan : null
      },
      // 我方 `/runtime` 的 `link` 只有 authenticated / token_required，没有 TLS 位。
      // Link 行按**同源页面的协议**推断：管理台本身走 https 时设备口一律 wss，
      // http 时是明文 ws（与 `[echomuse]` 默认明文端口一致）。这是前端推断，
      // README 的偏差表里已写明。
      linkTls: pageIsTls(),
      // `token_required` 才是"发没发 token"：true = 配置里有 `[echomuse].token`。
      linkTokenIssued: rt && rt.link ? !!rt.link.token_required : undefined,
      // 我方固件支持 register 里带 `pairing:true` 主动请求配对（routes.rs:145），
      // 所以"固件自己不会问、得从这儿点 Pair"的那条提示不适用（/pair 也未实现）。
      pairingCapable: true,
      pairRequest: rt && rt.pair_requested_at ? { at: rt.pair_requested_at } : null,

      config: record.settings || {},
      config_sections: ALL_SECTIONS,
      use_global_config: false
    };
    return dev;
  }

  // ---- HTTP 处理器 ----

  function listDevices() {
    return emCall('/devices').then(function (r) {
      if (!r.ok) return jsonResponse(r.status, r.data || { error: 'HTTP ' + r.status });
      var devices = (r.data && r.data.devices) || [];
      // 上游 App 把 /api/devices 的返回值**当数组**用，这里必须拆包。
      return jsonResponse(200, devices.map(function (d) { return mapDevice(d, null); }));
    });
  }

  function getDevice(id) {
    return emCall('/devices/' + encodeURIComponent(id)).then(function (r) {
      if (!r.ok) return jsonResponse(r.status, r.data || { error: 'HTTP ' + r.status });
      return jsonResponse(200, mapDevice(r.data, null));
    });
  }

  function deleteDevice(id) {
    return emCall('/devices/' + encodeURIComponent(id), { method: 'DELETE' })
      .then(function (r) { return jsonResponse(r.ok ? 200 : r.status, r.data || {}); });
  }

  /** 审批（首次认领）与改名走同一条端点：我方 approve 接受可选 label。 */
  function setLabel(id, init) {
    return readJson(init).then(function (body) {
      var payload = { label: (body && body.label) || '' };
      return emCall('/devices/' + encodeURIComponent(id) + '/approve', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
      }).then(function (r) {
        if (!r.ok) return jsonResponse(r.status, r.data || { error: 'HTTP ' + r.status });
        return jsonResponse(200, mapDevice(r.data, null));
      });
    });
  }

  /** 值相等判断：标量严格相等，数组/对象按 JSON 文本比（eqBands、listeningAnim 这类）。 */
  function sameValue(a, b) {
    if (a === b) return true;
    if (a && b && typeof a === 'object' && typeof b === 'object') {
      try { return JSON.stringify(a) === JSON.stringify(b); } catch (e) { return false; }
    }
    return false;
  }

  /**
   * 配置下发：上游 body = `{config_sections, ...config}`，我方 `/settings` 只接受
   * 40 键扁平白名单（部分更新）。
   *
   * 上游发的是**整份 effectiveConfig**（fleet 打底 + 设备覆盖，`dashboard.jsx:1695`），
   * 而我方 `/settings` 会把收到的每个键都写进设备设置。若照原样转发，用户只改一个
   * 开关也会把另外三十几个键按 fleet 默认值重刷一遍（例如把设备真实的
   * `startupVolume=60` 悄悄改成默认 85）。先读一次设备记录、**只下发与设备当前设置
   * 不同的键**，改一个键就只写一个键。
   *
   * 白名单外的键（meter 族、led 族、console 族、controllerEndpoints 等控制器侧
   * 字段）一律不转发，并在响应里以 `dropped` 报出——上游 UI 不显示它，见
   * em/README.md 的偏差表。
   */
  function pushConfig(id, init) {
    return readJson(init).then(function (body) {
      body = body || {};
      var sections = body.config_sections || ALL_SECTIONS;
      var wanted = {};
      var dropped = [];
      Object.keys(body).forEach(function (k) {
        if (k === 'config_sections' || k === 'use_global_config') return;
        if (SETTINGS_SET[k]) wanted[k] = body[k];
        else dropped.push(k);
      });
      return emCall('/devices/' + encodeURIComponent(id)).then(function (g) {
        var stored = (g.ok && g.data && g.data.settings) || {};
        var delta = {};
        Object.keys(wanted).forEach(function (k) {
          if (!sameValue(stored[k], wanted[k])) delta[k] = wanted[k];
        });
        var changedKeys = Object.keys(delta);
        if (!changedKeys.length) {
          // 一处都没改：不必打设备（上游"Revert all to fleet"后直接保存会走到这里）。
          return jsonResponse(200, {
            config: wanted,
            config_sections: sections,
            use_global_config: false,
            persisted: true,
            delivered: false,
            changed: [],
            dropped: dropped
          });
        }
        return emCall('/devices/' + encodeURIComponent(id) + '/settings', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(delta)
        }).then(function (r) {
          if (!r.ok) return jsonResponse(r.status, r.data || { error: 'HTTP ' + r.status });
          return jsonResponse(200, {
            config: wanted,
            config_sections: sections,
            use_global_config: false,
            persisted: !!(r.data && r.data.persisted),
            delivered: !!(r.data && r.data.delivered),
            changed: changedKeys,
            dropped: dropped
          });
        });
      });
    });
  }

  /**
   * 设备日志：上游要 `[{ts,level,source,message}]`，我方 `/system/logs?source=` 回
   * 一段 shell 输出（`{output:"<多行文本>"}`）。
   *
   * 注意：设备侧日志行**不带时间戳**，这里的 ts 是按行号倒推的合成值（拉取时刻 − 行号×1s），
   * 只用于把"最近的在最上"排开。真实时间需要设备固件配合，见 em/README.md。
   */
  function deviceLogs(id) {
    return emCall('/devices/' + encodeURIComponent(id) + '/system/logs?source=messages')
      .then(function (r) {
        if (!r.ok) return jsonResponse(r.status, r.data || { error: 'HTTP ' + r.status });
        var text = (r.data && r.data.output) || '';
        var now = Date.now();
        var lines = text.split('\n').filter(function (l) { return l.trim().length > 0; });
        if (lines.length > 400) lines = lines.slice(lines.length - 400);
        var entries = lines.map(function (line, i) {
          var level = /error|fail|panic|denied/i.test(line) ? 'error'
            : /warn/i.test(line) ? 'warn' : 'info';
          return {
            ts: now - (lines.length - i) * 1000,
            level: level,
            source: 'device',
            message: line
          };
        });
        return jsonResponse(200, entries.reverse());
      });
  }

  /**
   * OWW 资产五态：上游要 `{status, free_mb, blocked, missing[], required[], problems[]}`，
   * 我方 `/oww/state` 回 `{vad:{state}, ort:{state}, free_mb, states[]}`。
   * 聚合规则取"最坏的那一件"——与上游 dashboard 的显示语义一致。
   */
  function owwAssets(id) {
    return emCall('/devices/' + encodeURIComponent(id) + '/oww/state').then(function (r) {
      if (!r.ok) return jsonResponse(r.status, r.data || { error: 'HTTP ' + r.status });
      var d = r.data || {};
      var states = [d.vad && d.vad.state, d.ort && d.ort.state].filter(Boolean);
      var status = 'unknown';
      if (states.indexOf('blocked') >= 0) status = 'blocked';
      else if (states.indexOf('unknown') >= 0 || states.length === 0) status = 'unknown';
      else if (states.indexOf('outdated') >= 0 || states.indexOf('missing') >= 0) status = 'outdated';
      else if (states.length && states.every(function (s) { return s === 'installed'; })) {
        status = 'installed';
      }
      return jsonResponse(200, {
        status: status,
        free_mb: d.free_mb == null ? null : d.free_mb,
        blocked: status === 'blocked',
        missing: [],
        required: [],
        problems: [],
        vad: d.vad || null,
        ort: d.ort || null
      });
    });
  }

  /**
   * WiFi 扫描：上游 `POST /wifi/scan` 期望 `{networks:[{ssid,signal}]}`；我方只有
   * `command{action:"wifi_scan"}`（202 排队）→ 结果经 `/runtime` 的 `wifi_scan.networks`
   * 上报，两者形状一致，这里轮询等它出现。
   */
  function wifiScan(id, init) {
    return readJson(init).then(function () {
      return emCall('/devices/' + encodeURIComponent(id) + '/command', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'wifi_scan' })
      });
    }).then(function (r) {
      if (!r.ok) return jsonResponse(r.status, r.data || { error: 'HTTP ' + r.status });
      var deadline = Date.now() + 20000;
      function poll() {
        return emJson('/devices/' + encodeURIComponent(id) + '/runtime').then(function (rt) {
          var scan = rt && rt.wifi_scan;
          if (scan && Array.isArray(scan.networks) && scan.networks.length) {
            var networks = scan.networks.map(function (n) {
              return { ssid: n.ssid || '', signal: n.signal == null ? null : n.signal };
            });
            return jsonResponse(200, { networks: networks });
          }
          if (Date.now() > deadline) {
            return jsonResponse(504, {
              error: '设备 20 秒内没有回报扫描结果（可能不支持扫描，或信号门未开）'
            });
          }
          return new Promise(function (res) { setTimeout(res, 700); }).then(poll);
        });
      }
      return poll();
    });
  }

  /** 切网：上游 `POST /wifi {ssid,psk}` → 我方 `command{action:"wifi"}`。 */
  function wifiConnect(id, init) {
    return readJson(init).then(function (body) {
      var ssid = (body && body.ssid) || '';
      var psk = (body && body.psk) || '';
      if (!ssid) return jsonResponse(400, { error: '缺少 SSID' });
      return emCall('/devices/' + encodeURIComponent(id) + '/command', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'wifi', ssid: ssid, psk: psk })
      }).then(function (r) {
        if (!r.ok) return jsonResponse(r.status, r.data || { error: 'HTTP ' + r.status });
        pendingWifi[id] = ssid;
        return jsonResponse(202, { queued: true, ssid: ssid });
      });
    });
  }

  function readJson(init) {
    try {
      if (init && typeof init.body === 'string' && init.body) return Promise.resolve(JSON.parse(init.body));
    } catch (e) { /* 非 JSON 体按空处理 */ }
    return Promise.resolve(null);
  }

  function emptyArray() { return jsonResponse(200, []); }

  // ---- 路由表 ----

  var ROUTES = [
    [/^\/api\/devices$/, { GET: listDevices }],
    [/^\/api\/devices\/([^/]+)$/, { GET: getDevice, DELETE: deleteDevice, PATCH: setLabel }],
    [/^\/api\/devices\/([^/]+)\/approve$/, { POST: setLabel }],
    [/^\/api\/devices\/([^/]+)\/config$/, { POST: pushConfig }],
    [/^\/api\/devices\/([^/]+)\/logs$/, { GET: deviceLogs }],
    [/^\/api\/devices\/([^/]+)\/oww_assets$/, { GET: owwAssets }],
    [/^\/api\/devices\/([^/]+)\/turns$/, { GET: emptyArray }],
    [/^\/api\/devices\/([^/]+)\/wake-samples$/, { GET: emptyArray }],
    [/^\/api\/devices\/([^/]+)\/wifi\/scan$/, { POST: wifiScan }],
    [/^\/api\/devices\/([^/]+)\/wifi$/, { POST: wifiConnect }],
    [/^\/api\/system\/status$/, {
      GET: function () { return jsonResponse(200, { update_checks_enabled: false }); }
    }],
    [/^\/api\/releases\/latest$/, { GET: function () { return jsonResponse(200, null); } }],
    [/^\/api\/oww_models$/, { GET: function () { return jsonResponse(200, { models: [] }); } }],
    [/^\/api\/global\/config$/, { GET: function () { return jsonResponse(200, {}); } }],
    [/^\/api\/auth\/me$/, {
      GET: function () { return jsonResponse(200, { username: 'admin', role: 'admin' }); }
    }],
    [/^\/api\/auth\/logout$/, { POST: function () { return jsonResponse(200, {}); } }]
  ];

  // 未实现动作的**可读** 501：上游每个失败路径都会 alert(e.error) 或 setXxxError(e.error)，
  // 所以这里给出"接下来该怎么办"，而不是让它 404 成一句 Failed。
  var UNSUPPORTED = [
    [/\/update$/, '本服务端未实现固件 OTA 下发（需要 A/B 槽与 release 流）；请在设备侧直接升级。'],
    [/\/rollback$/, '本服务端未实现固件回滚槽操作。'],
    [/\/emos_update$/, '本服务端未实现 emOS 镜像更新下发。'],
    [/\/debloat$/, '本服务端未实现 debloat 脚本下发。'],
    [/\/pair$/, '配对在本服务端是只读状态：请长按设备 action 键发起，然后在「状态」页查看配对时刻。'],
    [/\/oww_assets$/, '本服务端不支持网页端打包下发 OWW 资产：VAD 语音门请用 Kuikly 管理台的「安装 VAD」，ONNX 运行时与唤醒词模型需人工部署。'],
    [/\/wake-samples\/[^/]+$/, '本服务端未采集唤醒样本，无样本可标记/删除。'],
    [/\/ble_proxy\/key$/, 'BLE 代理配对键未移植；BLE GATT 面板在 Kuikly 管理台的配置页。'],
    [/\/sendspin\/token$/, '本服务端未实现 Sendspin 配对令牌。'],
    [/^\/api\/releases\//, '本服务端没有 release 仓库（未实现固件发布流）。'],
    [/^\/api\/oww_models/, '本服务端没有唤醒词模型仓库；模型文件请用管理台「文件」页管理。'],
    [/^\/api\/emos\//, '本服务端未实现 emOS 镜像上传。'],
    [/^\/api\/provision\//, '配网向导未移植到本管理台（本移植范围仅设备卡 + 六个 tab）。'],
    [/^\/api\/users/, '本服务端没有多用户体系（管理台用单 token 鉴权）。'],
    [/^\/api\/support\//, '本服务端未实现支持包收集。'],
    [/^\/api\/system\/config$/, '本服务端没有 fleet 全局配置接口；每台设备的设置见「配置」页。'],
    [/^\/api\/events/, '本服务端没有事件总线；设备列表按 5 秒轮询刷新。']
  ];

  function matchRoute(path, method) {
    for (var i = 0; i < ROUTES.length; i++) {
      var m = ROUTES[i][0].exec(path);
      if (!m) continue;
      var handler = ROUTES[i][1][method];
      if (handler) {
        // 路由捕获的是 URL 里的原始片段（设备号形如 aa%3Abb%3Acc…），先解一次再交给
        // 处理器；处理器拼后端 URL 时会重新 encode。不在这儿解就会双重编码，
        // 服务端看到的设备号变成字面量 "aa%3Abb…" → 404。
        return { handler: handler, args: m.slice(1).map(function (s) {
          try { return decodeURIComponent(s); } catch (e) { return s; }
        }) };
      }
    }
    return null;
  }

  function unsupportedFor(path) {
    for (var i = 0; i < UNSUPPORTED.length; i++) {
      if (UNSUPPORTED[i][0].test(path)) return UNSUPPORTED[i][1];
    }
    return '本服务端的 EchoMuse 接口不提供 ' + path + '（前端已适配，后端未实现）。';
  }

  window.fetch = function (input, init) {
    var target = resolveTarget(input);
    if (!target) return RAW_FETCH(input, init);
    // 本服务端自己的 /api/echomuse/* 原样放行（host.js 直接调它）。
    if (target.path.indexOf(EM + '/') === 0 || target.path === EM) return RAW_FETCH(input, init);
    var method = String((init && init.method) || (input && input.method) || 'GET').toUpperCase();
    var hit = matchRoute(target.path, method);
    if (hit) {
      try {
        return Promise.resolve(hit.handler.apply(null, hit.args.concat([init])));
      } catch (e) {
        return Promise.resolve(jsonResponse(500, { error: String(e && e.message || e) }));
      }
    }
    if (method === 'GET' || method === 'POST' || method === 'PATCH' || method === 'DELETE') {
      return Promise.resolve(notSupported(unsupportedFor(target.path)));
    }
    return RAW_FETCH(input, init);
  };

  // ---- 控制台 WebSocket 桥 ----
  //
  // 上游 Shell（dashboard.jsx:1006）是**裸二进制**协议：`0x00+stdin`、`0x01+cols/rows`，
  // 服务端先发文本 `{type:"shell_meta",pty}`。我方 `/console` 是**JSON 行协议**：
  // 浏览器发 `{type:"input",data:"<一行>"}`，服务端回 `{type:"output",text}`。
  //
  // 关键决策：**恒定告诉上游 pty:false**。上游 pty 分支在 term.onData 里每敲一个字符就
  // 发一帧，而我方服务端对每个 input 帧都会补一个换行 —— 那样会变成"每敲一个字符执行
  // 一条命令"。pty:false 时上游自己做本地回显 + 行缓冲，回车才发一整行纯字符串帧，
  // 正好对上我方的行语义（功能等价于官方对老固件的降级路径）。
  function ConsoleSocket(officialUrl) {
    var m = /\/api\/devices\/([^/]+)\/shell(?:\?|$)/.exec(String(officialUrl));
    var id = m ? decodeURIComponent(m[1]) : null;
    var qs = '';
    try { qs = new URL(String(officialUrl), location.href).search || ''; } catch (e) { qs = ''; }
    if (!id) { qs = qs || (TOKEN ? '?token=' + encodeURIComponent(TOKEN) : ''); }

    var self = this;
    var ws;
    var mode = null;          // 设备实际建立的模式（pty / pipe）
    var pendingEcho = null;   // pty 设备会回显整行，剥掉以免重影
    var echoFrames = 0;

    this.readyState = 0;
    this.binaryType = 'arraybuffer';
    this.onopen = null;
    this.onmessage = null;
    this.onclose = null;
    this.onerror = null;

    ws = new RAW_WS(
      (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host +
      EM + '/devices/' + encodeURIComponent(id) + '/console' +
      (qs ? qs : (TOKEN ? '?token=' + encodeURIComponent(TOKEN) : ''))
    );

    ws.onopen = function () {
      self.readyState = 1;
      if (self.onopen) self.onopen({});
    };
    ws.onclose = function (e) {
      self.readyState = 3;
      if (self.onclose) self.onclose({ code: (e && e.code) || 1000 });
    };
    ws.onerror = function (e) {
      if (self.onerror) self.onerror(e);
    };

    function emit(text) {
      if (self.onmessage) self.onmessage({ data: text });
    }

    ws.onmessage = function (e) {
      if (typeof e.data !== 'string') return;
      var f;
      try { f = JSON.parse(e.data); } catch (err) { return; }
      switch (f.type) {
        case 'ready':
          mode = f.mode || 'pipe';
          emit(JSON.stringify({ type: 'shell_meta', pty: false }));
          if (f.insecure) {
            emit('\r\n\x1b[33m设备未配置 token：同网段任何人可连入 root 命令通道\x1b[0m\r\n');
          }
          break;
        case 'output':
          var text = f.text || '';
          if (pendingEcho != null) {
            if (text.indexOf(pendingEcho) === 0) {
              text = text.slice(pendingEcho.length).replace(/^\r?\n/, '');
              pendingEcho = null;
            } else if (++echoFrames > 2) {
              pendingEcho = null;
            }
          }
          if (text) emit(text);
          break;
        case 'notice':
          emit('\r\n\x1b[2m' + (f.text || '') + '\x1b[0m\r\n');
          break;
        case 'error':
          emit('\r\n\x1b[31m[' + (f.code || 'error') + '] ' + (f.error || '') + '\x1b[0m\r\n');
          break;
        case 'closed':
          emit('\r\n\x1b[2m' + (f.detail || f.reason || '会话已结束') + '\x1b[0m\r\n');
          break;
        default:
          break;
      }
    };

    this.send = function (data) {
      if (self.readyState !== 1) return;
      if (typeof data === 'string') {
        if (data === '\x03') { ws.send(JSON.stringify({ type: 'ctrl-c' })); return; }
        var line = data.replace(/\r$/, '').replace(/\n$/, '');
        // pty 设备会把整行回显回来；pipe 模式不回显，不需要剥。
        pendingEcho = mode === 'pty' ? line : null;
        echoFrames = 0;
        ws.send(JSON.stringify({ type: 'input', data: line }));
        return;
      }
      var bytes = new Uint8Array(data);
      if (bytes.length === 0) return;
      // 0x01 = resize：本服务端不做 resize，直接丢弃（上游仅 pty 模式会发）。
      if (bytes[0] === 0x01) return;
      if (bytes[0] === 0x00) {
        var text = '';
        try { text = new TextDecoder().decode(bytes.subarray(1)); } catch (err) { text = ''; }
        self.send(text);
      }
    };

    this.close = function () {
      try { ws.close(); } catch (e) { /* 已关闭 */ }
    };
  }

  window.WebSocket = function (url, protocols) {
    if (/\/api\/devices\/[^/]+\/shell/.test(String(url))) {
      return new ConsoleSocket(url);
    }
    return new RAW_WS(url, protocols);
  };
  window.WebSocket.prototype = RAW_WS.prototype;
  window.WebSocket.CONNECTING = 0;
  window.WebSocket.OPEN = 1;
  window.WebSocket.CLOSING = 2;
  window.WebSocket.CLOSED = 3;

  window.EMAdapter = {
    token: TOKEN,
    device: mapDevice,
    getDevice: function (id) {
      return emJson('/devices/' + encodeURIComponent(id));
    },
    getRuntime: function (id) {
      return emJson('/devices/' + encodeURIComponent(id) + '/runtime');
    }
  };
})();
