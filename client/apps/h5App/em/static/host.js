/*
 * host.js —— 官方 EchoMuse 设备管理页的宿主。
 *
 * 上游 dashboard.jsx 的最后一行是 `ReactDOM.createRoot(...).render(<App/>)`，加载即挂载
 * 完整的官方 App（含用户体系、Settings 全站页、配网向导、Deploy-all）。本移植只要
 * **设备卡列表 + 设备详情六个 tab**，所以生成物里那一行已被 tools/build_em_dashboard.sh
 * 摘除，由本文件挂载官方导出的 `Card` / `Detail` 组件——它们都是顶层 function 声明，
 * 在 --bundle=false 的经典脚本里就是 window.Card / window.Detail，源码零改动。
 *
 * 数据来源：直接读本服务端 /api/echomuse/**（经 adapter.js 的 EMAdapter 复用同一套
 * 记录→device 映射），不走被拦截的 /api/devices 通道。
 */
(function () {
  'use strict';

  var qs = new URLSearchParams(location.search);
  var TOKEN = qs.get('token') || '';
  var h = React.createElement;
  var AD = window.EMAdapter;

  // 官方 API.headers() 读 API.token（`const API` 在全局词法环境里，裸名可访问；
  // window.API 是 undefined，不要写 window.API）。
  if (TOKEN && typeof API !== 'undefined') API.token = TOKEN;

  /*
   * Fleet 基线：本服务端没有"全局配置"，每台设备只有自己的 settings（部分更新）。
   * effectiveConfig() 的语义是「fleet 打底 + 设备覆盖」，所以这里给一份设备端官方
   * loadDefaults() 的真值打底，保证每个控件都有值可显示。
   *
   * 值是官方默认（见 server/src/device/echomuse/admin.rs 的 SETTINGS_KEYS 文档注释）；
   * 带 * 的是本服务端不接收的 controller 侧键（不在 40 键白名单里），仅用于显示，
   * 保存时由 adapter 过滤掉。
   */
  var FLEET = {
    // 播放
    eqBands: [0, 0, 0, 0, 0, 0, 0, 0], eqLoudness: false, duckDb: -18,
    responseLevel: 'low', limiterEnabled: false, limiterThreshold: -3,
    limiterRelease: 100, bassGuardEnabled: false, bassGuardDb: 0,
    streamReply: false, volumeButtonSound: true,
    // 唤醒词
    wakeWordEnabled: true, owwModel: 'hey_jarvis_v0.1', owwThreshold: 0.5,
    owwOnDevice: 'off', owwSpeexNs: false, bargeInEnabled: true,
    bargeInThreshold: 0.05, wakeArbitrationMs: 0, wakeSound: false,
    wakeSoundLevel: 'medium', wakeClipCapture: false, wakeClipMinScore: 0.5,
    // 麦克风
    adcMicpga: 40, adcDigitalGain: 88, micGainDb: 24, beamformingEnabled: true,
    beamAngle: -1, wakeMic: 0, aecEnabled: true, aecDelayMs: 0, aecTailMs: 300,
    aecRefSource: 'auto', nsAsr: false, saveUtterances: false,
    // 灯环
    ledScene: 'off', ledListenColor: '#4080d0', ledThinkColor: '#a08020',
    remoteVolumeArc: false, meterAttack: 10, meterDecay: 100, meterFloor: 0,
    meterGamma: 1, meterRef: 100, meterCurve: 'linear',
    // 高级
    agcEnabled: true, vadThreshold: 0.004, vadSpeechMs: 80, vadSilenceMs: 600,
    buttonSingleTapEvent: 'none', buttonMultiTapMs: 400, consolePassword: '',
    consoleTimeoutMin: 0, controllerEndpoints: '',
    // 蓝牙 / Sendspin
    bleProxyEnabled: false, bleProxyConnections: 0,
    sendspinEnabled: false, sendspinUnpaired: false,
    // 设备自有状态键
    startupVolume: 85
  };

  function authHeaders() {
    return TOKEN ? { Authorization: 'Bearer ' + TOKEN } : {};
  }

  function getJson(path) {
    return fetch(path, { headers: authHeaders() }).then(function (r) {
      return r.json().then(function (body) {
        if (!r.ok) throw new Error((body && body.error) || ('HTTP ' + r.status));
        return body;
      });
    });
  }

  function readJson(path) {
    return getJson(path).catch(function (e) {
      return { __error: String((e && e.message) || e) };
    });
  }

  /** 设备列表：5 秒轮询（与官方 App 同频）。 */
  function useDevices() {
    var listSt = React.useState(null);
    var errSt = React.useState('');
    var aliveRef = React.useRef(true);
    function tick() {
      readJson('/api/echomuse/devices').then(function (body) {
        if (!aliveRef.current) return;
        if (body.__error) { errSt[1](body.__error); return; }
        if (body.enabled === false) {
          errSt[1]('服务端未启用 EchoMuse：配置里把 [echomuse].enabled 设为 true 后重启');
          return;
        }
        errSt[1]('');
        listSt[1]((body.devices || []).map(function (d) { return AD.device(d, null); }));
      });
    }
    React.useEffect(function () {
      aliveRef.current = true;
      tick();
      var iv = setInterval(tick, 5000);
      return function () { aliveRef.current = false; clearInterval(iv); };
    }, []);
    return { list: listSt[0], error: errSt[0], refresh: tick };
  }

  /**
   * 单台详情：registry 记录 + /runtime 合并成官方 device 形状，3 秒轮询。
   * `stats` / `listen` / `volume` / `muted` 全部来自 /runtime，缺了就是 null（UI 显示「—」）。
   */
  function useDetail(id) {
    var devSt = React.useState(null);
    var aliveRef = React.useRef(true);
    function tick() {
      if (!id) { devSt[1](null); return; }
      Promise.all([AD.getDevice(id), AD.getRuntime(id)]).then(function (r) {
        if (aliveRef.current) devSt[1](AD.device(r[0], r[1]));
      }).catch(function () {
        if (aliveRef.current) devSt[1](null);
      });
    }
    React.useEffect(function () {
      aliveRef.current = true;
      devSt[1](null);
      if (!id) return function () { aliveRef.current = false; };
      tick();
      var iv = setInterval(tick, 3000);
      return function () { aliveRef.current = false; clearInterval(iv); };
    }, [id]);
    return { device: devSt[0], refresh: tick, patch: devSt[1] };
  }

  var FONT_LABEL = {
    fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-xs)', color: 'var(--muted)',
    textTransform: 'uppercase', letterSpacing: 'var(--tracking-heading)'
  };

  function Page() {
    var devices = useDevices();
    var selSt = React.useState(qs.get('device') || null);
    var selected = selSt[0];
    var det = useDetail(selected);

    var list = devices.list;
    var approved = (list || []).filter(function (d) { return d.approved; });
    var pending = (list || []).filter(function (d) { return !d.approved; });

    var header = h('div', {
      style: {
        display: 'flex', alignItems: 'baseline', gap: 'var(--space-6)',
        marginBottom: 'var(--space-8)'
      }
    },
      h('div', { style: FONT_LABEL },
        'Devices · ' + approved.length +
        (pending.length ? ' · ' + pending.length + ' pending' : '')),
      h('a', {
        href: '/?page_name=admin' + (TOKEN ? '&token=' + encodeURIComponent(TOKEN) : ''),
        style: {
          marginLeft: 'auto', fontFamily: 'var(--font-sans)',
          fontSize: 'var(--fs-md)', color: 'var(--accent-deep)', textDecoration: 'none'
        }
      }, '← 返回管理台')
    );

    if (devices.error) {
      return h('div', { style: { padding: 'var(--space-12)' } },
        header,
        h('div', {
          className: 'em-on-dark',
          style: {
            background: 'var(--notice-bg)', border: '1px solid var(--notice-line)',
            borderRadius: 'var(--radius-lg)', padding: 'var(--space-7)',
            fontFamily: 'var(--font-sans)', fontSize: 'var(--fs-base)', color: 'var(--text)'
          }
        }, devices.error));
    }

    if (list === null) {
      return h('div', { style: { padding: 'var(--space-12)' } },
        header,
        h('div', { style: { fontFamily: 'var(--font-mono)', color: 'var(--muted)' } },
          'Reading devices…'));
    }

    var grid = list.length === 0
      ? h('div', { style: { fontFamily: 'var(--font-mono)', color: 'var(--muted)' } },
          'No devices yet — power on an Echo running emOS and it will appear here for approval.')
      : h('div', {
          style: {
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill,minmax(190px,1fr))',
            gridAutoRows: '1fr', gap: 'var(--space-6)'
          }
        }, list.map(function (d) {
          return h(window.Card, {
            key: d.device_id,
            device: d,
            onClick: function () { selSt[1](d.device_id); }
          });
        }));

    return h(React.Fragment, null,
      h('div', { style: { padding: 'var(--space-12)' } }, header, grid),
      det.device && h(window.Detail, {
        device: det.device,
        token: TOKEN,
        isAdmin: true,
        globalConfig: FLEET,
        onClose: function () { selSt[1](null); },
        // 审批/改名成功后官方会重取列表；这里同样立刻重取一次，不等 5s 轮询。
        onApprove: function () { devices.refresh(); det.refresh(); },
        onDeviceConfigChange: function (id, patch) {
          det.patch(function (prev) {
            return prev && prev.device_id === id ? Object.assign({}, prev, patch) : prev;
          });
        }
      })
    );
  }

  var root = document.getElementById('root');
  ReactDOM.createRoot(root).render(h(Page));
})();
