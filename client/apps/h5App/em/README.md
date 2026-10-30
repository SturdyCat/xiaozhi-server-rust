# 官方 EchoMuse 设备管理页（仅 web）

从上游 [EchoMuse controller](https://github.com/wilbowes/EchoMuse/tree/main/controller/static)
**原样 vendor** 的设备管理界面：设备卡列表 + 设备详情六 tab（状态 / 活动 / 配置 / 控制台 /
更新 / 日志）。由 Rust 服务端的静态目录同源托管在 `/em/`，与 Kuikly 管理台（`/`）是两个
独立文档，全局变量零冲突。

## 目录

```
em/
├── index.html                 裁剪自上游 dashboard.html（只改 4 处，见下）
├── LICENSE                    上游 MIT 全文（Copyright (c) 2026 Wil Bowes）
├── README.md                  本文件
├── README.sha256              源码/产物摘要（tools/build_em_dashboard.sh 生成）
└── static/
    ├── dashboard.jsx          上游源码逐字拷贝（源真相，不下发）
    ├── dashboard.js           esbuild 产物（入库；首行有 src-sha256 横幅）
    ├── adapter.js             本项目新增：/api/devices* → /api/echomuse/** 的 fetch/WS 适配
    └── host.js                本项目新增：挂载官方 Card + Detail 的宿主
```

`index.html` 相对上游 `dashboard.html` 只改 4 处：`<title>`、删 favicon 行、加
`<meta name="referrer" content="no-referrer">`（token 在 URL 里）、主题强制浅色，以及
`<body>` 末尾多加载 `adapter.js` / `host.js`。

## 第三方库走 CDN（不入库）

React / xterm / addon-fit / 字体不再 vendor 进仓库，`index.html` 直接从 CDN 加载，
URL 与上游 `controller/Dockerfile` 下载地址逐字一致，JS/CSS 均带 SRI
（`integrity="sha384-…"` + `crossorigin="anonymous"`）锁死字节：

| 资源 | 版本 | CDN |
|---|---|---|
| React / ReactDOM（UMD production） | 18.2.0 | cdnjs |
| @xterm/xterm（js + css） | 5.5.0 | jsdelivr |
| @xterm/addon-fit | 0.10.0 | jsdelivr |
| DM Sans / DM Mono（woff2，@font-face 直链） | gstatic v17 / v16 | fonts.gstatic |

字体经 CSS `url()` 引用无 SRI 机制，gstatic 路径自带 `/vNN/` 版本段锁定；页面已有
`no-referrer`。注意：该页面**运行时需要外网可达这两个 CDN**（内网/离线环境会白屏）。

## 再生成 dashboard.js

```bash
tools/build_em_dashboard.sh                      # 有 esbuild 在 PATH 时
ESBUILD_BIN=/path/to/esbuild tools/build_em_dashboard.sh
```

脚本做三件事，任何一步不符即失败：

1. `esbuild --bundle=false --jsx=transform`（与上游 Dockerfile 同参数、同版本 0.20.2），
   首行写入 `src-sha256` 横幅；
2. **摘除** `dashboard.jsx` 末尾的 `ReactDOM.createRoot(...).render(<App/>)`（断言恰好 1 处）
   ——否则加载即挂载官方完整 App（用户体系 / Settings / 配网向导）；
3. 写 `README.sha256`。

改了 `dashboard.jsx` 而没重跑脚本 → `:apps:h5App:verifyEmBundle` 会让 `publishWeb` 失败。

## 适配层（adapter.js）

上游全部网络请求只有一个出口（`const API` 里的 `fetch`），所以本页用一层
`window.fetch` 拦截完成路径与形状改写，`dashboard.jsx` 一行不改。

| 官方 | 本服务端 | 说明 |
|---|---|---|
| `GET /api/devices` | `GET /api/echomuse/devices` | **拆包**成数组（官方当数组用） |
| `GET /api/devices/{id}` | 同名（echomuse 前缀） | 记录 → 官方 device 形状 |
| `POST .../{id}/approve`、`PATCH .../{id}` | `POST .../{id}/approve` | 认领与改名同一端点 |
| `DELETE .../{id}` | `DELETE /api/echomuse/devices/{id}` | |
| `POST .../{id}/config` | `POST .../{id}/settings` | 只转发 40 键白名单 |
| `GET .../{id}/logs` | `GET .../{id}/system/logs?source=messages` | 文本 → 条目数组 |
| `GET .../{id}/oww_assets` | `GET .../{id}/oww/state` | 五态聚合 |
| `POST .../{id}/wifi`、`wifi/scan` | `POST .../{id}/command {action}` | 扫描轮询 `/runtime` 的 `wifi_scan` |
| `GET .../{id}/turns`、`wake-samples` | 空数组 | 本服务端未采集 |
| 控制台 `WS .../{id}/shell` | `WS .../{id}/console` | 帧协议转换，见下 |

未实现的动作（固件 OTA / rollback / emOS 更新 / debloat / pair / 资产打包下发 /
**release 流** / **模型仓库** / 配网向导 / 用户体系 / 支持包）统一返回 **501 + 中文可读
说明**。上游每个失败路径都会 `alert(e.error)` 或 `setXxxError(e.error)`，所以这些提示
一定会显示在界面上，不会静默。

### 控制台协议桥

上游 `Shell`（xterm）是**裸二进制**协议（`0x00+stdin`、`0x01+resize`），本服务端
`/console` 是 **JSON 行协议**（`{type:"input",data}` / `{type:"output",text}`）。桥的关键
决策是**恒定回 `{type:"shell_meta", pty:false}`**：上游 pty 分支每敲一个字符就发一帧，
而服务端对每个 input 帧都补一个换行 —— 那样会"每字符执行一条命令"。pty:false 时上游
自己做本地回显 + 行缓冲，回车才发一整行，正好对上本服务端的行语义（等价于官方对老
固件的降级路径）。`0x01` resize 直接丢弃（本服务端不做 resize）。

## 已知显示偏差（数据缺口，前端不编造）

本移植**不改 Rust**，所以下面这些上游有、本服务端没有的数据一律按"未采集"渲染：

| 位置 | 偏差 | 原因 |
|---|---|---|
| 状态 tab Voice assistant 行 | 恒显示红色 `No satellite server` | 本服务端没有 HA/ESPHome 语音卫星桥（`docs/echomuse.md` 明确"不做 HA 转发面"），语音走 xiaozhi 流水线；该行**与真实链路健康无关** |
| 状态 tab Link 行 | 按页面协议推断：`https` → 绿 `wss (TLS)`，`http` → 琥珀 `plain ws` | `/runtime.link` 只有 authenticated / token_required，没有设备侧 TLS 位 |
| 状态 tab | `Flash wear`、`Last boot`、`health`、`rttMs`、`linkQuality`（分钟丢包带）显示「—」 | 设备侧/控制器侧本就不采集 |
| 状态 tab | 不出现 `Pair` 按钮 | 固件支持 `register.pairing` 主动请求（`pairingCapable:true`），"固件自己不会问"那条提示不适用；`/pair` 端点本服务端未实现 |
| 活动 tab | Turns 恒 0，Success / Median 显示「—」 | 没有 turns 端点 |
| 更新 tab | Available / Rollback slot 等 release 字段显示「—」/ No release info，Auto-checks 显示 off | 没有 release 流与 `/api/system/status` |
| 日志 tab | 时间列是按行号倒推的**合成时间**（拉取时刻 − 行号 × 1s） | 设备日志行不含时间戳 |
| 配置 tab | 每段默认显示 `Local override (7 of 7)` | 本服务端没有 fleet 全局配置，settings 整份落盘；7 段全算"本地覆盖" |
| 配置 tab | 唤醒词模型下拉只有内置项 | 没有模型仓库端点 |
| 配置 tab | 保存时白名单外（controller 侧：meter 族、led 族、console 族、controllerEndpoints…）的键被丢弃且**界面无提示** | 这些键设备端不消费，服务端会过滤；改动它们请走 Kuikly 管理台对应面板 |

配置保存走**增量下发**：上游 `pushConfig` 发的是整份 effectiveConfig（fleet 打底 + 设备
覆盖），而本服务端 `/settings` 会把收到的每个键都写进设备设置。adapter 先读一次设备
记录，只 POST 与设备当前 settings 不同的键——否则"改一个开关"会把其余三十几个键按
fleet 默认值重刷（例如把设备真实的 `startupVolume=60` 悄悄写成 85）。

## 本地自测

页面没有自己的数据源，必须由服务端提供 `/api/echomuse/**`。只想看渲染时，起一个
静态桩服务托管 `client/apps/h5App/web/em/`，并桩出 `/api/echomuse/devices`、
`/devices/{id}`、`/runtime`、`/system/logs`、`/oww/state`、`/settings`、`/command`
以及 `/devices/{id}/console` 的 WebSocket（`ready` → `shell_meta{pty:false}` →
`output`），然后：

- 列表：`/em/?token=t`
- 详情：`/em/?token=t&device=<id>`（`Detail` 的 tab 是内部状态，靠点击切换）

## 第三方许可

| 组件 | 版本 | 许可 |
|---|---|---|
| EchoMuse controller（dashboard.jsx / dashboard.html） | main 分支 2026-10-29 | MIT, Copyright (c) 2026 Wil Bowes |
| React / ReactDOM（UMD production） | 18.2.0 | MIT |
| @xterm/xterm + @xterm/addon-fit | 5.5.0 / 0.10.0 | MIT |
| DM Sans、DM Mono（woff2 子集） | Google Fonts v17 / v16 | SIL Open Font License 1.1 |

CDN 地址与上游 `controller/Dockerfile` 下载源逐字一致（cdnjs / jsdelivr /
fonts.gstatic）。

## 与 Kuikly 管理台的关系

Kuikly 管理台的「EchoMuse」页顶部有一张入口卡（`EchoMuseOfficialEntry.kt`）指向
`/em/?token=…`。原 Kuikly 六 tab 页面**保留**，承载本项目专有能力：DLNA 音乐、
BLE GATT 面板、唤醒词资产（VAD）安装等官方界面没有的功能。

## 注意

- 链接必须带尾斜杠 `/em/`：Rust 的 `ServeDir` 未知路径会回退到主站 `index.html`，
  那样会在 `/em/` 下启动 Kuikly 造成二次初始化。
- `web/em/` 是 `syncEmAssets` 的拷贝产物，已 gitignore；改 `em/` 结构后先
  `rm -rf client/apps/h5App/web/em`（`publishWeb` 不清理旧文件）。
