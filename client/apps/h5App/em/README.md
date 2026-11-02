# 官方 EchoMuse 设备管理页（仅 web）

从上游 [EchoMuse controller](https://github.com/wilbowes/EchoMuse/tree/main/controller/static)
**原样 vendor** 的设备管理界面：设备卡列表 + 设备详情六 tab（状态 / 活动 / 配置 / 控制台 /
更新 / 日志）。由 Rust 服务端的静态目录同源托管在 `/em/`，与 Kuikly 管理台（`/`）是两个
独立文档，全局变量零冲突。

## 目录

```
em/
├── index.html                 裁剪自上游 dashboard.html（宿主适配见下）
├── LICENSE                    上游 MIT 全文（Copyright (c) 2026 Wil Bowes）
├── README.md                  本文件
├── README.sha256              源码/产物摘要（tools/build_em_dashboard.sh 生成）
└── static/
    ├── dashboard.jsx          上游源码逐字拷贝（源真相，不下发）
    ├── dashboard.js           esbuild 产物（入库；首行有 src-sha256 横幅）
    ├── adapter.js             本项目新增：/api/devices* → /api/echomuse/** 的 fetch/WS 适配
    └── host.js                本项目新增：挂载官方 Card + Detail 的宿主
```

`index.html` 相对上游 `dashboard.html` 的宿主适配：`<title>`、删 favicon 行、加
`<meta name="referrer" content="no-referrer">`（token 在 URL 里）、主题强制浅色、
`<body>` 末尾多加载 `adapter.js` / `host.js`，以及仅嵌入模式生效的布局 CSS。

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

- 独立列表：`/em/?token=t`
- 独立详情：`/em/?token=t&device=<encoded-id>`（列表 + 官方模态详情，关闭后仍是列表）
- 嵌入单设备：`/em/?embed=1&device=<encoded-id>&token=<encoded-token>`

### 嵌入 URL 契约

- `embed` 仅精确值 `1` 启用嵌入；缺省、`0`、`true` 等均保留独立模式。
- `device` 是原始设备 id 经 `encodeURIComponent(id)` 编码后的查询参数；宿主使用
  `URLSearchParams` 解码一次，适配器在请求设备端点时重新编码。不要把原始 `+`、`&`、
  `#` 等直接拼入查询串，也不要预先二次编码。
- `token` 可选，使用 `encodeURIComponent(token)` 编码；鉴权方式与独立模式一致。
  路径必须是带尾斜杠的 `/em/`，参数顺序不限。
- `tab` 可选且仅接受 `status`、`activity`、`config`、`shell`、`updates`、`logs`；缺省或非法值回退
  `status`。`shell` 映射官方 Detail 的内部 `console` tab。嵌入宿主在官方 Detail 首次提交后按精确
  tab 文本选择该 tab，再隐藏**本 Detail 范围内**的官方 tab bar；官方设备工具栏保留，
  因为改名与删除动作位于该处。适配器不修改 React 源码，也不做全局 DOM 搜索或点击。
  未审批设备忽略 `tab`，继续显示官方审批表单。
- 嵌入模式只读取指定设备的记录与 runtime（每 3 秒轮询），不请求设备列表，不渲染全局
  计数、返回管理台链接或设备卡。缺少/空 `device` 直接显示错误且不发设备请求；读取失败
  显示内联错误并继续轮询同一个 id，绝不回退列表或其他设备。
- 官方 `Detail` 原样复用：已审批设备保留状态 / 活动 / 配置 / 控制台 / 更新 / 日志六 tab；
  未审批设备仍按官方流程显示审批表单，审批后刷新并进入状态 tab。删除后刷新同一个 id，
  显示服务端返回的不存在错误，不跳转全局列表。
- 嵌入 CSS 只将详情最外层遮罩/浮动模态框展开成填满容器的普通面板（无遮罩、模糊、外边框、
  圆角和阴影），隐藏详情 Close 与内部 tab bar；保留官方设备工具栏中的改名、删除与审批能力。
  父容器/iframe 应提供明确高度，详情正文在容器内滚动。
  官方源码和生成 bundle 无须改动或重建。

### Node 回归测试

```bash
node --test client/apps/h5App/em/tests/host.test.cjs
node --check client/apps/h5App/em/static/host.js
```

测试无需安装依赖，执行真实宿主和适配器，覆盖 URL 编解码/鉴权、嵌入加载/错误恢复、缺少 id、
审批/删除回调、配置局部更新、独立列表与深链兼容，并校验 CSS 隔离及官方 bundle 摘要。
测试使用轻量 hook 驱动、将官方组件视为叶子，不代替浏览器里的布局、tab 交互和 xterm 验证。

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

Kuikly 管理台的「EchoMuse」页使用设备摘要卡打开同一管理弹窗，顶部由 GearUI 系统 Tabs
统一导航：官方状态/活动/配置/控制台/更新/日志六项，与 DLNA 音乐/GATT 工具/VAD 资产/
模型文件/WiFi 确认/原始诊断六个原生扩展混排。不再另开官方页面，不设「官方/原生」二级切换；
功能相同的旧原生六 Tab 源码已移除，官方内容通过带 `tab` 的 embed URL 嵌入。

## 注意

- 链接必须带尾斜杠 `/em/`：Rust 的 `ServeDir` 未知路径会回退到主站 `index.html`，
  那样会在 `/em/` 下启动 Kuikly 造成二次初始化。
- `web/em/` 是 `syncEmAssets` 的拷贝产物，已 gitignore；改 `em/` 结构后先
  `rm -rf client/apps/h5App/web/em`（`publishWeb` 不清理旧文件）。
