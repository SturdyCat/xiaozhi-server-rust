# W12 / devicehub 页前置只读研究 · 权威归档

> **归档说明（归档波次：W15-2）**：本文为 W12/W13/W14b 阶段**研究原文归档**，记录当时旧端权威与拍板依据，保留史实；当前实施口径见 [W15 资产盘点](<./w15-legacy-assets-research.md>) 与 [W15 构建链](<./w15-production-build-research.md>)，管理台当前为 `client/admin-web`（见 [admin-web 指引](<../../../client/admin-web/AGENTS.md>)）。

> **文档性质**：W12 / devicehub 页（EchoMuse iframe + 涂鸦 + 设备运行面）迁移的**唯一权威契约**，供工程师实施与 QA 回归逐节核对。由研究阶段发送给 team-lead 的「报告 1/2 + 报告 2/2 + 补充勘误」**原文全量合并**而成，另按交付要求补入「逐端点完整 wire 表」（附录 A–F）。研究原文一字未删。
> **研究纪律**：研究全程只读——未创建/修改任何源码文件、未执行构建、未做 git 操作；本归档是经 team-lead 授权写入的单一文档。
> **证据口径**：全部结论带 `file:line`；绝对路径索引见 §14。根路径 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/`。
> **勘误（QA 回归发现，2026-11-11）**：附录 C.5 sync result 值集含 `"busy"` 属**过声明**——服务端 `sync.rs` 全文 0 处产出 `"busy"`，并发场景实际返回 `result="skipped"` + message「已有一次同步在进行中」（QA W12 M1，实施与注释已按此修正，commit 8e59b35）。读附录 C 时以本勘误为准。

## 已拍板结论（用户经 team-lead 确认，2026-11-11）

| 编号 | 拍板项 | 结论 |
|---|---|---|
| R4 | `/em/` CDN vendor 与否 | **不 vendor，维持 CDN 直连**（保留全部 SRI sha384 + crossorigin 依赖；接受 em/README.md:41「内网/离线环境会白屏」的已知限制） |
| nav | devicehub 的 nav 形态 | **新增 `devicehub` 第 10 项**（现 nav.ts 9 项无此 key；`devices` ≠ Kotlin `devicehub`） |
| R11 | 扩展工具范围 | **本轮只做 `models` / `diagnostics` / `compat` 三项**；`music` / `ble` 延后 |

> 以上为**实施裁剪依据**。下方 §0–§14 与补充勘误均为**研究原文全量保留**：§7 决策事实表、§13 实施清单仍按「全部实施」给全量清单且「只给事实、不替下结论」，拍板后按上表范围裁减即可（§13 中 `music`/`ble` 相关新增 API 方法、`ExtensionWorkspace` 的 music/ble 两个 Tab、`emEmbed.ts` 的 CDN vendor 分支按拍板裁剪）。附录 A–F 是本归档新补的逐端点 wire 契约，与原文结论互为印证；**如遇口径冲突，以本次落盘前逐行核读的源码 `file:line` 为准**（补正清单见附录 F）。

## 目录与合并顺序

| 区段 | 内容 | 来源 |
|---|---|---|
| §0–§7 | 服务端权威契约、`/em/` 静态资产链路、iframe URL 契约、端点证伪第七轮、R3/R4/R11 决策事实表 | 报告 1/2 原文 |
| §8–§14 | Kotlin 旧端页面结构与文案逐字、State 请求纪律、B 层门控 + 扩展工具 5 项、诚实化锚点 11 项、iframe/CORS/fail-closed/api 检查单、实施清单、绝对路径索引 | 报告 2/2 原文 |
| 补充勘误 | `/em/` 产物不进 `admin-web/dist`（R3/R4 更前置缺口） | 补充勘误原文 |
| 附录 A–F | 统一鉴权与错误形状、18 端点 echomuse 完整 wire、tuya 全方法完整 wire、`/em/` 运行时端点 + console WS 帧、计数口径澄清、逐行核读补正清单 | 本归档新增（落盘前逐行核读 admin.rs / tuya/api.rs / system.rs / ble_http.rs / oww_state.rs / oww.rs / console.rs / shell.rs / runtime.rs / lan.rs） |

---


---

> **以下为「报告 1/2」原文（§0–§7），全量保留，未删减。**

---

W12 / devicehub 页前置只读研究 · 报告 1/2（服务端权威契约 + 端点证伪第七轮 + 决策事实表）。全部只读，未改任何文件。证据均为 file:line。绝对路径索引在 2/2 §14。

═══════════════════════════════════════
§0 摘要与三处结构性证伪（先看）
═══════════════════════════════════════
1) **feature 门控不是 `native-plugins`**：`/api/echomuse/*`、`/api/tuya/*`、`/em/*` 三组路由分别由**独立** `#[cfg(feature="echomuse-plugin")]` / `#[cfg(feature="tuya-plugin")]` 装配（gateway/ws/mod.rs:272-279）。Cargo.toml:11 `default=[]`、:14 `native-plugins=["echomuse-plugin","tuya-plugin"]`（native-plugins 只是传递启用）。**dev-server.sh:31 用 `--no-default-features` ⇒ 本地 dev 是 pure-base，这三组路由根本不挂载，请求全落 SPA fallback 返回 HTML 200**（不是 404 JSON）。这是本波 fail-closed 形态与 W6/W7/W8/W10 同源但 feature 名不同，且 dev 联调会踩坑。
2) **`/em/` iframe 在旧端 Kotlin 侧「能力已备、零接线」**：`EchoMuseOfficialView`（EchoMuseOfficialView.kt:23）与 `openAdminUrl`（AdminPlatform.kt:81）在 commonMain **只有定义/expect，全仓 Grep 零业务调用点**；当前 Kuikly 管理台用 GearUI 原生六 Tab（EchoMuseOfficialEntry.kt）；admin-web EchomusePage.tsx 仍是占位骨架；nav.ts 9 项**无 `devicehub` key**。iframe URL 契约的权威只在 em/README.md + static/host.js（h5App web 侧），不是 Kotlin。
3) **计划描述与实际端点规模偏差**：`/api/echomuse/*` 远不止 admin.rs:37-53 的 7 主端点，另有 system(5)/oww(2)/console(1)/ble(3) 共 11 个从 merge 与独立路由挂入，共 **18 个 HTTP/WS 端点**；`/api/tuya/*` 是 **10 条 .route / 11 方法**（非 9）。BLE 是**服务端 HTTP 代理**不是客户端原生 GATT。

═══════════════════════════════════════
§1 `/em/` 静态资产链路（R4 决策事实，逐字节实测）
═══════════════════════════════════════
源目录 `client/apps/h5App/em/`（README.md:1-6：从上游 EchoMuse controller 原样 vendor，由 Rust 静态目录同源托管在 `/em/`，与 Kuikly 管理台是**两个独立文档**、全局变量零冲突）。逐文件字节（Bash wc -c 实测）：
- index.html 28,417；static/dashboard.jsx 648,322（上游源码逐字，**不下发**）；static/dashboard.js 440,653（esbuild 0.20.2 产物，入库，首行 `em-bundle src-sha256=` 横幅）；static/adapter.js 29,400；static/host.js 12,937
- README.md 12,059 / README.sha256 170 / LICENSE 1,066；tests/ 5 个 .cjs 共 47,445
- **源目录非 tests 合计 1,173,024 B ≈ 1.12 MiB**

下发产物 `client/apps/h5App/web/em/`（README.md:184-185：syncEmAssets 拷贝产物、已 gitignore、publishWeb 不清理旧文件）：实测 index.html 28,417 + static/{adapter.js 29,400, dashboard.js 440,653, host.js 12,937} + tests/ 5 cjs（字节与源目录略有差异，是稍旧拷贝）；**web/em 合计 558,743 B ≈ 545.6 KiB，真正运行时下发的 4 个文件合计 511,407 B ≈ 499.4 KiB**。
- `syncEmAssets` build.gradle.kts:98-104：`from(emDir) { exclude("README.md","README.sha256","LICENSE","static/dashboard.jsx") }` → into web/em（**tests/ 未被排除，实测确实被拷进 web/em/tests**）。
- `verifyEmBundle` build.gradle.kts:72-95：dashboard.js 首行 src-sha256 必须等于当前 dashboard.jsx 摘要、必须含 `em-host: App auto-mount removed`、adapter.js/host.js/dashboard.jsx 存在、**LICENSE 必须是文件**（:94）、「第三方库与字体走 CDN，index.html 里带 SRI，仓库不再 vendor」（:93）。`publishWeb` :106-109 dependsOn 两者。注意口径张力：:94 校验 LICENSE 存在，但 :102 下发时把 LICENSE **排除**——若 R4 决定 vendor，需连同许可随分发一并复核。
- README.sha256:1-2 两行：`src dashboard.jsx 2c0194…` / `out dashboard.js a64dea…`——是**源码/产物摘要**，不是 CDN vendor 清单（勿误读）。
- **Rust 托管**：gateway/ws/mod.rs:326-334 `ServeDir::new(&admin_dir).precompressed_gzip().not_found_service(ServeFile::new(index.html))` → `api.fallback_service(admin)`；SPA 未知路径回退 index.html。目录不存在 `api.fallback(admin_missing)`（:353-359 中文提示「管理页面未找到：请先构建 h5App（web）…」）。`[server].admin_dir` config.rs:217-218 默认 `../client/apps/h5App/web`；dev.rs:83-92 未设 `XIAOZHI_ADMIN_DIR` 时指向同目录。
- dev-server.sh:10 `WEB="${XIAOZHI_ADMIN_DIR:-$ROOT/client/apps/h5App/web}"`、:11 检查 index.html/nativevue2.js/h5App.js、:16 触发 `:apps:h5App:publishWeb`、:18 export、:31 `cargo run --no-default-features -- --dev`。
- **vite proxy 排除口径**：vite.config.ts:37-43 `/api`(ws:true)、`/ws`(ws:true)、`/em` → proxyTarget；admin-web/AGENTS.md:44-45「proxy 中间件先于 SPA history fallback，`/em/`（含尾斜杠）原样转发 Rust server，不会被 index.html 吞掉；前端路由不得占用 `/em/` 前缀；后续官方页用 `<iframe src="/em/?...">` 带尾斜杠 + URL 参数接入，**本阶段不实现**；微前端方向已否决」。README.md:182-183「链接必须带尾斜杠 `/em/`：Rust ServeDir 未知路径回退主站 index.html，否则在 `/em/` 下启动 Kuikly 二次初始化」。
- **index.html 加载链**（:559-591）：4 个 CDN `<script>` + 1 个 `<link>` 全部带 `integrity="sha384-…"` + `crossorigin="anonymous"` → 然后 `static/adapter.js` → `static/dashboard.js` → `static/host.js`。:6-7 `<meta name="referrer" content="no-referrer">`（token 在 URL 里）；:10-17 `@font-face` DM Sans/DM Mono 直链 fonts.gstatic（gstatic `/v17/`、`/v16/`，CSS url() 无 SRI 机制靠版本段）；:563-567 同步强制 `data-theme="light"`。
- **CDN 依赖清单（R4 决策核心）** README.md:27-41：React/ReactDOM 18.2.0 UMD production（cdnjs）、@xterm/xterm 5.5.0 js+css（jsdelivr）、@xterm/addon-fit 0.10.0（jsdelivr）、DM Sans/DM Mono woff2（gstatic v17/v16）。URL 与上游 controller/Dockerfile 逐字一致，JS/CSS 均 SRI 锁字节。**原文 :41「该页面运行时需要外网可达这两个 CDN（内网/离线环境会白屏）」**——这是 vendor 与否的唯一硬事实。许可 README.md:161-171：上游 dashboard MIT（Copyright 2026 Wil Bowes）、React/xterm MIT、字体 SIL OFL 1.1。

═══════════════════════════════════════
§2 `/em/` 运行时端点 + iframe URL 参数契约
═══════════════════════════════════════
`em_routes()` routes.rs:56-69 共 6 条：GET `/control`、`/data`（**上游固件现版拨的是根路径**）、`/em/control`、`/em/data`（本项目旧版别名，mock/文档走它，:59-60）、`/shell/{id}`、`/em/status`。
- **GET /em/status** routes.rs:71-75 / mod.rs:376-385：`{"enabled":bool,"devices":[{device_id,connected_secs}]}`，**无鉴权**（不含敏感信息）；:66-67 明确「关闭时也返回 200 + enabled=false，不像 WS 路由那样 404——该接口本身就是用来区分这两种情况」。devices 取 hub 当前 key（=处于会话中的在线设备）。
- **token_ok** routes.rs:77-83：校验 `X-Em-Token` 头，`[echomuse].token`（config.rs:41-44）为空则不校验。这是**设备面凭据**，与管理面 `auth_ok`（`[server].expected_token`/Bearer/`?token=`）是两套，勿混。
- **em_control** routes.rs:85-104：enabled 404 "echomuse disabled"；token 不匹配**不立即 401**（配对流程 `register.pairing:true` 降级拨号不带 token，由 register 首帧裁决）；control_session :107-310 三分支 refused/pending/approved（:144-179）、ack 能力协商 + config 推送（:181-197）、OWW 资产自动下发（:235）、wasm-policy（:257-283）。
- **em_data** routes.rs:453-498：enabled 404、token 401 `"bad token"`、首帧 identify 精确认领（:467-487，IDENTIFY_WAIT 5s :46）；data_loop :515-595 心跳 15s（:53）、控制帧不是断链信号（:561-566）。
- 协议背景 mod.rs:1-47：权威在 EchoMuse 仓库 `docs/device-controller-interface.md`；mDNS `_emcontroller._tcp.local`（spawn_mdns :525-567）；首版不做 TLS/wss（无 CA 设备拨 ws:// 明文）；ack.features 仅声明控制器能力子集、未声明即降级；刻意不做交互 shell/任意命令/OTA/音乐面 audio_mix/Sendspin。`[echomuse].plain_port` 默认 8767（config.rs:51-60，官方明文 controller 端口，>0 额外监听、主端口始终服务 /em/*）。

**iframe URL 参数契约**（权威 README.md:122-148 + host.js:16-23 + host.test.cjs）：
- 路径**必须带尾斜杠** `/em/`，参数顺序不限。三形态：独立列表 `/em/?token=t`、独立详情 `/em/?token=t&device=<encoded-id>`、嵌入单设备 `/em/?embed=1&device=<encoded-id>&token=<encoded-token>`（:122-124）。
- `embed`：仅精确值 `1` 启用（host.js:19）；缺省/`0`/`true` 保留独立模式（README.md:128、host.test.cjs:220-224）。
- `device`：`encodeURIComponent(id)` 编码一次，宿主 `URLSearchParams` 解码一次、适配器请求设备端点时重新编码；禁止把原始 `+`/`&`/`#` 直接拼入、禁止二次编码（README.md:129-131）。
- `token`：可选、`encodeURIComponent`；host.js:17 读取、:29 写入 `API.token`、authHeaders :70-72 `Authorization: Bearer <token>`；adapter.js:23 同源读取。URL 含凭据，`no-referrer` + iframe `referrerpolicy=no-referrer`（EchoMuseOfficialViewHost.kt:36）双保险。
- `tab`：可选，仅 `status`/`activity`/`config`/`shell`/`updates`/`logs`；`shell`→官方内部 `console`（host.js:23）；缺省/非法回退 `status`（README.md:134-138、host.test.cjs:128-129）；**未审批设备忽略 tab** 继续显示官方审批表单。
- 嵌入模式只读指定设备的记录 + `/runtime`（每 3s 轮询，host.js:119-145/:240-304），**不请求设备列表、不渲染全局计数/返回管理台链接/设备卡**（README.md:139-141）；缺少/空 `device` 直接显示错误且不发设备请求；读取失败显示内联错误并继续轮询同一 id，绝不回退列表。父容器/iframe 应提供明确高度（:147）。
- **iframe 容器已备**（jsMain/EchoMuseOfficialViewHost.kt:10-66）：真实 `<iframe>`、`sandbox="allow-scripts allow-same-origin allow-forms allow-downloads"`、无 `window.open`、dispose 时 `src="about:blank"` 后移除；commonMain/EchoMuseOfficialView.kt:25 平台门控 `if (!IS_MACOS && !pager.pageData.isWeb)` →「当前平台不支持内嵌官方管理页」，`key(url)` 销毁浏览上下文。**但两处均零调用点**（§0.2）。

═══════════════════════════════════════
§3 `/api/echomuse/*` 全端点权威（feature=echomuse-plugin，共 18 个）
═══════════════════════════════════════
`admin_routes()` admin.rs:37-53 = 7 主 `.route` + 3 merge；另有 `ble::ble_routes()` 在 gateway/ws/mod.rs:277 **独立 merge**（不在 admin_routes 内）。
【主 7】
- GET `/api/echomuse/devices` :40 → devices_list :380-393；体 `devices_payload` :90-100 = `{enabled, version, devices[]}`，**无 `ok` 键**（`version` 是设备面变化号，与管理台 WS hello/`echomuse_devices_changed` 同源，mod.rs:180-205）。
- GET+DELETE `/api/echomuse/devices/{id}` :41-44（device_get :395-408 / device_delete :410-428）；POST `/{id}/delete` :45 是别名（Kuikly NetworkModule 无 DELETE，同 OTA 惯例）。
- POST `/{id}/approve` :46 → :430-449；body `{"label"?}`（:120-127），认领与改名同一端点。
- POST `/{id}/command` :47 → :502-525；`command_out` :131-154 action 白名单 **wifi / wifi_scan / wifi_commit**（**reboot 明确拒绝，要求走 system/reboot**），在线 202 `{queued}` / 离线 409。
- POST `/{id}/settings` :48 → :451-500；`SETTINGS_KEYS` :182-236 **40 键白名单**（kind 语言：bool / string / json / float list / 枚举 / (0,1] / integer A..B / number A..B）+ 官方默认值逐字 :165-178；filter_settings :266-301；settings_plan :306-339 在线 202 / 离线 200 `{persisted,delivered}`；merge snapshot 推送 :474-485。
- POST `/{id}/music` :49 → :535-619；DLNA action `scan`/`browse`/`play_url`/`stop`，扫描浏览不依赖在线，play_url/stop 需在线否则 409 `"device offline"`；`[echomuse].music_dir` 路径约束（config.rs:67-71，留空=音乐面 404）。
【merge system::routes() system.rs:39-47，5】
- GET `/{id}/runtime` :42（get_runtime :82-101；`link` = authenticated + token_required，:94-99）
- POST `/{id}/volume` :43（set_volume :111-148；level 0..127、需 `speaker` 能力、202 `{queued:true,applied:false,level}`）
- GET `/{id}/system/info` :44（INFO 固定命令 :17-18，客户端不插值 shell）
- GET `/{id}/system/logs` :45（`log_command` 6 源白名单 server/messages/messages_last/last_kmsg_prev/supervisor/net :27-37，source 校验 :181-185）
- POST `/{id}/system/reboot` :46（REBOOT :21；`{"confirm":true}` 精确 :195-198/:210-212；202 `{dispatched:true,rebooted:false}`）
- `system_gate` :59-80 = admin::gate + approved_emos + 在线 + `live.authenticated` + `peer` 五重；注释 :49-57「局域网默认可管理」：`server.expected_token` 空=免 token，配了即收紧。
【merge oww::routes() oww.rs:123-132，2】POST `/{id}/oww/install-vad` :126-129 + GET `/{id}/oww/state`（oww_state.rs:29，五态聚合）；oww.rs:110-121 环境变量 GITHUB_PROXY / XIAOZHI_AUTO_DOWNLOAD_MODELS / 8MiB 下载上限。
【merge console::routes() console.rs:68-71，1】GET `/{id}/console`（WS，准入 `system_gate` 必须在 on_upgrade 之前跑完才能返回真 HTTP 状态码 :73-85）。
【ble::ble_routes() ble_http.rs:88-95，3】GET `/{id}/ble/adverts`、GET `/{id}/ble/state`、POST `/{id}/ble/gatt`（`DefaultBodyLimit::max(4096)` :94）；gate :97-108 = admin::gate + **`ble_proxy` 开关**（关闭 404）。
- **gate()** admin.rs:366-378：`auth_ok` 失败 → **401 纯文本 `"unauthorized"`**（:372，**非 JSON**，同 W10 OTA）；`enabled=false` → `disabled_404()` JSON `{"error":"echomuse disabled"}`（:75-80/:374-375）。`err_json` :84-86 统一 `{"error"}`。
- `DeviceRecord.payload` registry.rs:76-96 全字段：device_id/label/approved/version/board/base_os/capabilities/first_seen/last_seen/online/connected_secs/settings/ip/kernel_arch/kernel_release/boot_id/boot_reason；`record_payload` :109-117 另注入 `session_id`。落盘 `echomuse_devices.json`（:1-6、default_path :253-265 复用 `[memory].db_path` 父目录，与 Files root 同源）。

═══════════════════════════════════════
§4 `/api/tuya/*` 全端点权威（feature=tuya-plugin，10 条 .route / 11 方法）
═══════════════════════════════════════
`routes()` tuya/api.rs:47-63：GET `/status` :50；POST `/credentials` :51；POST `/credentials/delete` :52；**DELETE `/credentials` :53-56**（计划漏）；POST `/sync` :57；GET `/devices` :58（?online 过滤 :496-515）；GET `/devices/{id}` :59；POST `/devices/{id}/dp` :60；POST `/devices/{id}/alias` :61；POST `/cloud/proxy` :62。
- **gate** :350-362 = `auth_ok`（401 纯文本 `"unauthorized"` :356）+ tuya.enabled=false → `json_404` `{"error":"tuya disabled"}` :364-366。
- `status_payload` :133-164：`configured` / `access_id_masked` / `region` / `sync_interval_secs` / `syncing` / `last_sync{finished_at,age_ms,result,device_count,message}` / `lan_unregistered[{device_id,ip,age_ms}]` / `store_path`；**永不含 access_secret**（mod.rs:25-29 安全铁律「只写不读」「local_key 只落独立库 0600」）。`mask_access_id` :68-79 `ab****ef`（≤4 位→`****`）。
- `credentials_apply` :83-120：region 白名单（REGIONS `cn/us/eu/in` config.rs:29）+ access_id 必填 + **secret 留空且已配置时保留旧值**；`credentials_put` :435-458 校验后写库 + invalidate + 服务端文案「凭证已保存（secret 不再回显）」；`credentials_clear` :123-127；`credentials_delete` :460-478。
- `devices_payload` :167-183 **alias 优先于 name**；`device_detail_payload` :186-214 `dps[{dp,value,value_text,writable,control}]`；`value_text` :217-225 bool→开/关；`control_shape` :228-234 boolean→switch / number→number / 其余 readonly；`device_dp` :537-574 需 dp+value；`device_alias` :576-597 alias 可空串清除。
- `cloud_proxy_call` :261-320 method GET/POST/DELETE 白名单 + path 必须 `/v1.` 前缀 + query 字典序 + body 仅 POST；`cloud_proxy` :599-649 凭证未配置 400「涂鸦凭证未配置：先 POST /api/tuya/credentials 录入」+ 错误三归类 auth/network/api 502。
- **sync 60s 闸门** sync.rs:19 `MIN_SYNC_INTERVAL=60s`（手动与定时共用）；`sync_now` :41-67 busy + 间隔双闸门，拦下返回 `result="skipped"`（**不算失败**，UI 提示稍后再试）；四归类 ok/partial/cloud_auth_failed/network_error（:1-7、classify_cloud_error :273-294）；`execute_sync` :90-170 凭证未就绪 → cloud_auth_failed「涂鸦云凭证未配置（请先在管理台录入）」。`credentials_ready` config.rs:123-125（id+secret 都非空才算）。
- **lan_unregistered 语义** mod.rs:79-92：LAN 扫描（UDP 6666/6667/7000）发现的**未登记**设备内存快照，**不纳管、不落库**；只透出「局域网里有这么一台设备」到 status，等云同步写入 local_key 才进目录；`UNREGISTERED_MAX=32`（:92，按发现时间倒序丢最旧）；scan_and_merge :426-468（20s 超时）；note_unregistered :471-489；unregistered_snapshot :492-501。
- 轮询/发现周期 mod.rs:138-140 `POLL_INTERVAL=60s` / `SCAN_INTERVAL=300s`；连续 2 次失败才标离线（:388-421，单次抖动不翻转）。`config.rs` :52-55 sync_interval_secs 默认 86400、0=仅手动；normalize :88-115 **全部热生效**、region 白名单收敛、显式配的 <60 也夹到 60（:107-114，静默收敛不是 400）。

═══════════════════════════════════════
§5 feature 门控、dev.rs blocked 与 fail-closed 形态
═══════════════════════════════════════
- Cargo.toml:11 `default=[]`（:9-10 注释「默认构建就是干净的通用服务底座，不装配任何设备/业务插件」）；:14 `native-plugins=["echomuse-plugin","tuya-plugin"]`；:15-31 echomuse-plugin 依赖集（rupnp/symphonia 系列/zip）；:32 `tuya-plugin=["dep:rustuya"]`；:40 `esp_home=[\"…\", "echomuse-plugin"]`（反向依赖）；:46 `wasm-plugins=["dep:wasmtime"]`。
- gateway/ws/mod.rs:272-279：`#[cfg(feature="echomuse-plugin")]` merge em_routes + admin_routes + ble_routes；`#[cfg(feature="tuya-plugin")]` merge tuya::api::routes。**未编入=路由不存在**。
- **fail-closed 形态**：pure-base 下这些路径落 `api.fallback_service(ServeDir…not_found_service(ServeFile(index.html)))`（:326-334）→ **HTML 200**；目录不存在才 `admin_missing` 404 中文文本（:353-359）。与 W6/W7/W8/W10 的 `native-plugins` 门控 HTML 200 同形态，**但 feature 名是 echomuse-plugin/tuya-plugin**；`GET /em/status` 同样落 HTML 200（其「enabled=false 也 200」的语义只在 echomuse-plugin 编入时才成立）。
- 二次保险（口径不对称，需登记）：echomuse/config.rs:115-117 与 tuya/config.rs:92-94 的 `normalize()` 用 **`cfg!(not(feature="native-plugins"))`** 强制 `enabled=false`，而路由门控用 `echomuse-plugin`/`tuya-plugin`。`default_true()` config.rs:92-94 同。
- **dev.rs blocked** :97-105：`/api/ws`、`/api/test*`、`/api/nlu/model/build*`、`/control`、`/data`、`/em/control`、`/em/data`、`*/inject` → dev 模式下 401（未鉴权）或 503 `dev_inference_disabled`（已鉴权，:111-130）。**`/em/status`、`/em/` 静态页、全部 `/api/echomuse/*`、`/api/tuya/*` 不在 blocked**；但 dev 是 pure-base（dev-server.sh:31），这些 API 路由不存在 → HTML 200。guard :107-143 全局 layer 注入 `x-xiaozhi-mode: dev`。
- auth_ok 边界 mod.rs:377-396（`[server].expected_token` 空=不校验；设备 Bearer、浏览器 `?token=` 兜底）；管理台 WS RPC `admin_ws::set_rpc_router(app.clone())` :339 复用同一份路由表、同 404 语义；WS 握手鉴权 admin_ws.rs:400。
- C6 分工登记（admin-web/AGENTS.md:163）：Tuya 凭证唯一录入路径是 devicehub 的 `POST /api/tuya/credentials`（不经 PUT /api/config），Config C6 只暴露 enabled/sync_interval_secs、region/access_id/access_secret 一律不渲染，两套 UI 不互相覆盖。

═══════════════════════════════════════
§6 端点证伪第七轮清单（逐条对照计划）
═══════════════════════════════════════
1. 计划「admin.rs:37-53 全 auth_ok+enabled=false 404」→ 属实但**不完整**：另有 system(5)/oww(2)/console(1)/ble(3) 共 11 端点从 4 处 merge/独立路由挂入；且各 gate 不同（system_gate 额外 approved+在线+authenticated+peer；ble 额外 ble_proxy；console 准入先于 upgrade）。
2. 计划「BLE 归 B 层还是 HTTP」→ **HTTP**：ble_http.rs 3 端点（服务端经 0x06/0x08 帧代理直驱 GATT + 广播采集，mod.rs:29-35/42-45），非客户端原生 BLE；`ble_proxy` 默认 false（config.rs:45-50），关闭时 BLE 端点 404；协议要求 ack + register **双向声明**才启用。
3. 计划「/api/tuya/* 全 9 端点」→ 实际 **10 条 .route / 11 方法**（credentials 有 POST + DELETE 两路，另加 POST `/credentials/delete` 别名）。
4. 计划漏列 `GET /em/status` **无鉴权**、且关闭时仍 200+enabled=false（非 404）。
5. `/em/` 运行时 4 个 WS 路径中 `/control`、`/data` 是**上游固件根路径**，`/em/control`、`/em/data` 是本项目别名（routes.rs:59-60），不是「/em/ 前缀专属」。
6. `POST /api/echomuse/devices/{id}/command` **不支持 reboot**（admin.rs:131-154 明确拒绝，要求 system/reboot）；`wifi` 白名单仅 wifi/wifi_scan/wifi_commit。
7. `GET /api/echomuse/devices` 响应**无 `ok` 键**（`{enabled,version,devices}`），勿判 `ok===true`；`GET /api/tuya/status` 同样无 ok 键。
8. `credentials` 需区分三路：POST `/credentials`（写入/保留 secret）、POST `/credentials/delete`（别名）、DELETE `/credentials`。
9. **iframe URL 拼装在 Kotlin 侧零调用点**（§0.2）：`EchoMuseOfficialView`/`openAdminUrl` 仅定义；契约权威在 em/README.md + host.js；当前 Kuikly 用原生六 Tab、admin-web 用占位骨架。计划「EchoMuseOfficialEntry iframe URL 拼装」应更正为「EchoMuseOfficialEntry 是 GearUI 原生六 Tab 工作区」。
10. 扩展工具：代码 `EM_EXTENSION_TABS`（EchoMuseOfficialEntry.kt:30-33）是 **5 项**（music/ble/models/diagnostics/compat），而 em/README.md:176-177 写「六个原生扩展（DLNA 音乐/GATT 工具/VAD 资产/模型文件/WiFi 确认/原始诊断）」是**演进前旧文案**；:29 注释「官方已有的网络确认与资产安装不再重复设置入口」——以代码为准（2/2 §10 详列）。
11. `X-Em-Token`（`[echomuse].token`，设备面）与 `auth_ok`（`[server].expected_token`，管理面）**两套凭据**，勿混；`/em/status` 无鉴权，WS 端点 token_ok，管理面 auth_ok。
12. feature 门控是 `echomuse-plugin`/`tuya-plugin`（native-plugins 仅传递），而 normalize 用 `native-plugins` 判 enabled——口径不对称；dev-server.sh `--no-default-features` ⇒ dev 全 HTML 200。

═══════════════════════════════════════
§7 待拍板项决策事实表（只给事实，不替实施下结论）
═══════════════════════════════════════
【R4 · `/em/` CDN vendor 与否】
| 维度 | 不 vendor（现状） | vendor |
|---|---|---|
| 现状依据 | README.md:27-41、index.html:572-591、build.gradle.kts:93（校验注释明写「仓库不再 vendor」） | 需改 index.html 引用为本地路径 + 改 syncEmAssets exclude + 可能改 verifyEmBundle |
| 外部依赖 | React/ReactDOM 18.2.0 UMD(cdnjs)、@xterm/xterm 5.5.0 js+css(jsdelivr)、@xterm/addon-fit 0.10.0(jsdelivr)、DM Sans/DM Mono woff2(gstatic v17/v16) | 同 4 类需落地入库（体积未实测，按上游 vendor 体量估算数百 KB） |
| 完整性 | JS/CSS 全 SRI sha384 + crossorigin；字体 CSS url() 无 SRI 靠 /vNN/ 版本段；页面 no-referrer | 本地后无 SRI 需求，但需复核 MIT/OFL 许可随分发（LICENSE 当前被 :102 排除，与 :94 校验矛盾） |
| 离线可用性 | **内网/离线白屏**（README.md:41 原文） | 离线可渲染 |
| 体积 | 已下发 4 文件 511,407 B ≈ 499.4 KiB（不含 CDN 拉取） | + 第三方库数百 KB；源目录 1.12 MiB（含 648 KB jsx 源码） |
| 前端感知 | iframe 内 CDN 失败会白屏，外层只能靠 `onload`/定时器粗判，无法读到「CDN 失败」诚实原因 | 可在 iframe 内显式报错 |
| 与 dev 关系 | dev-server.sh:31 pure-base → `/em/` 静态页在但 `/api/echomuse/*` 全 HTML 200，adapter 会报错 | 同上，vendor 不解决 API 不可达 |
| 关联测试 | em/tests/*.cjs（host.test.cjs 校验官方 bundle 摘要；console.browser.cjs 覆盖 standalone/iframe 两模式） | 改 index.html 后需回归 host.test.cjs、build.gradle.kts verifyEmBundle |

【隐含 R3 · iframe 本波是否接入 / devicehub 形态】
- 已有能力：vite proxy `/em` 排除（vite.config.ts:42）、HashRouter 不占 `/em/`（AGENTS.md:45-46）、`withGlobalTauri:false`（:64）、EchoMuseOfficialView native view + EchoMuseOfficialViewHost iframe 容器（sandbox/no-referrer）、em/host.js 完整 embed/device/token/tab 契约与 5 个 .cjs 回归测试。
- 未接线：Kotlin commonMain 零调用点；admin-web EchomusePage.tsx 占位（:8/:12-13 明写「本阶段不实现 iframe」）；admin-web/AGENTS.md:198 待办第 3 项「`/em/` iframe 接入」、:199「Settings / Echomuse / devicehub / Testbench 仍待迁移」、:209 迁移顺序 `devicehub → Settings → Testbench`。
- **nav 结构缺口**：admin-web nav.ts 9 项（`overview/devices/config/settings/echomuse/console/files/plugins/testbench`），**无 `devicehub` key**；`devices`(W2 已迁移，数据源 `/api/devices` 小智协议注册表) 与 Kotlin `devicehub`（2026-11-03 合并原 EchoMuse/涂鸦两页，DeviceManagementCenter.kt:28-33）**不是同一页**。需拍板：新增 nav 项 / 复用 echomuse 项 / 合并进 devices。
- `router.tsx` 用 `visibleNavDescriptors(tauriEnv).map` 静态生成路由、`path="*"` 兜底（:37-52）；`tauriOnly` 仅 testbench=true（nav.ts:67）。

【R11 · 扩展工具范围】决策事实在 2/2 §10（5 项逐项依赖 + 各自服务端端点 + 演进差异），此处不重复。

—— 1/2 完；2/2 覆盖：Kotlin 页面结构与文案逐字、State 请求纪律、B 层门控、扩展工具清单、诚实化锚点、iframe 同源/CORS/IPC、fail-closed 与 api 两检查单落地、上传复用、W11 边界、实施清单、绝对路径索引。

---

> **以下为「报告 2/2」原文（§8–§14），全量保留，未删减。**

---

W12 / devicehub 页前置只读研究 · 报告 2/2（旧端 Kotlin 权威 + 扩展工具清单 + 诚实化锚点 + fail-closed/api 检查单 + 实施清单 + 绝对路径索引）。接 1/2。全部只读，未改任何文件。

═══════════════════════════════════════
§8 旧端 Kotlin 页面结构与文案逐字
═══════════════════════════════════════
【DeviceManagementCenter.kt 设备中心——侧栏唯一入口 `devicehub`】
:28-33 注释原文：「2026-11-03 由原 EchoMuse / 涂鸦设备两页合并」；形态「顶部 title 栏（摘要 + 扩展工具 + 刷新 + **齿轮一次性配置**）→ 设备卡片列表（EchoMuse 卡 + 涂鸦卡，同一套栅格断点）→ 点卡片弹各自详情（EchoMuse 六标签弹窗 / TuyaDeviceDialog DP 弹窗）。配置是**一次性**的：接入开关与凭据在齿轮弹层 / `openConfigSection` 跳转的唯一配置表单里，不在设备卡片里重复」；:35-36「配置卡尾部的『管理设备』按钮跳回本页；运行面全仓只此一份」。
- :39-49 入参 `EchoMuseState / TuyaState / PluginMetaState / ToastState / isActive / apiBase / token / openConfigSection / modelFiles`。
- :54-58 栅格：contentWidth 减 SIDEBAR_WIDTH、columns=(contentWidth+gutter)/(240+gutter)，cardWidth 均分（同一套断点）。
- :59-64 本地态 `extensions / selectedEmId / selectedTuyaId / gearOpen` + 从实时列表反查 selectedEm/selectedTuya。
- :66-71 **注册门控**：`devicePluginRegistered(plugins,"echomuse"/"tuya")` = `DevicePluginNav.showPluginPartition(plugins.devicePluginNav, id)`、:179-181 注释「旧服务端未上报该插件行时按已注册处理（维持合并前行为，不闪隐）」；`noneRegistered = !DevicePluginNav.showDeviceHub(...)`。（**与 W11 边界见 §12**）
- :75-82 DisposableEffect：`echoMuse.ensureLiveUpdates(pager){active}`；onDispose 退订 + `echoMuse.detail.leave()` + `tuya.closeDetail()`。:84-86 首次进入补拉涂鸦目录（`!tuya.loaded && !tuya.loading`）。:87-93 选中设备从实时列表反查，列表里没了（被删/换令牌）就收起弹窗。
- :97-125 title 栏逐字：固定标题「设备管理」（:105）；摘要 :109 `if (extensions) "自定义扩展 · 选择设备" else deviceHubSummary(...)`；按钮 :114「扩展工具」/「返回设备管理」（切换时清空两个 selected + `echoMuse.detail.leave()`）；:120 刷新按钮（loading=`echoMuse.loading||tuya.loading`，onClick 双 refresh）；:124 齿轮 `IconToken.Settings` contentDescription「设备接入配置」。
- `deviceHubSummary` :183-188：`if (双 loading) "加载中…"` else `EchoMuse N 台 · 涂鸦 M 台 · 在线 X`（注释「涂鸦未配置/未启用时如实说，不冒充 0 台」）。
- :131-138 两路加载失败就地 **Alert DANGER**：「EchoMuse 设备列表加载失败」/「涂鸦设备加载失败」+ description=`loadError`；:139「正在读取 EchoMuse 设备列表…」。
- :140-146 `noneRegistered` EmptyState：「没有已注册的设备插件」/「EchoMuse 与涂鸦设备都是插件：在「EchoMuse 接入」或「Tuya 接入」配置开启（注册）后，对应设备分区与管理入口会出现在这里。」
- :147-158 扩展模式只留 EchoMuse 卡（涂鸦区隐藏，`!extensions && tuyaRegistered`）；:157 onSync 回调 `toast.show(msg, if(ok) "ok" else "error")`。
- :190-199 `DeviceHubSectionLabel`：标题 + 「N 台」+ 徽标「降级」(phase=degraded)/「异常」(failed)，**正常/在线不渲染徽标**；:211 `registration.reason` 非空用 EmNote 透出。
- :212-227 EchoMuse 分区逐字：「EchoMuse 未启用。到「EchoMuse 接入」配置开启设备入口并重启服务端。」+「打开接入配置」；空态「还没有 EchoMuse 设备登记」/「设备首次连上服务端后会出现在这里，经批准后即可管理。」
- :240-266 涂鸦分区逐字：「涂鸦设备未启用。到「Tuya 接入」配置开启（保存即生效，无需重启）。」+「打开接入配置」；「凭证未配置：点右上角齿轮录入涂鸦云凭证，或到「Tuya 接入」配置。」；空态「还没有涂鸦设备」/「录入凭证后点「立即同步」拉取账号下的设备目录；控制全部走服务端局域网直连。」+ actionText「立即同步」；:260-265「设备目录存储：${tuya.storePath}」。
- :162-176 三个弹窗挂载：`EchoMuseManagementDialog(row = selectedEm.takeIf{ active && echoMuse.enabled })` / `TuyaDeviceDialog(row = selectedTuya.takeIf{ active && tuya.enabled && !extensions })` / `DeviceAccessConfigDialog(visible = gearOpen && active)`。
- :283-290 齿轮弹层注释：「EchoMuse 只放状态与『打开接入配置』跳转（开关/令牌/欢迎语在语音流程的接入节点，唯一配置表单不复制第二份）；Tuya 凭证表单就在这里（`POST /api/tuya/credentials` 是唯一录入路径）+ 立即同步 +『Tuya 接入』配置跳转（开关/同步周期在全局配置）」。
- :351-365 齿轮内容逐字：「设备接入配置」/「关闭设备接入配置」/「一次性配置：接入开关与凭据只在这里和配置页维护；设备列表用顶部刷新按钮更新。」
- :373-390 EchoMuse 块：「EchoMuse 接入」/ 状态「已启用」|「未启用」/「开关 / 接入令牌 / 欢迎语在「语音流程 → 终端与音频入口 → EchoMuse 接入」节点编辑（重启生效）。」/「打开接入配置」（onClick 先 onDismiss 再 `openConfigSection("echomuse")`）。
- :392-437 Tuya 块：「涂鸦设备」/ 状态「未启用」|「已配置 · ${accessIdMasked} · ${region}」|「凭证未配置」（:399-404）/ `TuyaCredentialsCard` /「同步中…」|「立即同步」（`loading/disabled=tuya.syncing`，:415-420）/「打开接入配置」/ `tuya.lastSyncText` / 「涂鸦设备未启用：/api/tuya/* 全部 404（凭证录入同样不可用）。先到「Tuya 接入」开启（保存即生效），再回来录入凭证。」（:431）/「没有已注册的设备插件：…」（:440）。
- 弹窗 overlay 选项（:58-64、:311-318）：Center + modal + `dismissPolicy` outsideClick=false（**点遮罩不关闭**，对齐 W10 删除弹层）。

【EchoMuseOfficialEntry.kt —— 是 GearUI 原生六 Tab 工作区，不是 iframe】
:23 注释「单一 Kuikly 管理工作区；沿用官方分区顺序与布局，不再嵌入英文官方文档」。
- :24-27 `EM_MANAGEMENT_TABS` = 状态 / 活动 / 配置 / 控制台 / 更新 / 日志。
- :29-33 `EM_EXTENSION_TABS` = **DLNA 音乐 / GATT 工具 / 模型文件 / 原始诊断 / 设备兼容**（5 项，注释「官方已有的网络确认与资产安装不再重复设置入口」）。
- :37-87 `EchoMuseManagementDialog`：DisposableEffect(id,apiBase,token,extensions) 起 overlay；:66 注释「null 时立即卸载文档，不等 overlay 退出动画，防止隐藏控制台占设备 shell」；onDispose `state.detail.leave()` + close。
- :100 `tabs = if (extensions) EM_EXTENSION_TABS else EM_MANAGEMENT_TABS`；:101 tab 初值 `tabs.first().id`；:106 `label = if (tab=="compat") EM_TAB_CONFIG else tabs.first{it.id==tab}.label`。
- :107-110 DisposableEffect：`if (row.approved) state.enterTab(row,label,pager)`、onDispose `state.detail.pauseTab()`；:111 SideEffect `state.detail.sync(listOf(row))`。
- :112-113 Card 尺寸：width `(viewport.width*0.95f).coerceIn(1f,900f)` / height `(viewport.height*0.90f).coerceIn(1f,700f)`。
- :115-118 `EchoMuseDeviceHeader`（重命名/删除/审批在此）+ extensions 时「自定义扩展」标题。
- :119-123 Tabs：**仅 `row.approved` 才渲染**；extensions 用 UNDERLINE、否则 CARD outline。
- :124-142 when 分支逐项：`!row.approved`→`EchoMuseApprovalTab`；`shell`→`EchoMuseConsoleTab`；`config|compat`→`EchoMuseNativeConfigTab(compatibilityOnly = tab=="compat")`；`updates`→`EchoMuseNativeUpdatesTab`；`logs`→`EchoMuseNativeLogsTab`；`status|activity`→ScrollView + `EchoMuseNativeStatusTab`/`EchoMuseNativeActivityTab`；`models`→`modelFiles()`；else→`EchoMuseExtensionContent(row,state,toast,tab)`。

【涂鸦详情/凭证逐字】
- `TuyaPage.kt:36-118 TuyaCredentialsCard`：:52「涂鸦云凭证」；:58「已配置 · ${accessIdMasked} · ${region}」|「未配置」；:63-72「区域」下拉 4 项「中国（cn）」「美洲（us）」「欧洲（eu）」「印度（in）」；:74-79「access_id」placeholder「涂鸦 IoT 平台云开发项目的 access_id」；:80-86 已配置提示「access_secret 已配置（不再回显）。留空保存 = 保留旧值；更换请输入新值。」；:87-92 secret 标签「access_secret（留空保留旧值）」|「access_secret」+ placeholder「涂鸦 IoT 平台云开发项目的 access_secret」；:95「保存中…」|「保存凭证」；:101-115 清除走**行内二次确认**「确认清除？」/「取消」/「清除凭证」。
- `TuyaPage.kt:125-176 TuyaDeviceCard`：name 空回退 deviceId、「涂鸦」徽标、`displayModel`（空→「型号未知」，TuyaState.kt:433）、「在线」|「离线」、`ip` 空→「IP 未上报」。
- `TuyaDetailPage.kt:89-100` 本地别名卡：「本地别名（仅本管理台展示，不影响云端名称）」/「保存别名」。
- `TuyaDeviceDialog.kt:20-28` 弹窗含「设备信息 / 本地别名 / DP 控制」。

═══════════════════════════════════════
§9 State 请求纪律与逐字文案
═══════════════════════════════════════
【EchoMuseState.kt】
- :10-35 类注释：数据源 `GET /api/echomuse/devices` + 批准/下发指令/删除记录；`enabled=false` 显示「未启用」提示而不是空列表；列表是**请求时刻快照但不需要轮询**——设备面变化（上线/离线/批准/设置/删除）经管理台 WS `GET /api/admin/ws` 推 `echomuse_devices_changed`，随即刷新（:16-19）；加载失败就地显示 loadError + 刷新不弹 toast；写动作结果走顶部 toast；409 设备离线单独「设备离线」提示。
- :43-44 `base` + `connectionGeneration`；:72-99 `configureAccess(token, baseUrl)`：token 或 base 变化即 generation++、清全部状态、`liveSub.close()`；:97 `detail.configureConnection` / `ble.configureConnection`。
- :101-116 `load`：`getJson(echoMuseAuthenticatedUrl("$base/api/echomuse/devices", accessToken))`——**GET 把 token 编进查询串**；失败「EchoMuse 设备列表加载失败：$reason」；成功 `apply(data)` + `modelFilesRefresh`（:112-113 注释「顺带拉一次 oww-models/ 自定义模型清单，失败静默」）。
- :120-159 `apply` 全字段解析（device_id/label/approved/version/board/base_os/first_seen/last_seen/online/connected_secs/settings/session_id/ip/kernel_arch/kernel_release/boot_id/boot_reason/capabilities）+ `detail.sync` / `logs.syncDevice` / `ble.sync`。
- :162-168 `summaryText`：「EchoMuse 未启用」|「共 N 台 · M 台在线」。
- :170-196 自定义唤醒词模型：`GET /api/files?path=oww-models` 收 `.onnx` 文件名，**刻意安静**（失败不置 loadError、不弹 toast）。
- :202-224 `enterTab`（六 tab 唯一入口）：`refill = detail.target!=id || detail.tab!=label`，同设备同 tab 重复点**不重填草稿**；进配置 tab 时 `ble.select(row)`。
- :232-260 `approve`：body `{"label":…}`；成功「已批准 ${displayName}」；404「设备不存在（列表可能已过期，请刷新后重试）」；else「批准失败：…」。
- :270 `actionBusy` 互斥锁（批准/重启/WiFi/音乐共用）。
- :276-296 `command`：成功**「WiFi 指令已排队下发，执行结果请在设备上核对」**；409「设备离线，指令未下发：请先唤醒设备再试」；404「设备不存在（列表可能已过期，请刷新后重试）」；else「指令下发失败：…」。
- :299-317 `sendWifi`：空 SSID「请先填写 WiFi 名称（SSID）」；成功置 `wifiPending=true`。
- :330-345 `delete`：成功「已删除记录 …」；404「设备不存在（可能已被删除，刷新列表即可）」。
- :358-387 `saveConfig`：body 由 `drafts.buildConfigBody` 组装「**校验失败一个字都不发**」；空 body「没有需要保存的修改」；`delivered|queued`→「配置已保存并排队下发，实际生效请在设备上核对」；否则「配置已保存，设备下次接入时自动推送」；400「配置未保存：字段不合法（不在白名单或形状不对）」；409「配置冲突，未保存的输入已保留；请核对后重新加载设备设置：…」。
- :393-444 WiFi 闭环 `wifi_scan`→`runtime`→`wifi_commit`：:431「扫描指令已下发，稍候点击「读取扫描结果」查看周边网络」；:442「已确认新网络；设备将固定使用新网接入」；`wifiPending` 注释 :404-408「**既不能说成功也不能说失败**：设备可能正在重连、也可能已经回滚到旧网」。
- :490-530 `ensureLiveUpdates`：订阅 `GET /api/admin/ws`，`echomuse_devices_changed`→refresh；**无轮询、无长挂**；AdminRealtime 退避 3s→30s、25s 应用层心跳；「订阅失败/平台无 WS 能力自动退化为『进页刷新一次』」。
- :536-552 `post` 走 WS RPC `rpcCall`（鉴权在连接升级时完成，无需逐请求头）+ generation 防迟到。

【TuyaState.kt】
- :10-29 类注释：目录来自云同步、控制走服务端纯局域网（UI 只见三归类错误与建议动作）；`enabled=false` 全部 404 显示「未启用」；**凭证铁律 secret 永不回显**（成功保存后立即清空 secret 草稿，:17-21）；请求三重凭证 generation + base + adminAccessRevision 防迟到（:23-26）。
- :105-155 `load`：`GET /api/tuya/status`；失败「涂鸦状态加载失败：…」；`last_sync` 为空「从未同步」；否则「最近同步：${resultLabel(result)} · N 台 · ${humanAge(age)}」（`age_ms` **服务端算好**，客户端不依赖本机时钟）；解析 `lan_unregistered[]`；成功后同票据递进 `loadDevices`（避免两次全量竞态）。
- :159-187 `loadDevices`：`GET /api/tuya/devices`；失败「设备目录加载失败：…」。
- :189-193 `summaryText`：「涂鸦设备未启用」|「凭证未配置 · N 台」|「共 N 台 · M 台在线」。
- :200-229 `saveCredentials`：空 id「请先填写 access_id」；未配置时空 secret「请填写 access_secret（已配置过可留空保留旧值）」；成功「凭证已保存（secret 不再回显）」+ 清空草稿 + load；失败「保存失败：…」。
- :232-247 `clearCredentials`：成功「凭证已清除」；失败「清除失败：…」。
- :250-270 `syncNow`：注释「60s 闸门在服务端；skipped = 拦下，不算失败」；`onDone(result != "skipped", msg)`——**skipped 不报成功但也不算失败**；成功后 load 刷新。
- :277-301 详情：`GET /api/tuya/devices/{urlComponent(id)}`；失败「详情读取失败」；`applyDps` 解析 `dps[{dp,value_text,writable,control}]`（control 空→readonly）。
- :336-367 `setDp`：成功「已下发 DP $dp」/ dpMessage「已下发」；失败透出 `kind`/`error`/`advice`，`classifyFailure`。
- :370-386 `saveAlias`：成功「别名已保存」；失败「保存失败：…」。
- :396-411 `request` 走管理台 WS RPC + ticket + `adminAccessRevision` 双校验；:413-421 `urlComponent` 手写兜底编码（commonMain 无 URLEncoder）。
- :455-463 `resultLabel`：ok→「成功」/ partial→「部分成功」/ cloud_auth_failed→「凭证失败」/ network_error→「网络失败」/ skipped→「已跳过」/ else 原样。
- :465-471 `classifyFailure`：offline→「设备离线：…」/ timeout→「设备无响应：…」/ unsupported→「协议不支持：…」（advice 括注）。

═══════════════════════════════════════
§10 B 层门控 + 扩展工具清单（R11 决策事实）
═══════════════════════════════════════
【平台门控 / tauriOnly】
- admin-web `nav.ts:3` 注释「`tauriOnly` 与现有 Kuikly `NavDescriptor.macOnly` 同构」、:11「仅在 Tauri 桌面环境可见」；**9 项中仅 `testbench` 为 true**（:67）；:72-74 `visibleNavDescriptors(tauriEnv)` 过滤；`router.tsx:30` `isTauri()` + :37 按描述符静态生成路由。
- Kotlin：`IS_MACOS`（AdminShell.kt:135/157、ConfigPage.kt:137、AdminPage.kt:245）；`pager.pageData.isWeb`（EchoMuseOfficialView.kt:25、EchoMuseConsoleTab.kt:25）→ 非 macOS 非 web 显示「当前平台不支持内嵌官方管理页」/「请使用 Web 管理台或 macOS 客户端。」（EchoMuseOfficialView.kt:27-28）。
- `openAdminUrl`（AdminPlatform.kt:72-81）：「Kuikly 的 RouterModule 只能打开 @Page，打不开纯静态页；web 用 window.open，其余平台返回 false——该入口只在 web 侧有意义」；各 actual：jsMain/AdminPlatform.kt:33 window.open，iosMain/androidMain/macosArm64Main 均 `false`。**commonMain 零业务调用点**（1/2 §0.2）。
- **mDNS 双语境**：服务端 `spawn_mdns`（echomuse/mod.rs:525-567）是**服务端广播** `_emcontroller._tcp.local`（`[echomuse].enabled=true` 时由 main 调、plain_port 8767）；客户端 mDNS 发现是 macOS 原生平台 hook（MacServerSwitch.kt:35-36/:92/:313）；admin-web/AGENTS.md:211「mDNS 发现与服务器地址配置页：本轮未实现」（局域网需显式 `XIAOZHI_ADMIN_URL`）。
- **BLE 是服务端 HTTP 代理不是客户端原生**（1/2 §6.2）：`ble_proxy` 默认 false（echomuse/config.rs:45-50）、ack+register 双向声明（mod.rs:29-35/42-45）；Kotlin EchoMuseBleState.kt:130-133 文案「设备离线：旧广播/GATT/事件已清除；请唤醒设备并刷新列表」/「每设备 BLE 开关尚未同时开启；可读取广播/状态，GATT 不可用」；:333 45s 串行 + 47s 服务端超时；:350-356 状态码逐字（404「设备/BLE接口不存在：请开启 echomuse.enabled 与 ble_proxy 并确认服务端版本」/409「会话已变化、设备离线、BLE设置未启用或data链路不可用；刷新列表后重新操作」/429「设备已有 BLE 请求正在执行，请等待后重试」/503「BLE 代理控制连接不可用，请唤醒设备后重试」/504「BLE请求超时，未确认执行结果；先查询槽位/读回，勿盲目重复写入」）；AGENTS.md:134「服务端 ble_proxy 与双 token 是额外门控，不能由页面开启」、:138「结果只说明已保存/在线推送排队，不假定固件已应用」。

【扩展工具 5 项（以 `EM_EXTENSION_TABS` 代码为准，逐项依赖）】
| # | tab id / 标签 | 渲染 | 服务端端点 | 依赖/门控 |
|---|---|---|---|---|
| 1 | `music`「DLNA 音乐」 | EchoMuseExtensionContent.kt:25 `EchoMuseMusicStage` + EchoMuseMusicState / EchoMuseMusicPanel.kt | `POST /api/echomuse/devices/{id}/music`（scan/browse/play_url/stop，admin.rs:535-619） | `[echomuse].music_dir` 非空；play_url/stop 需设备在线否则 409；扫描不依赖在线 |
| 2 | `ble`「GATT 工具」 | :26 `EchoMuseBlePanel(showSettings=false)` + EchoMuseBleState；:20-22 进 tab 时 `state.ble.select(row)` | `GET/POST /api/echomuse/devices/{id}/ble/{adverts,state,gatt}`（ble_http.rs:88-95，gatt 4096 body limit） | `ble_proxy=true` + 双向声明 + 已批准 + 在线 + 每设备 bleProxyEnabled/bleProxyConnections + 非空 session；`session_id` 严格核对（AGENTS.md:99/:134） |
| 3 | `models`「模型文件」 | EchoMuseOfficialEntry.kt:138 `modelFiles()`（由 DeviceManagementCenter 传入）**复用 FilesPage**，默认 `oww-models/` | `GET /api/files?path=oww-models`（EchoMuseState.kt:182-195）+ W2 Files upload（64MB、XHR onprogress） | 无新增端点；**「上传不等于安装」**（AGENTS.md:118）；失败静默 |
| 4 | `diagnostics`「原始诊断」 | :27 `EchoMuseRawDiagnostics`（EchoMuseExtensionContent.kt:34-68） | `GET …/runtime`、`GET …/system/info`、`POST …/system/reboot` | 系统面 `system_gate` 五重；二次确认见下 |
| 5 | `compat`「设备兼容」 | EchoMuseOfficialEntry.kt:128-129 `EchoMuseNativeConfigTab(compatibilityOnly=true)`；label 用 `EM_TAB_CONFIG`（:106） | 复用 settings 白名单（只读/兼容视角） | 复用配置草稿，非独立端点 |
- 诊断逐字（EchoMuseExtensionContent.kt:40-67）：「读取设备报告」/「读取原始系统信息」/「复制诊断内容」（失败「复制失败：剪贴板不可用」、成功「诊断内容已复制」）；分组标签「当前会话最近报告」|「历史设备报告」、「历史系统信息」|「原始系统信息」；「重启设备」二次确认：**「确认重启设备？」/「当前连接与语音会中断。请求发送不代表重启已完成。」/「发送重启请求」/「取消」、`destructive=true`、`dismissOnOutside=false`**；:67「自定义诊断包含原始设备与网络信息，请仅向可信人员分享。」
- **演进差异（供 R11 拍板）**：em/README.md:176-177 写「六个原生扩展混排：DLNA 音乐 / GATT 工具 / VAD 资产 / 模型文件 / WiFi 确认 / 原始诊断」是**演进前旧文案**；:29 注释「官方已有的网络确认与资产安装不再重复设置入口」、AGENTS.md:118「资产与网络确认仅留官方对应位置，不在扩展重复」——现代码 5 项（VAD 资产 / WiFi 确认已并入官方页、新增「设备兼容」）。以 `EM_EXTENSION_TABS` 为准。
- 扩展模式交互（DeviceManagementCenter.kt:59/:113-119/:147-153）：按钮切换 `extensions`；扩展模式隐藏涂鸦区、只留 EchoMuse 卡、摘要变「自定义扩展 · 选择设备」、Tab outline 用 UNDERLINE。
- 官方六 Tab 对应原生实现文件：EchoMuseNativeStatusTab.kt / EchoMuseNativeActivityTab（在 NativeStatusTab 内）/ EchoMuseNativeConfigTab.kt / EchoMuseConsoleTab.kt / EchoMuseNativeUpdatesTab.kt / EchoMuseNativeLogsTab.kt（:24-27、:126-137）。

═══════════════════════════════════════
§11 诚实化锚点（11 项，UI 不得伪装）
═══════════════════════════════════════
1. **设备离线**：EchoMuseState.kt:290「设备离线，指令未下发：请先唤醒设备再试」；TuyaState.kt:467「设备离线：…」；music play_url/stop 409 `"device offline"`；setDp 离线预检（tuya/mod.rs:248-251）。
2. **credentials_ready false**：TuyaState.kt:191「凭证未配置 · N 台」；DeviceManagementCenter.kt:247「凭证未配置：点右上角齿轮录入涂鸦云凭证，或到「Tuya 接入」配置。」；sync.rs:118-127「涂鸦云凭证未配置（请先在管理台录入）」；cloud_proxy 400「涂鸦凭证未配置：先 POST /api/tuya/credentials 录入」。
3. **tuya disabled 404**：DeviceManagementCenter.kt:431「涂鸦设备未启用：/api/tuya/* 全部 404（凭证录入同样不可用）。先到「Tuya 接入」开启（保存即生效），再回来录入凭证。」；服务端 gate `{"error":"tuya disabled"}`。
4. **echomuse enabled=false（RestartOnly）**：DeviceManagementCenter.kt:213「EchoMuse 未启用。到「EchoMuse 接入」配置开启设备入口并重启服务端。」；em/host.js:99「服务端未启用 EchoMuse：配置里把 [echomuse].enabled 设为 true 后重启」。
5. **sync 60s 闸门**：TuyaState.kt:264 `onDone(result != "skipped", msg)`；`resultLabel("skipped")`=「已跳过」；服务端消息「距上次同步不足 60s，请稍后再试」/「已有一次同步在进行中」（sync.rs:44-55）——**skipped 不得报成功**。
6. **iframe 加载失败 / 断网 / CDN 白屏**：em/README.md:41「运行时需要外网可达这两个 CDN（内网/离线环境会白屏）」；host.js:98-101 enabled=false 分支；:272-280「Missing device parameter.」/「Unable to read device: …」/「Reading device…」；adapter.js 未实现动作统一 **501 + 中文可读说明**（README.md:78-81，上游每个失败路径都会 alert/setXxxError，不会静默）。
7. **X-Em-Token 未配置**：echomuse/config.rs:41-44 空=不校验；ConfigEchoMuseCard.kt:46「同网段设备接入凭据（X-Em-Token）。留空 = 不校验；留空保存不会覆盖已设置的值。」；system.rs:49-57「局域网默认可管理」；`/runtime.link` 只有 authenticated/token_required、**无设备侧 TLS 位**（README.md:99）。
8. **未批准设备**：EchoMuseOfficialEntry.kt:125 `!row.approved → EchoMuseApprovalTab`（不渲染六 Tab）；README.md:138「未审批设备忽略 tab，继续显示官方审批表单」；:142-144 审批后刷新进 status tab、删除后显示不存在错误不跳全局列表。
9. **排队 ≠ 应用**：EchoMuseState.kt:287「已排队下发，执行结果请在设备上核对」；system.rs:146 `{queued:true,applied:false,level}`；reboot 202 `{dispatched:true,rebooted:false}`（system.rs:19-21 注释「确认调度 ≠ 已完成」）；EchoMuseState.kt:369-373 两档配置文案；AGENTS.md:138「不假定固件已应用」。
10. **已知显示偏差（前端不编造，em/README.md:92-107 十项）**：Voice assistant 行恒红 `No satellite server`（与真实链路健康无关）、Link 行靠 http/https 推断、`Flash wear`/`Last boot`/`health`/`rttMs`/`linkQuality` 显示「—」、无 Pair 按钮、Turns 恒 0、release 字段「—」、日志时间是合成时间、配置段恒 `Local override (7 of 7)`、唤醒词下拉只有内置项、白名单外键被丢弃且界面无提示；:109-112 配置保存走**增量下发**（否则改一个开关会把其余 30+ 键按 fleet 默认值重刷）。
11. **wifiPending 不确定态**：EchoMuseState.kt:404-408「既不能说成功也不能说失败：设备可能正在重连、也可能已经回滚到旧网」。

═══════════════════════════════════════
§12 iframe 同源/CORS/IPC、fail-closed 形态、api 两检查单落地、上传通道、W11 边界
═══════════════════════════════════════
【同源 / CORS / IPC】
- admin-web/AGENTS.md:15「Rust server 当前无 CORS 层，因此开发/生产都依靠同源访问」；:51「纯单二进制、同源托管静态页与 `/api`、`/ws`、`/em/`，没有 CORS 层」；`api.ts` 默认相对 `/api`；生产默认「加载服务器页面」方案 B（:56，全可达无 CORS）；`XIAOZHI_ADMIN_EMBEDDED=1` 内嵌 SPA 预览 **API 不可达**（:57/:43，仅 UI 预览）。⇒ `/em/` iframe 同源、无 CORS 问题。
- `app.withGlobalTauri:false`（:64）「**不给 /em/ iframe 注入任何 IPC 能力**」；:63 远程 origin 下 init script 仍无条件注入 `isTauri`（与 withGlobalTauri 无关）；:99-105 `remote.urls` 二选一明确**不配置**（URLPattern 无法安全限定局域网 IP，`http://*:8000` 等价于任意 HTTP 源）；生产服务器页面是 Remote origin → IPC 被 ACL 拒绝 → 剪贴板走 `navigator.clipboard` + `execCommand` 三级降级（:95-97、copyText 返回 boolean 不抛）。iframe 不依赖任何 IPC。
- iframe 容器 sandbox（jsMain/EchoMuseOfficialViewHost.kt:38）`allow-scripts allow-same-origin allow-forms allow-downloads`、:36 `referrerpolicy=no-referrer`、:10「Never window.open」；em/index.html:7 `no-referrer`（token 在 URL）。

【fail-closed 形态（与 W11 相反，务必勿套用）】
- W11：`GET /api/plugins` 是**无条件基础路由永远 JSON**（判 `ok===true`+形状，wasm 键缺失→静态基线 fail-open）。
- W12：**所有** `/api/echomuse/*`、`/api/tuya/*`、`/em/status` 都是 `echomuse-plugin`/`tuya-plugin` 门控路由，pure-base（default=[]，dev-server.sh:31 `--no-default-features`）下整组**不挂载** → `api.fallback_service(ServeDir…ServeFile(index.html))` → **HTML 200**；目录不存在才 404 中文文本。⇒ 本波须按「HTML 200 → UnavailableError」归一，**不能复用 W11 清单 JSON 恒 200 口径**。
- 特例：`GET /em/status` 在 feature 编入时 **无鉴权且 enabled=false 仍 200 JSON**（routes.rs:66-67）；`/api/echomuse/devices` 与 `/api/tuya/status` 响应**无 `ok` 键**。

【api 两检查单落地（AGENTS.md:165 W6/W8/W10/W11 联合沉淀）】
- ① 门控端点（feature-gate、SPA fallback 会接走）的 catch 链必须**同时**覆盖两条：`SyntaxError → UnavailableError`（`requestApi` 对 HTML 200 抛裸 SyntaxError，api.ts:141）与「JSON 解析成功但形状不符 → UnavailableError」；模板 `soulMemoryApi.ts:82-114` / `otaApi.ts:119-126`。
- ② 混合鉴权先逐 handler 列清单再决定 401 分支。本波逐 handler 清单：
  · **`auth_ok` + enabled 闸门**（401 **纯文本** `"unauthorized"`、404 JSON `{"error":"echomuse disabled"}` / `{"error":"tuya disabled"}`）：全部 `/api/echomuse/*`（admin 7 + system 5 + oww 2 + console 1 + ble 3）、全部 `/api/tuya/*`（10 条）。⇒ 这些方法写 401 分支，按 `status===401` 区分（**不要当 JSON 解析**），Bearer 由 `requestApi` 自动注入。
  · **无鉴权**：`GET /em/status`（头注释**显式声明**「不写 401/token 分支」）。
  · **设备面 `X-Em-Token`**（非 auth_ok）：4 个 WS 端点 + `/shell/{id}`——**不拼管理台 token、不写 Bearer**；管理台不直接调这些端点（它们由固件拨号）。
  · 一律**判业务形状不判 200**：devices 判 `enabled`+`devices[]`+`version`；tuya status 判 `configured`/`access_id_masked`/`lan_unregistered`/`last_sync`；sync 判 `result`/`message` 且 `skipped` 不报成功；dp/music 判 `dps[]`/`queued`/`delivered`；approve/command/delete 判 `{queued}`/`{persisted,delivered}`/HTTP 码。
  · 错误体形状统一 `{"error":…}`（err_json/json_404/failure_payload），400 也是 JSON；sync/dp/music 回包的 `message`/`advice`/`kind` 须透出中文原文（不吞、不英文化）。

【上传是否复用 Files 通道】
- **本波无上传端点**：`/api/echomuse/*`、`/api/tuya/*` 全是 GET/POST JSON，无 octet-stream、无 multipart、无 64MB 大小限制；`POST /ble/gatt` 挂 `DefaultBodyLimit::max(4096)`；`oww/install-vad` 是**服务端自己下载资产**（oww.rs:110-121，GITHUB_PROXY / 8MiB），不是客户端上传。
- 「模型文件」上传复用 **W2 Files 页**（`/api/files/upload`、裸字节 `application/octet-stream`、XHR onprogress 真实百分比、64MB、`oww-models/` 目录；AGENTS.md:159、EchoMuseState.kt:182-195），**上传 ≠ 安装**（AGENTS.md:118）。固件目录 `/data/firmware` 与 Files root 重叠风险在 W10 已登记（AGENTS.md:164），本波不新增上传链、不重复该结论。

【与 W11 Plugins 的边界（device_plugins 历史键）】
- Kotlin `DeviceManagementCenter.kt:66-71/:179-181` 用 `devicePluginRegistered` / `DevicePluginNav.showDeviceHub` / `showPluginPartition` 做分区与菜单门控，数据源是 `GET /api/plugins` 的 **`device_plugins[]`**（PluginMetaState.kt:149、PluginSlots.kt:315-336：缺键→`UNKNOWN`→静态基线常显不闪隐；明确 `[]`→pureBase 隐藏 devicehub 入口）。
- **W11 已证伪：`device_plugins` 在服务端源码零匹配、不下发**（`/api/plugins` 真实投影是 `wasm.plugins[]` + capabilities）；本次复查 admin-web/src 全目录 Grep `device_plugins` / `devicePluginNav` / `echomuse_devices_changed` **零匹配**（W11 迁移未消费）。
- ⇒ admin-web devicehub **不得照搬 Kotlin 注册门控**（照搬会永远 UNKNOWN→常显，或与服务端语义脱节）；应改为按 `/api/echomuse/devices` 的 `enabled` 与 `/api/tuya/status` 的 `configured`/`enabled` 做**字段级 fail-closed**，与 C6（admin-web/AGENTS.md:163「echomuse/tuya 的 fail-closed 是字段级……按『能力未编入当前构建』UnavailableNotice」）口径一致。这是 W11 `device_plugins[].registered` 历史键结论在本波的延伸。

═══════════════════════════════════════
§13 实施清单（新增/修改文件与 UI 映射；三个待拍板前置不替下结论）
═══════════════════════════════════════
待拍板前置：R4 `/em/` CDN vendor 与否（1/2 §7）；R3 iframe 本波是否接入 + devicehub 的 nav 形态（1/2 §7）；R11 扩展工具范围（§10）。以下按「全部实施」给全量清单，拍板后按范围裁减。

【新增 API 封装（建议 3 个，均按 helper 模板）】
- `src/lib/echomuseApi.ts`：GET devices（判 `{enabled,version,devices[]}`）、GET `/em/status`（**无鉴权方法**，头注释显式声明不写 401/token）、GET `devices/{id}`、POST approve/command/settings/music、POST `devices/{id}/delete` 别名、DELETE `devices/{id}`、GET runtime / system/info / system/logs、POST volume / system/reboot、GET oww/state、POST oww/install-vad、BLE 3 端点（若 R11 含 ble）、console WS URL 构造（若做）；`EchomuseUnavailableError` 双路径归一 + 401 纯文本分支。
- `src/lib/tuyaApi.ts`：GET status、POST credentials / credentials/delete、DELETE credentials、POST sync（**skipped 不报成功**）、GET devices / devices/{id}、POST dp / alias、POST cloud/proxy；`TuyaUnavailableError` 双路径归一 + 401 纯文本分支。
- `src/lib/emEmbed.ts`（仅 R3 选 iframe）：拼 `/em/?embed=1&device=<enc>&token=<enc>&tab=<tab>`，**尾斜杠**、`encodeURIComponent` 一次、tab 白名单映射（shell→console 由 host.js 内部处理，前端只传 shell）、iframe 明确高度、加载失败探测（onload 超时 + 诚实文案）。
- 三个文件均：头注释契约 + `XxxUnavailableError` + `isRecord/str/num` + 判业务形状不判 200 + `SyntaxError` 归一（模板 `mcpApi.ts` / `otaApi.ts:119-126`）。

【新增页面 / 组件】
- `src/pages/DevicehubPage.tsx`（或改造 `EchomusePage.tsx`）：title 栏（「设备管理」+ `EchoMuse N 台 · 涂鸦 M 台 · 在线 X` + 扩展工具/返回设备管理 + 刷新 + 齿轮「设备接入配置」）→ 两分区卡栅格 → 点卡弹详情。
- `src/components/devicehub/EchoMuseDeviceCard.tsx` / `TuyaDeviceCard.tsx`（在线/离线 Badge、「涂鸦」徽标、型号未知、IP 未上报、未批准态）。
- `src/components/devicehub/DeviceAccessConfigDialog.tsx`：齿轮一次性配置（EchoMuse 状态+跳 Config C6「echomuse」；Tuya 凭证表单 + 立即同步 + 跳「tuya」；**outsideClick 不关闭**）；**凭证唯一录入路径，不接 ConfigDraftProvider/SaveBar**（C6 :163 铁律）。
- `src/components/devicehub/TuyaCredentialsCard.tsx`：区域 4 选项、access_id、secret 留空保留、保存后立即清空草稿、清除行内二次确认、「已配置 · 掩码 · 区域」。
- `src/components/devicehub/TuyaDeviceDialog.tsx`：DP 列表（switch/number/readonly 由服务端 `control` 决定，**客户端不重算 writable**）、下发后回包刷新、三归类错误 + `advice` 中文原文、本地别名。
- `src/components/devicehub/EchoMuseDetailDialog.tsx`：R3 若保留原生六 Tab → 状态/活动/配置/控制台/更新/日志；若选 iframe → `<iframe src={emEmbedUrl}>` + 高度 + 失败提示（未批准设备忽略 tab）。
- `src/components/devicehub/ExtensionWorkspace.tsx`（R11 范围内 5 项，UNDERLINE Tabs）：DLNA 音乐 / GATT 工具 / 模型文件（复用 FilesPage 默认 `oww-models`，上传≠安装）/ 原始诊断（runtime+system/info+复制+二次确认重启）/ 设备兼容。
- 复用既有：`@/components/feedback/{UnavailableNotice,DataState,ConfirmDialog,ToastHost}`、`@/lib/clipboard` 的 `copyText()`（AGENTS.md:210，三级降级不抛）、`@/lib/useApiQuery`（带请求序号，:185）。

【修改文件】
- `src/lib/nav.ts`：拍板 devicehub 的 key/label/description（现 9 项无 devicehub；`devices`≠Kotlin devicehub）；`tauriOnly` 保持仅 testbench。
- `src/router.tsx`：`PAGE_COMPONENTS` 增/改映射；若做扩展工作区子路由须声明在 `path="*"` **之前**（对齐 W11 `plugin/:pluginId/:entryId`，:48-51）。
- `src/pages/EchomusePage.tsx`：从 PlaceholderPage 改真实页或按 R3 决定保留。
- `client/admin-web/AGENTS.md`：迁移完成后按 W6–W11 惯例追加 W12 段落——:163「`/api/echomuse/*`、`/api/tuya/*` 是 W12 运行面，本轮 0 调用」需更新；:198 待办第 3 项、:199 待迁移清单、:209 迁移顺序、:205 bundle 体积基线（现 669.02 kB / gzip 210.21，九波 500→…→669.02，code-splitting W11 结论仍不做）。

【UI 映射要点】
- 两路加载失败用 danger Alert 就地（不弹 toast）：「EchoMuse 设备列表加载失败」/「涂鸦设备加载失败」+ description=服务端 `error` 原文；**列表请求失败不得落到「该目录为空」**（AGENTS.md:159 Files 教训）。
- EchoMuse 实时更新：走 `GET /api/admin/ws` 的 `echomuse_devices_changed` → refresh（**无轮询、无长挂**）；注意 admin-web 现有 `getWebSocketUrl()` 零调用点（AGENTS.md:160）、Console 用 `/api/logs` 单独推导；或保守用手动刷新（W9 MCP 口径 AGENTS.md:163）。
- Tuya 同步按钮：`syncing` 时「同步中…」禁用；`skipped` 显示服务端 message 但**不报成功**；`lastSyncText`「最近同步：成功 · N 台 · X 前」用服务端 `age_ms` 相对时间（对齐 W9 MCP ageText，不把上次伪装成现在）。
- 空态四型逐字沿用：未启用 / 未配置 / 没设备 / 没已注册插件（§8、§11）；删除、重启走 `ConfirmDialog` destructive + 点遮罩不关闭。
- **devicehub 是 0 配置字段纯运行面板**（同 W10 C7 形态）：凭证/同步/DP/音乐/诊断全走独立端点，不接 ConfigDraftContext / setField / SaveBar / dirtyCount / revision。
- **不要**把 `/api/tuya/credentials` 接进 PUT /api/config 主链；C6 只暴露 enabled/sync_interval_secs，两套 UI 不互相覆盖。

【QA 关注点】
- pure-base HTML 200 双路径归一（新增 api 文件必查，AGENTS.md:165）；401 纯文本 vs 404 JSON 区分。
- `skipped` / `queued` / `applied:false` / `rebooted:false` / `delivered:false` 一律**不得报成功**；wifiPending 不确定态显示。
- iframe：尾斜杠 + tab 白名单 + 未审批忽略 tab + 缺 device 不发请求 + CDN 失败诚实提示。
- `device_plugins` 门控不得照搬；`writable`/`control` 由服务端下发，客户端不重算。
- bundle 体积回归（AGENTS.md:205 基线）；R4 若 vendor 需回归 `em/tests/host.test.cjs` 与 `build.gradle.kts verifyEmBundle`。

═══════════════════════════════════════
§14 绝对路径索引（根 /Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/）
═══════════════════════════════════════
服务端：
- server/Cargo.toml
- server/src/main.rs、server/src/dev.rs、server/src/config.rs
- server/src/gateway/ws/mod.rs、server/src/gateway/ws/admin_ws.rs、server/src/gateway/ws/AGENTS.md、server/src/gateway/files/AGENTS.md
- server/src/device/echomuse/{mod.rs,config.rs,registry.rs,routes.rs,admin.rs,system.rs,oww.rs,oww_state.rs,console.rs,ble_http.rs,ble.rs,settings.rs,shell.rs,runtime.rs,frames.rs,music.rs,dlna.rs,AGENTS.md}
- server/src/device/tuya/{mod.rs,config.rs,api.rs,sync.rs,store.rs,cloud.rs,lan.rs,dps.rs,worker.rs,voice.rs,protocol.rs}
- server/src/device/{AGENTS.md,firmware/http.rs}
静态资产与托管：
- client/apps/h5App/em/{index.html,README.md,README.sha256,LICENSE}；em/static/{adapter.js,dashboard.js,dashboard.jsx,host.js}；em/tests/{console.browser.cjs,host.test.cjs,native-console.browser.cjs,native-management.browser.cjs,native-volume.browser.cjs}
- client/apps/h5App/build.gradle.kts、client/apps/h5App/web/em/、dev-server.sh
Kotlin 旧端（均在 client/shared/src/commonMain/kotlin/com/xiaozhi/admin/）：
- DeviceManagementCenter.kt、EchoMuseOfficialEntry.kt、EchoMuseOfficialView.kt、EchoMuseExtensionContent.kt、EchoMuseState.kt、EchoMuseBleState.kt、EchoMuseDetailState.kt、EchoMuseMusicState.kt、EchoMuseMusicPanel.kt、EchoMuseConfigDrafts.kt、EchoMuseLogsState.kt、EchoMuseConsoleTab.kt、EchoMuseNativeConfigTab.kt、EchoMuseNativeStatusTab.kt、EchoMuseNativeUpdatesTab.kt、EchoMuseNativeLogsTab.kt、EchoMuseApprovalTab.kt、EchoMuseBleCard.kt、EchoMuseBleCodec.kt、TuyaState.kt、TuyaPage.kt、TuyaDeviceDialog.kt、TuyaDetailPage.kt、ConfigTuyaCard.kt、ConfigTuyaState.kt、ConfigEchoMuseCard.kt、ConfigEchoMuseState.kt、AdminPlatform.kt、AdminShell.kt、MacServerSwitch.kt、PluginMetaState.kt、AGENTS.md、contracts/PluginSlots.kt
- client/shared/src/{jsMain,iosMain,androidMain,macosArm64Main}/kotlin/com/xiaozhi/admin/AdminPlatform.kt
- client/apps/h5App/src/jsMain/kotlin/com/xiaozhi/admin/{EchoMuseOfficialViewHost.kt,KuiklyWebRenderViewDelegator.kt}
admin-web：
- client/admin-web/{AGENTS.md,vite.config.ts,package.json}；src/lib/{nav.ts,api.ts,adminApi.ts,mcpApi.ts,otaApi.ts,useApiQuery.ts,clipboard.ts}；src/{router.tsx,App.tsx}；src/components/{AppShell.tsx}；src/pages/{PlaceholderPage.tsx,EchomusePage.tsx,PluginsPage.tsx,FilesPage.tsx,ConfigPage.tsx}；src-tauri/src/lib.rs

—— W12 报告完（1/2 + 2/2）。三条待拍板项的全部决策事实已给齐，未替实施下结论。研究全程只读：未创建/修改任何文件，未执行构建、未 git 操作。

---

> **以下为「补充勘误」原文，全量保留，未删减。**

---

W12 报告补充一条（1/2 §1 / §7、2/2 §13 的追加事实，属于收尾复查时发现的前置缺口，请与两条报告一并归档）。

任务原文提到「构建产物进 admin-web/dist 的构建步骤」——**实测当前并不存在这一步**，这是 R3（iframe 本波是否接入）与 R4（CDN vendor）之前更前置的一个决策点，需拍板：

【逐项实测证据】
- `client/admin-web/dist/` 现仅 `assets/`、`index.html`(510 B)、`logo.png`(383,356 B)，**无 `dist/em/`**（`ls` 实测）；`client/admin-web/public/` 仅 `logo.png`。
- `client/admin-web/vite.config.ts` 全文只有 `/em` 的 dev proxy（:42），**无 publicDir / 无 em 拷贝配置**；`build: { outDir:"dist", emptyOutDir:true }`（:46-48，每次构建清空 dist）。
- `client/admin-web/package.json` Grep `em|h5App|copy|publishWeb|dist` **零匹配**——没有任何把 em 资产拷进 dist 的脚本。
- `/em/` 产物的**唯一**来源是 `client/apps/h5App/build.gradle.kts:98-104 syncEmAssets` → `client/apps/h5App/web/em/`（1/2 §1），由 Gradle `publishWeb` 触发，与 admin-web 的 Vite 构建是**两条独立构建链**。

【由此推出的关键事实】
1. 默认链可用：`[server].admin_dir` 默认 `../client/apps/h5App/web`（server/src/config.rs:217-218、dev.rs:88-91、dev-server.sh:10）——该目录含 `em/`，故默认部署下 `/em/` 同源可达。
2. **生产方案 B 的缺口**：admin-web/AGENTS.md:43「生产由 Rust ServeDir 同源托管（dev-server.sh 的 XIAOZHI_ADMIN_DIR 可指向本目录 dist/）」、:51「纯单二进制、同源托管静态页与 /api、/ws、/em/」、:56「生产默认加载服务器页面」——若 `XIAOZHI_ADMIN_DIR` 真指向 `admin-web/dist`，则 `/em/` 路径不存在 → 落 `api.fallback_service(ServeDir…ServeFile(index.html))`（gateway/ws/mod.rs:326-334）→ **返回 admin-web 的 index.html（HTML 200）**，iframe 内会二次挂载 React SPA 而非官方页。这正是 em/README.md:182-183 警告的「在 `/em/` 下启动 Kuikly 造成二次初始化」的等价风险（此处换成 React）。
3. vite proxy `/em`（vite.config.ts:42）**只解决 dev 转发，不解决生产构建产物**；`emptyOutDir:true` 还会每次清掉任何手工放进 dist 的 em/。
4. 与 fail-closed 形态叠加：本波 pure-base 下 `/api/echomuse/*` 本就落 SPA fallback HTML 200（1/2 §5）；若 admin_dir 又指向 admin-web/dist，`/em/` 也落同一个 HTML 200，iframe 报的是「加载了一个 SPA」而非「能力未启用」——前端难以区分，诚实文案会失真。

【对决策表/实施清单的追加（不替下结论）】
- §7 R4 表新增一行维度「**资产落点**」：现状 em/ 只在 h5App/web/em；若生产用 admin-web/dist 作 admin_dir，必须先新增「把 em/ 拷进 dist」的构建步骤（当前零），否则 vendor 与否都白屏/二次挂载；R4 若决定 vendor，可与拷贝步骤合并处理（改 syncEmAssets exclude + 新增 admin-web 侧拷贝 + 复核 LICENSE 随分发）。
- §7 R3 表补充：接 iframe 的前置条件除「vite proxy 排除 + withGlobalTauri:false + host.js 契约」外，**还必须先拍板生产 admin_dir 指向哪个构建产物**——保持 h5App/web（`/em/` 可达，但与「admin-web dist 是生产前端」的方案 B 表述冲突）还是改造 admin-web 构建链（新增 em 拷贝，且要处理 emptyOutDir）。
- §13【修改文件】追加：`client/admin-web/package.json`（或 `vite.config.ts`）新增 em 资产拷贝步骤（若选 admin-web/dist 方案）；§13【QA 关注点】追加：生产 admin_dir 下直接请求 `/em/` 必须返回官方 index.html（含 `static/adapter.js` 引用），**不得返回 SPA 的 index.html**。
- em/README.md:184-185「web/em/ 是 syncEmAssets 的拷贝产物，已 gitignore；改 em/ 结构后先 rm -rf client/apps/h5App/web/em（publishWeb 不清理旧文件）」——若改为双落点，这条清理口径需同步。

其余结论不变。研究全程只读：未创建/修改任何文件、未构建、未 git 操作。W12 报告（1/2 + 2/2 + 本补充）至此完整。


---

# 附录 · 逐端点完整 wire 表（落盘前逐行核读新增）

> 本附录满足交付要求「18 个 echomuse 端点 + 11 个 tuya 方法的完整 wire 表（路径/方法/鉴权/请求/响应字段/错误形态）」。每个端点固定给出五个维度：**方法/路径、鉴权、请求、成功响应字段、错误形态**。公共约定集中在附录 A（闸门 G0–G4、错误形状 E1–E3、复用枚举、全字段结构），附录 B/C 逐端点引用，避免重复但不省略任何字段。
> 权威源码：`server/src/device/echomuse/{admin.rs,system.rs,oww.rs,oww_state.rs,console.rs,ble_http.rs,shell.rs,runtime.rs,registry.rs,routes.rs}`、`server/src/device/tuya/{api.rs,lan.rs,sync.rs,config.rs,mod.rs}`、`server/src/gateway/ws/mod.rs`。

## 附录 A · 统一鉴权闸门与错误形状（wire 表公共约定）

### A.1 闸门

| 编号 | 闸门 | 组成 | 失败响应 |
|---|---|---|---|
| G1 | `admin::gate`（echomuse/admin.rs:366-378） | `auth_ok(headers, query, [server])` → `[echomuse].enabled` | 401 **纯文本** `unauthorized`（非 JSON）；404 JSON `{"error":"echomuse disabled"}` |
| G2 | `system::system_gate`（system.rs:59-80） | G1 + `approved_emos`（已批准 / emOS / 能力）+ 在线 control + `live.authenticated` + `peer` 五重 | G1 失败同上；其后按 `ShellError::response`（A.3） |
| G3 | `tuya::gate`（tuya/api.rs:350-362） | `auth_ok` → `[tuya].enabled` | 401 **纯文本** `unauthorized`；404 JSON `{"error":"tuya disabled"}` |
| G4 | `ble_http::gate`（ble_http.rs:97-116） | 生产态 = G1（`Arc<Engines>` 走 `admin::gate`，:83-85）+ `[echomuse].ble_proxy` | G1 失败同上；`ble_proxy=false` → 404 JSON `{"error":"ble proxy disabled"}` |
| G0 | 无鉴权 | 仅 `GET /em/status`（routes.rs:66-67、71-75） | 头注释显式声明「不写 401/token 分支」 |

> 管理面凭据 = `[server].expected_token`（Bearer / `?token=`）；设备面凭据 = `X-Em-Token`（`[echomuse].token`）。**两套凭据勿混**（1/2 §6.11、§7 R3）。

### A.2 三种错误形状（并存，前端归一不可只认一种）

- **E1 纯 error 单键** `{"error": "<text>"}`：`admin::err_json`（admin.rs:84-86）、`disabled_404`（:75-80）、`tuya::json_404`/`json_error`（api.rs:364-374）、`failure_payload`（tuya 控制失败额外含 `kind` + `advice`，api.rs:237-243）。
- **E2 error 在前 + code** `{"error": "<text>", "code": "<code>"}`：`system::bad_request`（system.rs:222-227，code 恒 `bad_request`）、`ShellError::response`（shell.rs:116，`CommandFailed` 再带 `exit_code`）、`oww::install_vad` 的 `asset_unreadable`（oww.rs:529-532）。
- **E3 code 在前** `{"code": "<code>", "error": "<text>"}`：`BleError::response`（ble_http.rs:50-53）。

> 401 闸门失败是**纯文本** `unauthorized`，既非 E1/E2/E3 也不带 code——`requestApi` 必须按 `status===401` 单独分支，不要当 JSON 解析（1/2 §5、2/2 §12 api 检查单 ②）。pure-base 下这些路径落 SPA fallback 返回 **HTML 200**（1/2 §5），catch 链须同时覆盖 `SyntaxError → UnavailableError` 与「JSON 成功但形状不符 → UnavailableError」。

### A.3 `ShellError::response` 17 类映射（shell.rs:63-119；system 5 端点 / oww 2 端点 / install-vad 复用）

- `NotFound` → 404 `not_found` / `device not found`
- `Unapproved` → 403 `unapproved` / `device not approved`
- `Unsupported` → 422 `unsupported_os` / `system management requires emOS`
- `MissingSpeaker` → 422 `unsupported_capability` / `device does not declare speaker capability`
- `Offline` → 409 `offline` / `device control is offline or replaced`
- `Busy` → 409 `busy` / `another system operation is active`
- `PeerUnavailable` → 503 `peer_unavailable` / `authenticated control peer unavailable`
- `Unauthorized` → 401 `unauthorized` / `invalid device token or control peer`
- `Unsolicited` → 409 `no_pending_shell` / `no matching pending shell request`
- `Timeout` → 504 `shell_timeout` / `shell deadline exceeded; outcome unknown`
- `Transport` → 502 `shell_transport` / `shell disconnected or transport failed; outcome unknown`
- `OutputLimit` → 502 `output_limit` / `shell output exceeded limit`
- `InvalidMarker` → 502 `invalid_marker` / `invalid shell completion marker`
- `CommandFailed(exit)` → 502 `command_failed` / `device command returned nonzero status` + `exit_code`
- `NoDecoder` → 502 `no_base64_decoder` / `device has no base64 decoder — no data was sent`
- `NoHashTool` → 502 `no_hash_tool` / `device has no sha256 tool — refusing to send unverified bytes`
- `VerifyFailed` → 502 `verify_failed` / `device checksum did not match — target file left untouched`

### A.4 `BleError::response` 13 类映射（ble_http.rs:17-55）

- `NotFound` → 404 `not_found` / `device not found`
- `Unapproved` → 403 `unapproved` / `device not approved`
- `Unsupported` → 422 `unsupported_capability` / `device does not declare ble_connect`
- `Unauthorized` → 401 `unauthorized` / `authenticated control required`
- `Offline` → 409 `offline` / `device control is offline`
- `SessionChanged` → 409 `session_changed` / `control session changed; refresh devices`
- `Disabled` → 409 `ble_disabled` / `BLE scanning and connections must both be enabled`
- `DataOffline` → 409 `data_offline` / `device data link is not ready`
- `Busy` → 429 `busy` / `too many pending BLE requests`
- `Timeout` → 504 `ble_timeout` / `BLE deadline exceeded; operation outcome unknown`
- `OutputLimit` → 502 `output_limit` / `device BLE output exceeded limit`
- `Protocol` → 502 `ble_protocol` / `invalid device BLE result`
- `BadRequest(message)` → 400 `bad_request` / message（`session_id` 非法、`build_gatt_request` 失败）

### A.5 复用的响应结构（供 B/C 引用）

- **`record_payload`**（registry.rs:76-96 + admin.rs:109-117 注入 `session_id`）：`device_id, label, approved, version, board, base_os, capabilities, first_seen, last_seen, online, connected_secs, settings, ip, kernel_arch, kernel_release, boot_id, boot_reason, session_id`（在线时为 UUID 字符串，离线为 `null`）。**无 `ok` 键**。
- **runtime `payload_with_link`**（runtime.rs:191-198）：`payload()` 全字段 + `link: {authenticated, token_required}`（无设备侧 TLS 位）；**离线 `offline_payload`**（:201-209）：`device_id, session_id:null, online:false, stats:null, listen_state:null, listen_end:null, playback_stats:null, oww_shadow_cross:null, wifi_result:null, wifi_scan:null, ambient_light:null, pair_requested_at:null, volume_state:null, mute_state:null, updated_at:null, link:null`（显式 null 不缺键）。
- **tuya `last_sync`**（api.rs:147-153）：`{finished_at, age_ms, result, device_count, message}`（`age_ms` 服务端算好）；**`lan_unregistered[]`**（:154-161）：`{device_id, ip, age_ms}`。
- **OWW 五态**（oww_state.rs:60-61）：`installed / outdated / blocked / unknown / missing`；**读不到设备 ≠ missing**（一律 `unknown`，:70-93）。

---

## 附录 B · `/api/echomuse/*` 18 端点完整 wire 表

> 全部 `#[cfg(feature="echomuse-plugin")]` 门控。18 = admin 7 `.route` + system 5 + oww 2 + console 1 + ble 3；`GET|DELETE /devices/{id}` 是同一 `.route` 双方法，故 **18 端点 / 19 方法**（附录 E）。所有响应 `Content-Type: application/json; charset=utf-8`（`admin::json` :58-72）。

### B.1 `GET /api/echomuse/devices`（admin.rs:40 / :380-393）
- **方法/路径**：GET `/api/echomuse/devices`
- **鉴权**：G1
- **请求**：无 body、无 query
- **成功响应 200**：`{enabled, version, devices[]}`（`devices_payload` :90-100）。`version` 是设备面变化号（管理台 WS hello / `echomuse_devices_changed` 同源）；`devices[]` 元素 = A.5 `record_payload`。**无 `ok` 键**（勿判 `ok===true`）。
- **错误形态**：401 纯文本 `unauthorized`；404 E1 `{"error":"echomuse disabled"}`。

### B.2 `GET|DELETE /api/echomuse/devices/{id}`（admin.rs:41-44 / :395-408、:410-428）
- **方法/路径**：GET `/api/echomuse/devices/{id}`；DELETE `/api/echomuse/devices/{id}`
- **鉴权**：G1
- **请求**：路径 `{id}`；无 body
- **成功响应**：GET 200 = A.5 `record_payload`；DELETE 200 = `{"removed":true,"device_id":id}`（:421-424）
- **错误形态**：401 纯文本；404 E1 `{"error":"echomuse disabled"}`；404 E1 `{"error":"device not found"}`（记录不存在 :406 / :426）。

### B.3 `POST /api/echomuse/devices/{id}/delete`（admin.rs:45 / :410-428）
- **方法/路径**：POST `/api/echomuse/devices/{id}/delete`
- **鉴权**：G1
- **请求**：无 body（`NetworkModule` 无 DELETE 的别名，同 OTA 惯例）
- **成功响应 200**：`{"removed":true,"device_id":id}`
- **错误形态**：同 B.2 DELETE。

### B.4 `POST /api/echomuse/devices/{id}/approve`（admin.rs:46 / :430-449、:120-127）
- **方法/路径**：POST `/api/echomuse/devices/{id}/approve`
- **鉴权**：G1
- **请求**：`{"label": "..."}`（可选，缺省 `{}`）；认领与改名同一端点
- **成功响应 200**：A.5 `record_payload`（`bump_devices_version`，:444）
- **错误形态**：401 纯文本；404 E1 `{"error":"echomuse disabled"}`；404 E1 `{"error":"device not found"}`。

### B.5 `POST /api/echomuse/devices/{id}/command`（admin.rs:47 / :502-525、:131-154、:342-361）
- **方法/路径**：POST `/api/echomuse/devices/{id}/command`
- **鉴权**：G1
- **请求**：`{"action": "wifi"|"wifi_scan"|"wifi_commit", "ssid"?, "psk"?}`。`wifi` 需 `ssid` 非空（trim）；下发 `wifi_change` 帧带 `ssid`+`psk`。**`action=reboot` 明确拒绝**（要求走 B.12 `system/reboot`）。
- **成功响应 202**：`{"queued":true}`（排队 ≠ 应用）
- **错误形态**：401 纯文本；404 E1 `{"error":"echomuse disabled"}`；409 E1 `{"error":"device offline"}`（离线或 control 发送失败，:514/:520）；400 E1 `{"error":"wifi 需要 ssid"}` / `{"error":"action 只支持 wifi/wifi_scan/wifi_commit；重启请使用确认 system/reboot 接口"}` / `{"error":"reboot requires POST system/reboot with confirm=true"}`。

### B.6 `POST /api/echomuse/devices/{id}/settings`（admin.rs:48 / :451-500、:182-236、:266-301、:306-339）
- **方法/路径**：POST `/api/echomuse/devices/{id}/settings`
- **鉴权**：G1
- **请求**：**40 键白名单**（`SETTINGS_KEYS` :182-236），部分更新、零值忽略；每个键须匹配 kind：
  - `wakeWordEnabled`(bool)、`owwOnDevice`(off|shadow|on)、`owwModel`(string)、`owwThreshold`((0,1])、`wakeSound`(bool)、`wakeSoundLevel`(quiet|medium|loud)、`wakeMic`(integer 0..6)
  - `vadThreshold`((0,1])、`vadSpeechMs`(integer 0..60000)、`vadSilenceMs`(integer 0..60000)
  - `adcMicpga`(integer 0..127)、`adcDigitalGain`(integer 0..127)、`micGainDb`(integer 0..42)、`startupVolume`(integer 0..127)
  - `aecEnabled`(bool)、`aecDelayMs`(integer 0..1000)、`aecTailMs`(integer 0..500)、`aecRefSource`(auto|hw|sw)、`agcEnabled`(bool)
  - `beamformingEnabled`(bool)、`beamAngle`(number -1..360)、`bargeInEnabled`(bool)、`bargeInThreshold`(number 0..1)
  - `responseLevel`(low|medium|high)、`volumeButtonSound`(bool)、`remoteVolumeArc`(bool)
  - `eqBands`(float list 非空数字数组)、`eqLoudness`(bool)、`bassGuardEnabled`(bool)、`bassGuardDb`(number -60..24)、`limiterEnabled`(bool)、`limiterThreshold`(number -60..0)、`limiterRelease`(number 10..2000)、`duckDb`(number -60..0)
  - `bleProxyEnabled`(bool)、`bleProxyConnections`(bool)、`sendspinEnabled`(bool)、`sendspinName`(string)、`sendspinUnpaired`(bool)、`listeningAnim`(json)
  - 官方默认值逐字（QA 排障基准，:171-178）：`vadThreshold=0.004, vadSpeechMs=80, vadSilenceMs=600, owwThreshold=0.5, owwModel=hey_jarvis_v0.1, owwOnDevice=off, bargeInThreshold=0.05, wakeWordEnabled=true, duckDb=-18, responseLevel=low, wakeSound=false, wakeSoundLevel=medium, remoteVolumeArc=false, adcDigitalGain=88, adcMicpga=40, micGainDb=24, beamAngle=-1, beamformingEnabled=true, wakeMic=0, agcEnabled=true, aecEnabled=true, aecDelayMs=0, aecTailMs=300, aecRefSource=auto, bleProxyEnabled=false, sendspinEnabled=false, sendspinUnpaired=false, startupVolume=85, volumeButtonSound=true`
- **成功响应**：在线 **202** / 离线 **200**，`{"persisted":true,"delivered":bool}`（:496-499；`delivered` 是逐 `ControlOut` 发送结果，非 `online` 布尔；离线仍持久化，设备下次接入由 `control_session` 推送合并 snapshot，:474-485 防 owwOnDevice 基线被 delta 重刷）
- **错误形态**：401 纯文本；404 E1 `{"error":"echomuse disabled"}`；404 E1 `{"error":"device not found"}`；400 E1 三类逐字 —— `{"error":"body 必须是 JSON 对象"}`（:274）/ `{"error":"不支持的设置键: {k}"}`（:278）/ `{"error":"设置键 {k} 需要 {kind}"}`（:296）。

### B.7 `POST /api/echomuse/devices/{id}/music`（admin.rs:49 / :535-619）※ R11 本轮延后，wire 仍全留
- **方法/路径**：POST `/api/echomuse/devices/{id}/music`
- **鉴权**：G1（`[echomuse].music_dir` 留空 = 音乐面 404，config.rs:67-71）
- **请求**：`{"action": "scan"|"browse"|"play_url"|"stop", "location"?, "url"?}`
- **成功响应（按 action）**：
  - `scan` → 200 `{"servers":[{"name","location"}]}`；空结果 200 `{"servers":[],"hint":"局域网未发现 DLNA 媒体服务器（确认服务器已开机且同网段；容器部署需 host 网络才能收到 SSDP 组播）"}`（:552-558）
  - `browse` → 200 `{"tracks":[{"title","artist","album","url","duration"}]}`（:568）
  - `play_url` → 202 `{"queued":true}`（:593）
  - `stop` → 200 `{"stopped":true}`（:612）
- **错误形态**：401 纯文本；404 E1 `{"error":"echomuse disabled"}`；400 E1 `{"error":"browse 需要 location"}`（:564）/ `{"error":"play_url 需要 url"}`（:576）/ `{"error":"action 支持 scan/browse/play_url/stop"}`（:616）/ browse、play_url 底层失败 `{"error": e}`（:569、:594）；**play_url / stop 两级 409**：`{"error":"device offline"}`（:582、:602）与 `{"error":"data channel offline"}`（:588、:608）。扫描/浏览不依赖设备在线。

### B.8 `GET /api/echomuse/devices/{id}/runtime`（system.rs:42 / :82-101）
- **方法/路径**：GET `/api/echomuse/devices/{id}/runtime`
- **鉴权**：**G1 + registry 存在性**（注意：**不是** `system_gate`，:88-93）；`token_required = [echomuse].token 非空`
- **请求**：路径 `{id}`；无 body
- **成功响应 200**：在线 = A.5 `payload_with_link`（含 `link{authenticated,token_required}`）；离线 = A.5 `offline_payload`（全 null）
- **错误形态**：401 纯文本；404 E1 `{"error":"echomuse disabled"}`；404 E2 `{"error":"device not found","code":"not_found"}`（`ShellError::NotFound`，:92）。

### B.9 `POST /api/echomuse/devices/{id}/volume`（system.rs:43 / :111-148、:103-109）
- **方法/路径**：POST `/api/echomuse/devices/{id}/volume`
- **鉴权**：G1 + `approved_emos` + `speaker` 能力 + `live.authenticated` + 在线 control（:118-143）
- **请求**：**仅且必须** `{"level": int 0..127}`（body 只能 1 个键，:104-107）
- **成功响应 202**：`{"queued":true,"applied":false,"level":<原值>}`（排队 ≠ 应用）
- **错误形态**：401 纯文本；404 E1 `{"error":"echomuse disabled"}`；400 E2 `{"error":"body must contain only level","code":"bad_request"}` / `{"error":"level must be an integer in 0..127","code":"bad_request"}`；其余按 A.3（403 unapproved / 422 unsupported_capability / 409 offline / 401 unauthorized / 503 peer_unavailable）。

### B.10 `GET /api/echomuse/devices/{id}/system/info`（system.rs:44 / :150-166）
- **方法/路径**：GET `/api/echomuse/devices/{id}/system/info`
- **鉴权**：G2（`system_gate` 五重）
- **请求**：路径 `{id}`；固定命令 `INFO`（:17-18，客户端不插值 shell）
- **成功响应 200**：`{"device_id":id,"output":<shell 输出>,"exit_code":0}`
- **错误形态**：G1 失败同上；G2 后续按 A.3（403/422/409/401/503/504/502 …）。

### B.11 `GET /api/echomuse/devices/{id}/system/logs`（system.rs:45 / :168-193、:27-37）
- **方法/路径**：GET `/api/echomuse/devices/{id}/system/logs`
- **鉴权**：G2
- **请求**：`?source=` 6 源白名单：`server / messages / messages_last / last_kmsg_prev / supervisor / net`（缺省 `messages`，:177-180）；各源固定 `tail -c 49152`，不插值
- **成功响应 200**：`{"device_id":id,"source":source,"output":<shell 输出>,"exit_code":0}`
- **错误形态**：G1 失败同上；400 E2 `{"error":"source must be server / messages / messages_last / last_kmsg_prev / supervisor / net","code":"bad_request"}`（:182-185）；其余按 A.3。

### B.12 `POST /api/echomuse/devices/{id}/system/reboot`（system.rs:46 / :200-220、:195-198）
- **方法/路径**：POST `/api/echomuse/devices/{id}/system/reboot`
- **鉴权**：G2
- **请求**：**精确** `{"confirm":true}`（只能 1 个键且值为 true，:195-198）
- **成功响应 202**：`{"device_id":id,"dispatched":true,"rebooted":false}`（确认调度 ≠ 已完成）
- **错误形态**：G1 失败同上；400 E2 `{"error":"reboot requires exactly {\"confirm\":true}","code":"bad_request"}`（:211）；其余按 A.3。

### B.13 `POST /api/echomuse/devices/{id}/oww/install-vad`（oww.rs:126-129 / :517-554）
- **方法/路径**：POST `/api/echomuse/devices/{id}/oww/install-vad`
- **鉴权**：G2（`system_gate`，:523）
- **请求**：无 body（服务端自己下载 typed 重写版 silero_vad.onnx + mkdir -p + 经受限 shell 推流；不依赖客户端上传）
- **成功响应 200**：`{"device_id":id,"installed":true,"file":"silero_vad.onnx","path":"/data/local/share/echomuse/oww/silero_vad.onnx","bytes":<transfer.bytes>,"sha256":<transfer.sha256>,"applies":"next-turn"}`（:539-551）
- **错误形态**：G1 失败同上；G2 后续按 A.3；500 E2 `{"error":<说明>,"code":"asset_unreadable"}`（模型缺失/畸形，:529-532）；传输失败按 A.3（`no_base64_decoder` / `no_hash_tool` / `verify_failed` 等 502）。

### B.14 `GET /api/echomuse/devices/{id}/oww/state`（oww_state.rs:29 / :141-164、:100-134）
- **方法/路径**：GET `/api/echomuse/devices/{id}/oww/state`
- **鉴权**：G2（`system_gate`，:147）
- **请求**：路径 `{id}`；无 body
- **成功响应 200**：`{"device_id":id,"vad":{"state","file":"silero_vad.onnx","sha256":<服务端预期|null>,"bytes":<预期长度|null>},"ort":{"state","file":"libonnxruntime.so","sha256":<预期|null>},"free_mb":<u64|null>,"states":["installed","outdated","blocked","unknown","missing"],"checked_at":<unix 秒>}`（:116-133）。**读不到设备一律 `unknown`，绝不假填 `missing`/`installed`**；`free_mb < 50`（`FREE_FLOOR_MB`）时缺件判 `blocked`。
- **错误形态**：G1 失败同上；G2 后续按 A.3；**读不到设备不是错误**（仍 200 + unknown）。

### B.15 `GET /api/echomuse/devices/{id}/console`（WS，console.rs:68-71 / :76-91）
- **方法/路径**：GET `/api/echomuse/devices/{id}/console`（`WebSocketUpgrade`）
- **鉴权**：G2 **`system_gate` 必须在 `on_upgrade` 之前跑完**（:83-85，此时还能返回真 HTTP 状态码）；`insecure = [echomuse].token` 为空（:87）
- **请求**：WS upgrade；浏览器侧只允许 JSON 文本帧（入站 `input` / `ctrl-c` / `resize` / `raw`），设备侧 Binary（PTY 模式 stdin 帧需 `0x00` 帧头，:15-18）
- **成功响应（WS 帧，见附录 D.2）**：`ready{device_id,mode,raw_input,resize,insecure,notice}` → `shell_meta{pty}` → `output{text}` … → `closed{reason,detail,inputs,bytes_out,secs}`
- **错误形态**：HTTP 阶段按 G1/G2（含 A.3）；**upgrade 之后**占槽失败才发首帧 `error{code:"busy"}` 再关（浏览器 WS API 拿不到 HTTP 状态码）；`ErrorCode` ∈ busy / device_offline / internal；`ws.max_message_size/max_frame_size = INBOUND_MAX`。

### B.16 `GET /api/echomuse/devices/{id}/ble/adverts`（ble_http.rs:91 / :284-291、:301-333）※ R11 本轮延后，wire 仍全留
- **方法/路径**：GET `/api/echomuse/devices/{id}/ble/adverts`
- **鉴权**：G4（含 `ble_proxy` 开关）
- **请求**：`?session_id=<uuid>`（可选；`request::session_id` 校验，非法 → BadRequest）
- **成功响应 200**：`{"device_id":id,"session_id":<uuid>,"updatedAt":<u64|null>,"count":<int>,"adverts":[...]}`（`envelope` :218-222 注入前两键；`adverts_snapshot` :325-326）
- **错误形态**：401 E3 `{"code":"unauthorized","error":"authenticated control required"}`（测试态 Context）/ 生产态 G1 401 纯文本；404 E1 `{"error":"echomuse disabled"}`；404 E1 `{"error":"ble proxy disabled"}`；其余按 A.4（13 类）。

### B.17 `GET /api/echomuse/devices/{id}/ble/state`（ble_http.rs:92 / :292-299、:301-333）※ R11 本轮延后，wire 仍全留
- **方法/路径**：GET `/api/echomuse/devices/{id}/ble/state`
- **鉴权**：G4
- **请求**：`?session_id=<uuid>`（可选）
- **成功响应 200**：`{"device_id":id,"session_id":<uuid>,"slots":<value>,"events":[...]}`（`state_snapshot` :322-323；`events` 最多 50 条，含 `t:"notify"` / `t:"disconnected"`）
- **错误形态**：同 B.16。

### B.18 `POST /api/echomuse/devices/{id}/ble/gatt`（ble_http.rs:93 / :335-362）※ R11 本轮延后，wire 仍全留
- **方法/路径**：POST `/api/echomuse/devices/{id}/ble/gatt`
- **鉴权**：G4 + `Capture::new(..., gatt=true)`（已批准 / `ble_connect` 能力 / `bleProxyEnabled`+`bleProxyConnections` 均 true / 在线且已认证 / `session_id` 一致 / data 链就绪，:127-180）
- **请求**：JSON body 含 `session_id`（严格核对）+ GATT 命令，经 `build_gatt_request` 转设备控制帧；**`DefaultBodyLimit::max(4096)`**（:94）
- **成功响应 200**：`envelope(value)` = 设备应答原样 + `{device_id,session_id}`（:359）；**设备 `ok:false` 也是 200**（真实应答，非动作成功，:358）
- **错误形态**：404 `{"error":"ble proxy disabled"}`；400 E3 `{"code":"bad_request","error":...}`（session_id / build_gatt_request 失败）；其余按 A.4（13 类，含 409 session_changed / ble_disabled / data_offline、429 busy、504 ble_timeout、502 output_limit / ble_protocol）。

---

## 附录 C · `/api/tuya/*` 全方法完整 wire 表

> 全部 `#[cfg(feature="tuya-plugin")]` 门控。源码 `routes()`（tuya/api.rs:47-63）实为 **10 条 `.route` / 10 个 method handler / 9 个不同路径**；报告 1/2 §4、§6.3 与任务口径记为「10 条 `.route` / 11 方法（非 9）」——计数差异见附录 E。下表**把每个 method 独立成行全列，不遗漏任何 handler**。响应 `Content-Type: application/json`（`axum::Json`）。`status` 与 `devices` 响应**无 `ok` 键**；`access_secret` 任何响应/日志永不回显（api.rs:20-21）。

### C.1 `GET /api/tuya/status`（api.rs:50 / :404-433、:133-164）
- **方法/路径**：GET `/api/tuya/status`
- **鉴权**：G3
- **请求**：无 body、无 query
- **成功响应 200**：`{"enabled":bool,"configured":bool,"access_id_masked":string,"region":string,"sync_interval_secs":int,"syncing":bool,"last_sync":{finished_at,age_ms,result,device_count,message}|null,"lan_unregistered":[{device_id,ip,age_ms}],"store_path":string}`。**无 `ok` 键、无 secret**；`access_id_masked` = `ab****ef`（≤4 位 → `****`，:68-79）；`age_ms` 服务端算好。
- **错误形态**：401 纯文本 `unauthorized`；404 E1 `{"error":"tuya disabled"}`；500 E1 `{"error":<配置读取失败>}`（:414）。

### C.2 `POST /api/tuya/credentials`（api.rs:51 / :435-458、:83-120）
- **方法/路径**：POST `/api/tuya/credentials`
- **鉴权**：G3
- **请求**：`{"region":"cn|us|eu|in"(可选，缺省沿用旧值),"access_id":string(必填非空),"access_secret":string(可选：留空且已配置 = 保留旧值)}`；region 白名单 `REGIONS`（config.rs:29）
- **成功响应 200**：`{"ok":true,"message":"凭证已保存（secret 不再回显）"}`（:457；写库后 `invalidate_credentials()`）
- **错误形态**：401 纯文本；404 E1 `{"error":"tuya disabled"}`；400 E1 三类逐字 —— `{"error":"region 只支持 [\"cn\", \"us\", \"eu\", \"in\"]"}`（:91-94）/ `{"error":"access_id 不能为空"}`（:103）/ `{"error":"access_secret 不能为空（已配置过可留空保留旧值）"}`（:116）；500 E1 `{"error":<配置读/写失败>}`（:448/:454）。

### C.3 `DELETE /api/tuya/credentials`（api.rs:53-56 / :460-478）
- **方法/路径**：DELETE `/api/tuya/credentials`
- **鉴权**：G3
- **请求**：无 body
- **成功响应 200**：`{"ok":true,"message":"凭证已清除"}`（:477；三字段全清，enabled/周期不动）
- **错误形态**：401 纯文本；404 E1 `{"error":"tuya disabled"}`；500 E1 `{"error":<配置写回失败>}`（:474）。

### C.4 `POST /api/tuya/credentials/delete`（api.rs:52 / :460-478）
- **方法/路径**：POST `/api/tuya/credentials/delete`（`NetworkModule` 无 DELETE 的别名）
- **鉴权**：G3
- **请求**：无 body
- **成功响应 200 / 错误形态**：同 C.3。

### C.5 `POST /api/tuya/sync`（api.rs:57 / :480-494；sync.rs:19、:41-67）
- **方法/路径**：POST `/api/tuya/sync`
- **鉴权**：G3
- **请求**：无 body
- **成功响应 200**：`{"result":<string>,"message":<string>}`（:493）。`result` 取值：`ok` / `partial` / `cloud_auth_failed` / `network_error` / `skipped` / `busy`。**`skipped`（60s 闸门 `MIN_SYNC_INTERVAL` 或并发拦下）不算失败、UI 不得报成功**（sync.rs:44-55「距上次同步不足 60s，请稍后再试」/「已有一次同步在进行中」）；凭证未就绪 → `cloud_auth_failed`「涂鸦云凭证未配置（请先在管理台录入）」。
- **错误形态**：401 纯文本；404 E1 `{"error":"tuya disabled"}`；500 E1 `{"error":<配置读取失败>}`（:490）。

### C.6 `GET /api/tuya/devices`（api.rs:58 / :496-515、:167-183）
- **方法/路径**：GET `/api/tuya/devices`
- **鉴权**：G3
- **请求**：`?online=true|1|false|0`（过滤「只看在线」，:509-513）
- **成功响应 200**：`{"devices":[{device_id,name,model,online,ip,updated_at}],"total":int,"online":int}`（:182）。`name` = **alias 优先于 name**（alias 空则用 name，:173）；**total / online 是过滤后行数统计**；不回显 `local_key`。
- **错误形态**：401 纯文本；404 E1 `{"error":"tuya disabled"}`；500 E1 `{"error":<目录读取失败>}`（:506）。

### C.7 `GET /api/tuya/devices/{id}`（api.rs:59 / :517-535、:186-214）
- **方法/路径**：GET `/api/tuya/devices/{id}`
- **鉴权**：G3
- **请求**：路径 `{id}`
- **成功响应 200**：`{"device_id":id,"name":<alias 优先>,"model":string,"online":bool,"ip":string,"dps":[{dp,value,value_text,writable,control}]}`（:206-213）。`dps` 按 dp 字典序；`value_text`：bool→`开`/`关`、string 原样、null→空串、其余 JSON 文本（:217-225）；`control`：bool→`switch`、number→`number`、其余 `readonly`（:228-234）；**`writable` 由服务端 `dp_writable` 下发，客户端不得重算**。
- **错误形态**：401 纯文本；404 E1 `{"error":"tuya disabled"}`；404 E1 `{"error":"device not found"}`（:528）；500 E1 `{"error":<读取失败>}`（:529）。

### C.8 `POST /api/tuya/devices/{id}/dp`（api.rs:60 / :537-574、:237-243；lan.rs:67-73）
- **方法/路径**：POST `/api/tuya/devices/{id}/dp`
- **鉴权**：G3
- **请求**：`{"dp":"<数据点 ID>","value":<目标值>}`（两者必填）
- **成功响应 200**：`{"dps":{...},"message":"已下发"}`（:567）
- **错误形态**：401 纯文本；404 E1 `{"error":"tuya disabled"}`；400 E1 `{"error":"需要 dp（数据点 ID）"}`（:555）/ `{"error":"需要 value（目标值）"}`（:561）；控制失败 E1 扩展 `{"error":<message>,"kind":"offline|timeout|unsupported","advice":<中文建议>}`（:569-572 + `failure_payload` :237-243），HTTP 状态按 `ControlFailure::status_code`：`offline`→409 / `timeout`→504 / `unsupported`→400（lan.rs:67-73）。

### C.9 `POST /api/tuya/devices/{id}/alias`（api.rs:61 / :576-597）
- **方法/路径**：POST `/api/tuya/devices/{id}/alias`
- **鉴权**：G3
- **请求**：`{"alias":"<本地展示名，可为空串清除>"}`（必填，值可空串）
- **成功响应 200**：`{"ok":true}`（:594）
- **错误形态**：401 纯文本；404 E1 `{"error":"tuya disabled"}`；400 E1 `{"error":"需要 alias（本地展示名，可为空串清除）"}`（:590）；500 E1 `{"error":<写入失败>}`（:595）。

### C.10 `POST /api/tuya/cloud/proxy`（api.rs:62 / :599-649、:261-320、:337-345）
- **方法/路径**：POST `/api/tuya/cloud/proxy`
- **鉴权**：G3
- **请求**：`{"method":"GET|POST|DELETE"(缺省 GET),"path":"/v1...."(必填前缀), "query":{...}(可选，键按字典序签名), "body":<JSON>(仅 POST 允许)}`
- **成功响应 200**：`{"status":<云 HTTP 状态>,"body":<云回包原样透传；非 JSON 则为字符串>}`（:337-340）
- **错误形态**：401 纯文本；404 E1 `{"error":"tuya disabled"}`；400 E1 校验链逐字 —— `{"error":"method 只支持 GET/POST/DELETE（收到 {other}）"}`（:272）/ `{"error":"path 必须以 /v1. 开头（涂鸦 OpenAPI 路径，如 /v1.0/devices）"}`（:281）/ `{"error":"path 不能含空格、? 或 #（query 参数用 query 对象传）"}`（:284）/ `{"error":"只有 POST 请求允许带 body"}`（:309）/ `{"error":"涂鸦凭证未配置：先 POST /api/tuya/credentials 录入"}`（:621）；502 `{"kind":"auth|network|api","error":<message>}`（:643-646，消息不含 secret）；500 E1 `{"error":<配置读取失败>}`（:616）。

---

## 附录 D · `/em/` 运行时端点与 console WS 帧协议

### D.1 `/em/` 运行时 6 端点（`em_routes()` routes.rs:56-69；设备面，非管理面）

| # | 方法/路径 | 鉴权 | 请求 | 响应 / 错误形态 |
|---|---|---|---|---|
| 1 | GET `/control`（WS，**上游固件根路径**） | `token_ok`（`X-Em-Token`，config.rs:41-44 空=不校验）；**token 不匹配不立即 401**（配对 `register.pairing:true` 降级拨号，由 register 首帧裁决，routes.rs:85-104） | WS upgrade + 首帧 `register` | enabled=false → 404 `echomuse disabled`；`control_session` 三分支 refused/pending/approved（:107-310）、ack 能力协商 + config 推送（:181-197）、OWW 自动下发（:235）、wasm-policy（:257-283） |
| 2 | GET `/data`（WS，**上游固件根路径**） | `token_ok` | WS upgrade + 首帧 `identify`（5s `IDENTIFY_WAIT`，:46） | enabled 404、token 401 `"bad token"`；`data_loop` 心跳 15s（:53）、控制帧非断链信号（:561-566） |
| 3 | GET `/em/control`（WS，本项目旧版别名） | 同 #1 | 同 #1 | 同 #1（mock/文档走它，:59-60） |
| 4 | GET `/em/data`（WS，本项目旧版别名） | 同 #2 | 同 #2 | 同 #2 |
| 5 | GET `/shell/{id}`（设备回拨，`?pty=1` 可选） | 认领 `shell::claim` | 设备侧 Binary 输出 / 浏览器侧 stdin | PTY 模式 stdin 帧带 `0x00` 头；pty 分配失败省略参数退 pipe |
| 6 | GET `/em/status`（**无鉴权** G0，routes.rs:71-75 / mod.rs:376-385） | 无（头注释显式声明不写 401/token） | 无 | **200 `{"enabled":bool,"devices":[{device_id,connected_secs}]}`，关闭时仍 200 + `enabled=false`（非 404）**；devices 取 hub 当前 key = 会话中在线设备 |

> 管理台**不直接调** #1–#5（由固件拨号），不拼管理台 token、不写 Bearer；`/em/status` 是唯一管理台会调的设备面只读端点（1/2 §2、§6.4、2/2 §12）。

### D.2 console WS 帧协议（B.15 展开，console.rs + `console_frames`）

- **出站帧（服务端 → 浏览器）**：`ready{device_id,mode:"pty"|"pipe",raw_input,resize,insecure,notice}`（:208-226）→ `shell_meta{pty}`（:230-233）→ `output{text}`（:257-275，8 KiB 切片 + UTF-8 对齐 + `SESSION_BYTES_CAP` 预算；超限发 `notice{code:"output_capped",text}`）→ `closed{reason,detail,inputs,bytes_out,secs}`（:307-314）。
- **错误帧**：`error{code,error}`（:235-241 / :357-366）；`ErrorCode` ∈ `busy` / `device_offline` / `internal`（占槽 `Busy`→busy；`Offline|Unauthorized|PeerUnavailable`→device_offline，:369-377）。
- **`CloseReason`**：`client_closed` / `device_offline` / `superseded` / `device_shell_lost` / `idle_timeout`（600s 无输入）/ `session_limit`（7200s 上限）/ `internal`（:58-65 各带中文 detail）。
- **入站帧（浏览器 → 设备）**：仅 JSON 文本帧（二进制帧忽略）；`ready` 之前的 input/ctrl-c 一律丢弃（:449-450）；PTY 模式写给设备 stdin 必须带 `0x00` 帧头（:15-18，否则命令首字符被设备 `switch data[0]` 静默丢弃）。
- **生命周期约束**：`system_gate` 在 `on_upgrade` 前；占槽后发 `shell_open{pty:true}`（:56）；设备 15s 内回拨；每轮 `supervise()` 复核 control 身份（绝不用 `hub_live(id)` 现取，:279-288）；收尾五步顺序不可颠倒（:292-334）；空闲 600s / 硬上限 7200s 取早者（:500-502）。

---

## 附录 E · 计数口径澄清（「18 端点 / 11 方法」）

- **echomuse 18 端点**：按 `.route` 计 = admin.rs:37-53 的 **7**（`GET /devices`；`GET|DELETE /devices/{id}` 一条双方法；`POST /delete`；`POST /approve`；`POST /command`；`POST /settings`；`POST /music`）+ system.rs:39-47 的 **5** + oww.rs:123-132 的 **2**（`install-vad` + merge `oww_state`）+ console.rs:68-71 的 **1** + ble_http.rs:88-95 的 **3** = **18**。若按 method 计为 **19**（`GET|DELETE /devices/{id}` 拆两个）。附录 B 按 18 端点组织，双方法端点（B.2）把 GET/DELETE 两路 wire 都写全。
- **tuya「11 方法」**：逐行核对 `routes()`（api.rs:47-63）实为 **10 条 `.route` / 10 个 method handler / 9 个不同路径**（`/api/tuya/credentials` 同路径挂 POST :51 与 DELETE :53-56，另有 `POST /credentials/delete` 别名 :52）。报告 1/2 §4「10 条 `.route` / 11 方法（非 9）」与 §6.3 是相对**计划描述的「9 端点」**做对照时的计数口径（ credential 三路 POST/DELETE/别名 + 其余 7 路被分别/重复计入），源码并不存在第 11 个 method handler。**附录 C 已按源码把 10 个 method 独立成行全列，无任何 handler 被省略**——这是「零遗漏」的实质保证；请勿为凑「11」而虚构端点。
- **`/em/` 运行时 6 端点**（附录 D.1）与 `/api/echomuse/*` 18、`/api/tuya/*` 10 是**三组独立路由**，勿混算。

---

## 附录 F · 本次逐行核读对报告原文 wire 精度的补正清单

> 报告 1/2/2/2 为摘要级分组列举；以下为落盘前逐行核读源码后补齐/收紧的 wire 细节，QA 回归以本清单 + 附录 A–D 为准。

1. **music 两级 409**：`play_url`/`stop` 除 `{"error":"device offline"}` 外还有 `{"error":"data channel offline"}`（admin.rs:585-589 / :605-609）；scan 空结果带中文 `hint`（:552-558）；browse / play_url 底层失败是 400 `{"error":e}`（:569 / :594）；非法 action 400 文案逐字（:616）。
2. **settings 三类 400 文案逐字**（admin.rs:274 / :278 / :296）与 40 键白名单 + kind、官方默认值逐字（:182-236 / :171-178）全列于 B.6；`delivered` 是逐 `ControlOut` 发送结果（:489-495），不是 `settings_plan` 的 `online` 布尔。
3. **tuya devices 列表含 `total` / `online`**（api.rs:182），元素含 `updated_at`、`name` 是 alias 优先；`device_detail_payload` 的 `dps[]` 五字段（api.rs:197-203）。
4. **tuya 控制失败** `failure_payload` 带 `kind` + `advice`（api.rs:237-243），HTTP 状态三归类 offline→409 / timeout→504 / unsupported→400（lan.rs:67-73）——前端须透出 `advice` 中文原文。
5. **cloud_proxy 校验链 4 条 400 文案**（method / path 前缀 / path 含空格 ? # / body 限 POST，api.rs:272-313）+ 凭证未配置 400 文案（:621）+ 502 `{kind,error}`（kind ∈ auth/network/api，:343-345 / :638-646）+ 200 `{status,body}` 原样透传（:337-340）。
6. **BLE 13 类错误码映射**（ble_http.rs:19-49，附录 A.4）与错误体形状 `{"code","error"}`；两快照形状 `{slots,events}` / `{updatedAt,count,adverts}` + `envelope` 注入 `{device_id,session_id}`（:218-222 / :319-330）；`DefaultBodyLimit::max(4096)`（:94）；`session_id` query/body 严格核对（:311-314 / :349-352）。
7. **OWW 五态回包全字段**（oww_state.rs:116-133，附录 A.5 / B.14）+ `states` 自描述 5 值数组 + 「读不到设备一律 `unknown`」（:70-93）；install-vad 200 全字段含 `applies:"next-turn"`（oww.rs:541-550）与 500 `asset_unreadable`（:529-532）。
8. **三种错误形状并存**（附录 A.2）：admin/tuya 纯 `{"error"}`；system + `ShellError` `{"error","code"}`（+ 可选 `exit_code`）；BLE `{"code","error"}`。`system::bad_request` 带 code（system.rs:222-227）；`get_runtime` 用 **G1 而非 G2**（:88-93）；volume 两个 400 文案（:105 / :107）。
9. **`ShellError::response` 17 类**（shell.rs:63-119，附录 A.3）此前报告仅提「按 `ShellError` 映射」未列全；`record_payload` 注入 `session_id`（admin.rs:109-117）、runtime `link{authenticated,token_required}` 与 offline 全 null（runtime.rs:191-209）全列于附录 A.5。
10. **console WS 帧协议**（console.rs，附录 D.2）从「按路径/方法/鉴权粒度」补到帧级：ready / shell_meta / output / notice / error / closed 六类出站帧 + `ErrorCode` / `CloseReason` 取值 + PTY `0x00` 帧头约束 + 600s/7200s 生命周期。
11. **tuya 计数口径**（附录 E）：源码 10 method / 9 路径，「11」为对照计数差异，附录 C 已全列 10 行不省略。
12. **拍板映射**：R4 不 vendor ⇒ §7 R4 表、补充勘误「资产落点」维度、§13 `package.json`/`vite.config.ts` 拷贝步骤均可不实施，但仍需保留 em/README.md:41 白屏诚实提示；nav 第 10 项 ⇒ §13 `src/lib/nav.ts` 新增 `devicehub` key、`router.tsx` 映射；R11 只做 models/diagnostics/compat ⇒ §13 `ExtensionWorkspace` 只保留 3 个 Tab、`echomuseApi.ts` 的 music + BLE 方法本轮可不实现（wire 仍全留在 B.7 / B.16–B.18 供后续波次）。

—— 附录完。W12 研究（报告 1/2 + 2/2 + 补充勘误 + 逐端点 wire 附录）至此为**工程师实施与 QA 回归的唯一权威依据**。
