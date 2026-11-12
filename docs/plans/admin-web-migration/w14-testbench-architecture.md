# W14b / Testbench Tauri 音频桥架构深化研究 · 权威归档

> **归档说明（归档波次：W15-2）**：本文为 W12/W13/W14b 阶段**研究原文归档**，记录当时旧端权威与拍板依据，保留史实；当前实施口径见 [W15 资产盘点](<./w15-legacy-assets-research.md>) 与 [W15 构建链](<./w15-production-build-research.md>)，管理台当前为 `client/admin-web`（见 [admin-web 指引](<../../../client/admin-web/AGENTS.md>)）。

> **文档性质**：W14b / Testbench 音频桥架构的**唯一权威契约**，供 W14b-1 / W14b-2 实施与 QA 回归逐节核对。由研究阶段发送给 team-lead 的「报告 1/2 + 报告 2/2」**原文全量合并**而成，研究原文一字未删。
> **研究纪律**：研究全程只读——未创建/修改任何源码文件、未执行构建、未做 git 操作；本归档是经 team-lead 授权写入的单一文档。
> **证据口径**：全部结论带 `file:line`；外部结论统一标注「（WebSearch 2026-11-11，外部：来源）」。根路径 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/`；报告内 `xxx:行` 均相对该仓库根。
> **架构结论（先看）**：路径 **X（Tauri 本地 loopback WS 服务桥）** 可行且推荐；**Y（内嵌 SPA + 绝对 URL API）** 四处撞墙、**Z（remote.urls）** 维持否决。决定性事实：音频二进制帧的起点/终点始终是 server `/api/ws`，与本地桥无关；本地桥只承载 Tauri Rust 音频硬件 ↔ 前端的 PCM/Opus 数据，前端持有唯一 `/api/ws` 会话做协议编排。死结只针对 Tauri IPC（`invoke` / 插件 IPC），不针对标准网络请求（fetch / WS）——本地桥以标准 WS 彻底绕开。

## 已确认采用的架构组合（team-lead 确认，2026-11-11）

| 编号 | 架构项 | 采用结论 |
|---|---|---|
| 1 | 架构路径 | **X 本地 loopback WS 桥**（Y 不采用、Z 维持否决） |
| 2 | 音频帧编排 | **子路径 B 前端编排**：前端持唯一 `/api/ws` 会话，Rust 只做音频编解码 I/O |
| 3 | 端口传递 | **`WebviewWindowBuilder::initialization_script()` 注入端口常量**（不经 IPC / 不受 ACL） |
| 4 | 本地桥协议形态 | **全 WS**（无需 CORS；必须校验 `Origin`/`Host`、只绑 loopback） |
| 5 | 音频实现 | **全 Rust cpal + audiopus + cpal output**（不引 rodio、不写 Swift/ObjC 桥） |
| 6 | CSP | **维持 `csp: null`**（本波不补） |
| 7 | 权限与 bundle | 正式引用 `Info.plist`（补 `NSAllowsLocalNetworking`）/ `Entitlements.plist` + 签名／沙箱复核 |
| 8 | capability | 路径 X 全走本地桥 → **不新增 capability** |
| 9 | 波次 | **拆 W14b-1（上行 / ASR）+ W14b-2（下行 / 播放）** |
| 10 | 范围边界 | **本波不做 Windows、不承诺跨机 LNA 结论**（只能真机给） |

> 以上为**实施依据**。§5 十项拍板清单是研究阶段的待拍板表述，team-lead 已按推荐组合确认采用；下方「报告 1/2」「报告 2/2」为**研究原文全量保留、零删减**，如遇口径冲突，以原文逐行核读的源码 `file:line` 为准。**外部结论（WebSearch 2026-11-11）在原文中均已标注来源**。

## 目录与合并顺序

| 区段 | 内容 | 来源 |
|---|---|---|
| 报告 1/2 原文（§1.0–§2） | §1.0 ACL 死结边界、§1.1 路径 X 全量核死（CSP / mixed-content、ATS / `NSAllowsLocalNetworking`、`initialization_script`、Origin 校验、Chrome LNA 真机风险、CORS 分工、端口 / 生命周期）、§1.1④ 双 WS 禁忌（`test_buf` 会话分裂）与完整数据流图、dev 门控、§1.2 路径 Y 四处撞墙、§1.3 路径 Z 否决依据、§2 音频实现路径对比（全 Rust cpal+audiopus vs macOS 原生 Swift 桥） | 报告 1/2 原文 |
| 报告 2/2 原文（§3–§5） | §3 三层验收策略（L1 编译 / 单测、L2 前端 mock、L3 macOS 真机清单 8 项 + 诚实化锚点）、§4 工作量分解 M1–M11 与拆波建议、§5 十项拍板清单（含推荐与理由）、收尾声明 | 报告 2/2 原文 |

---

---

> **以下为「报告 1/2」原文，全量保留，未删减。**

---

W14b / Tauri 音频桥架构深化研究 **报告 1/2**（1/2 = 三条架构路径逐个核死 + 音频二进制帧通道澄清 + 音频实现路径对比；2/2 = 验收策略 + 工作量分解/波次建议 + 需拍板清单）。

路径基准：以下 `xxx:行` 均相对仓库根 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust`。外部结论统一标注「（WebSearch 2026-11-11，外部：来源）」。全程只读。

**一句话结论：路径 X（Tauri 本地 loopback 服务桥）可行且推荐，Y 有 4 处撞墙、Z 维持否决；决定性事实是「音频二进制帧的终点/起点始终是 server `/api/ws`，与本地桥无关」，而本地桥用标准 WS 完全绕过 Tauri IPC 的 ACL 死结。**

---

## §1.0 前置：ACL 死结的准确边界（三条路径的共同起点）

- `client/admin-web/src-tauri/capabilities/default.json:4` 逐字：「...仅对 Local origin（内嵌 tauri://localhost）默认可用；生产默认加载的是远程服务器页面（如 http://127.0.0.1:8000），origin 为 Remote，IPC 默认被 ACL 拒绝，故 app_capabilities 与剪贴板插件 IPC 都不会成功。」
- `client/admin-web/AGENTS.md:70`、`:89`、`:106-112`；`client/admin-web/src/lib/platform.ts:53` 逐字「IPC 通道不可用（远程 origin 未授权 remote.urls）」。
- **关键边界：死结只针对 Tauri IPC（`invoke` / 插件 IPC），不针对标准网络请求（fetch / WebSocket）。** 这是路径 X 成立、也是重评 Y/Z 的共同前提。

---

## §1.1 路径 X：Tauri 本地 loopback 服务桥 —— 可行（推荐）

### ① CSP / mixed-content 对 http://127.0.0.1 的实际拦截面
- `client/admin-web/src-tauri/tauri.conf.json:17` `"csp": null` → **当前无 CSP，无 CSP 拦截**；`:13` `withGlobalTauri: false`。
- http 页面 → http/ws：**不触发 mixed content**（外部：websocket.org/guides/troubleshooting/cors；sysseder.com 2026-05「An HTTP page requesting HTTP is not automatically 'mixed content'」）。管理台页面是 `http://LAN-IP`，连 `ws://127.0.0.1:<port>` 属 http→ws，协议不降级。
- **WebSocket 不受 CORS**（外部：websocket.org「WebSocket connections bypass this entirely ... browser ... never checks the response for CORS headers」；localtonet.com）。
- **但发现 WKWebView 专属拦截面（本波新增，必须登记）**：外部 BloopAI/vibe-kanban PR #3053 逐字「macOS 15 tightened App Transport Security enforcement for WebSocket connections in WKWebView. The per-domain ATS exceptions for localhost and 127.0.0.1 didn't fully cover WebSocket upgrade requests. → Add `NSAllowsLocalNetworking` to Info.plist so ATS permits all local network connections including WebSocket upgrades」「Switch server URL from 127.0.0.1 to localhost for reliable ATS domain matching (IP literals aren't matched by ATS exceptions)」「On modern macOS, localhost resolves to ::1 (IPv6) first ... Binding to localhost:0 ensures the server listens on the same address family」。→ 现有 `Info.plist:10-17` **只有** `NSMicrophoneUsageDescription` / `NSLocalNetworkUsageDescription` / `NSBonjourServices`，**缺 `NSAllowsLocalNetworking`**；且需评估 IPv6 双栈与 `AGENTS.md:10` 既有「devUrl 用 127.0.0.1 避免 localhost IPv6 解析不一致」的取舍。
- **CSP 若将来补则必拦**：外部 incompl/pudding e2e README 逐字「Production CSP (connect-src 'self' ipc: http://ipc.localhost) blocks the loopback WebSocket, and we don't weaken it ... overrides only the CSP to also allow `ws://127.0.0.1:*`」；外部 opendeep.wiki/QwenPaw 给了可用范式 `"connect-src": "'self' http://127.0.0.1:* ws://127.0.0.1:* ipc: http://ipc.localhost"`。即「确认无其他拦截」的例外：一旦补 CSP，必须手写放行 `ws://127.0.0.1:*`（Tauri 自动注入只补 ipc:/asset:，不补 loopback WS）。
- **Chrome LNA（2025-2026 新权限层，头号真机风险）**：外部 Chrome 开发者文档「區域網路存取權限制也控管 WebSocket 和 WebTransport 連線」、Chrome 147 起对 WS 有 local-network 权限提示；localtonet.com 逐字「Chrome limits LNA permission to secure contexts ... A plain HTTP public origin cannot rely on receiving an LNA permission prompt and should be treated as unsupported for a production direct-local workflow」。我们页面是 `http://LAN-IP`（非 secure context）→ 普通 Chrome 里连 loopback WS 可能被权限提示/拦。但 **Tauri macOS 是 WKWebView（非 Blink），WKWebView 是否跟进 LNA 本机无 GUI 无法验证，只能真机给**（`AGENTS.md:208`）。

### ② CORS：server 与本地桥各自需要什么
- **server 侧：零改动、零 CORS**。`server/Cargo.toml:64` `tower-http = { version = "0.6", features = ["fs"] }` —— 只有 `fs`，**无 `cors` feature**；全仓 grep `CorsLayer|Access-Control|allow_origin` 只命中 `gateway/ws/mod.rs:59` 的 `ServeDir, ServeFile`（静态托管）与涂鸦云私有 `mode=cors` 签名（无关）。`AGENTS.md:15` / `:58` 逐字「Rust server ... 没有 CORS 层」。生产页面由 server 同源托管，前端相对 `/api` 天然同源（`api.ts:8-10`）。
- **本地桥**：
  - 若控制+数据**全走 WS** → 无需任何 CORS（WS 不受 CORS）→ 推荐。
  - 若控制面用 HTTP `fetch`（POST start/stop）→ 跨源 fetch（`http://LAN-IP:8000` → `http://127.0.0.1:<port>`）需本地桥返回**精确** `Access-Control-Allow-Origin`（不可 `*`，本地桥握有麦克风能力）+ 处理 OPTIONS 预检。
  - **安全强制（外部 websocket.org + sysseder 均强调）**：WS 服务端必须校验 `Origin` 头（防 CSWSH / DNS rebinding），并只绑 `127.0.0.1`/`::1`；本地桥不经过 server 的 `dev::guard`，须自己兜住这一层。

### ③ 端口选择 / 冲突 / 生命周期
- 固定端口会冲突 → 推荐 `TcpListener::bind("127.0.0.1:0")` 由 OS 分配空闲端口。
- **端口如何告知前端是路径 X 的关键卡点**：生产服务器页面模式下 `invoke("bridge_port")` 会被 ACL 拒绝（§1.0），不能用 command。外部 docs.rs `WebviewWindowBuilder::initialization_script()` 逐字：「run after the global object has been created, but **before the HTML document has been parsed and before any other script** included by the HTML document is run」「executed only on the main frame」「recommended to check the `window.location` to guard your script from running on unexpected origins」，官方示例 `window.__MY_CUSTOM_PROPERTY__ = { foo: 'bar' };` + origin 守卫；外部 ChanYikLam/MyReader SPEC 证实 macOS 上是 WKUserScript documentStart。`AGENTS.md:70` 已证 init script **无条件注入**（与 `withGlobalTauri:false` 无关）。→ **用 `initialization_script` 注入端口常量，完全不经 IPC / 不受 ACL**，且必须加 origin 守卫防泄露到其他导航。行业范式对照：QwenPaw 用 `backend_port` command、pudding 用 `e2e_port` command（二者无 ACL 问题，故我们不能照抄 command 形式）。
- 生命周期：随 app 在 `src-tauri/src/lib.rs:103-111` 的 `setup` 内起 tokio runtime + axum（推荐，端口早定便于注入），app 退出时 abort；懒启动会让端口注入变复杂，不推荐。

### ④ 与 `/api/ws` 测试帧通道的分工（本节核心，回答「音频采集编码在哪一层做」）
**先把通道钉死：**
- `server/src/gateway/ws/mod.rs:154` `.route("/api/ws", get(ws_handler))`；`:155-156` 逐字注释「`/api/ws` 已被小智设备协议占用，管理台走独立路径 `/api/admin/ws`」。`/api/ws` 是**唯一**测试会话 WS 入口。
- `ws_handler` `mod.rs:367` 先 `auth_ok`；`:377-396` `expected_token` 空 → true（局域网默认可管理），否则 Bearer 头或 `?token=`；`:391-392` 逐字「浏览器 WebSocket 无法自定义请求头，允许 ?token= 查询参数」→ **前端直连必须把 token 拼进 query**。
- `is_test` 来自 hello.test（`server/src/workflow/session/start.rs:25-26`、`:46`、`:233`）；`accept_test_only`（`session/mod.rs:543-554`）；四 match 臂（`:611-639`）；`bench.rs:1-7` 逐字模块文档「测试台（macApp，`hello.test=true`）四个专用端点」。
- **上行音频二进制帧确实打进 `/api/ws` 同一条 WS**：`server/src/workflow/session/media.rs:133-145` `handle_binary` → `unwrap_uplink` → `uplink_decoder.decode_frame(payload)` → `handle_pcm`；`:147-151` `if self.test_recording { self.test_buf.extend_from_slice(pcm) }`；`bench.rs:28-80` `asr_test` start 置 `test_recording=true` / stop 取 `test_buf` 整段识别 → `Stt`（`:75`）。下行：`bench.rs` 多处 `TtsTestResult` 文本帧（`:126/:141/:177/...`）+ 既有 downlink 链路推 Opus 二进制帧（W14 已核）。
- **结论：音频帧起点/终点始终是 server `/api/ws`，与本地桥无关。本地桥只承载「Tauri Rust 音频硬件 ↔ 前端」的 PCM/Opus 数据与控制，绝不承载测试协议帧。**

**两条子路径对比（team-lead 问的 X1/X2）：**
| 子路径 | 内容 | 判定 |
|---|---|---|
| A. Rust 全托管 server WS | Rust 实现 WS 客户端 + 完整测试协议（hello/asr_test/tts_test/llm_test/nlu_test 文本帧）+ 拿 token + 全部状态回传前端 | **不推荐**：token 在前端 sessionStorage（`api.ts:68/:78-109`），Rust 拿不到；直接违背 `src-tauri/src/lib.rs:3-4` 逐字「业务逻辑一律留在前端 React SPA 中，不往 Rust 侧搬」；复杂度最高 |
| B. 前端持唯一 `/api/ws` 做协议编排，Rust 只做音频编解码 I/O | 前端是协议中枢（发 hello/测试文本帧、转发 Opus 帧、收 Stt/TtsTestResult），本地桥只是二进制数据管道 | **推荐**：贴合旧端 Kotlin(`TestBenchState.kt` 编排)/ObjC(`XiaoZhiModule+Audio.m` I/O) 分层；token 注入复用 `getAuthToken()`；Rust 不懂协议；无需新增 capability |

**必须写明的坑**：不能「本地桥代发 Opus 到 server WS」+「前端另开一条 `/api/ws` 发文本帧」——`test_buf` 是会话级字段（`media.rs:150` `self.test_buf`），两条 WS = 两个 session_id / 两个 hello.test，缓冲分裂，ASR 永远录不到完整段。

**完整数据流（供实施用）：**
```
麦克风 → cpal 48k PCM → 重采样 16k → Opus 编码(60ms)        [Tauri Rust]
   → ws://127.0.0.1:<port> 本地桥 (binary)
   → 前端 JS (ArrayBuffer 中转；~300B/帧、~17 帧/s，开销可忽略)
   → ws(s)://<server>/api/ws?token=... (同源, binary)
   → server media.rs decode → test_buf → asr_recognize → Stt
   ← Stt / TtsTestResult 文本帧 + Opus 二进制帧
   → 本地桥 (binary) → Opus 解码 → 重采样 → cpal output 播放
```
**dev 模式限制（诚实化）**：`server/src/dev.rs:97-105` blocked 含 `/api/ws`；`:107-143` guard 返回 503 `dev_inference_disabled`（`:16` DISABLED 逐字「开发模式仅运行管理台与真实管理 API，未启用模型推理、语音会话或训练」）。**本地桥不经 server 路由，dev 下能起来，但音频帧终点 `/api/ws` 被禁 → `--dev` 下测试台整体仍不可用**；W14 结论在本架构下继续成立。

---

## §1.2 路径 Y：内嵌 SPA + 绝对 URL API —— 不推荐（4 处撞墙）

① **server 无 CORS 层**：`server/Cargo.toml:64` 只有 `["fs"]`、全仓无 CorsLayer；`AGENTS.md:15/:58` 逐字「没有 CORS 层」。`tauri://localhost` → `http://<server>:8000` 跨源 fetch 需 OPTIONS 预检 + ACAO。要么给 server 加 `CorsLayer`（改 Rust server 源码 + Cargo.toml feature，`AGENTS.md:211` 逐字「Rust server 源码改动与 CORS 层改动」列为明确未做），要么装 `@tauri-apps/plugin-http`（外部 cmsj.in 证实「跳过 CORS 限制」；但 `AGENTS.md:125` 逐字「本轮也不装 http-client ... `src/lib/api.ts` 保持原生 fetch 相对路径，不改成插件请求」，且要改全部 `*Api.ts` 请求实现）。
② **与既有架构直接冲突**：`src-tauri/src/lib.rs:19-24` 注释 + `AGENTS.md:56-69` 方案 B；`api.ts:11-13` 逐字「内嵌 SPA 预览 = **API 不可达**」。`decide_window_url` `lib.rs:75-77` embedded 分支**优先于 remote 并直接返回 App**（`:234-240` 单测逐字「显式内嵌开关优先于服务器地址」）→ **内嵌模式下 `XIAOZHI_ADMIN_URL` 不生效**，绝对地址只能靠编译期 `VITE_API_BASE_URL`（`api.ts:41-47`、`:15-17`）。等于推翻 W0 以来「生产默认加载服务器页面」基线。
③ **「Tauri 壳连远端 server」是否必须同机 —— 纠正 team-lead 的预设**：**不必须同机**（内嵌 SPA 不加载任何服务器页面，API 绝对 URL 可指任意 B 机），但**该场景反而更难过**：`api.ts:85` `getAuthToken()` 从 `window.location.search` 读 `?token=`，`tauri://localhost/index.html` 无 query → **token 输入来源断**，需另设入口；`getWebSocketUrl()` `:145-155` 绝对 baseURL 固定落到 `/api/admin/ws`，而测试台要的是 `/api/ws`（设备协议），需新增独立推导（W14 两套通道）。跨机 + 跨源 + token 断 + 两套 WS，全面劣于路径 X（页面天然同源、本地桥恒在壳本机、录本机麦克风）。
④ **IPC 收益不成立**：内嵌模式唯一卖点是 Local origin 下 `invoke` 可用（`AGENTS.md:89`、`platform.ts:37`），但硬件能力完全可由本地桥提供且**不需要 ACL**，为拿 IPC 而推翻架构不划算。

---

## §1.3 路径 Z：remote.urls 重评 —— 维持否决

- 外部（WebSearch 2026-11-11）Tauri **v3** 官方 `https://v3.tauri.app/security/capabilities/`「Remote API Access」仍是 `"remote": { "urls": ["https://*.tauri.app"] }`，示例仅子域通配，与 `AGENTS.md:108` 引用的 v2 官方原文一字不差 → **无新语法 / 无新版本机制可安全限定局域网 IP**。
- `AGENTS.md:109-110` 三重依据仍成立：`tauri-utils-2.10.2/src/acl/capability.rs:237-249` URLPattern、`acl/mod.rs:280-321` RemoteUrlPattern、`urlpattern 0.6.0` 离线实测（`http://192.168.*.*:8000` 等不匹配 LAN IP；`http://*:8000` 等价任意 HTTP 源）；`:111` 结论「不配 `remote.urls`」。
- 且配 remote.urls 只恢复 IPC，音频数据流仍要解决（= X 的超集 + 信任边界扩大到任意 HTTP 源），绕远路。外部 zudo 文档给的 `"http://localhost:*/**"` 是 dev 用法（`AGENTS.md:110` 已证 localhost 通配覆盖不了 LAN IP），不构成新解。

---

## §2 音频实现路径对比（Rust 侧）

**全 Rust：cpal + audiopus + cpal output（推荐）**
- cpal（外部 lib.rs / docs.rs / crates.io，2026-11-11）：最新 `0.18.2`（2026-08-16），macOS **CoreAudio 默认后端**、MSRV 1.85、**最低 macOS 14.2**（loopback 录制需 14.6+）；Windows WASAPI（Win8+）；Linux 需 ALSA 开发库。本机 Darwin 27.2.0 满足。
- 项目已有可复用经验：`server/Cargo.toml:102` `audiopus = { version = "0.2", optional = true }`、`:33` `sherpa = ["dep:sherpa-onnx", "dep:audiopus"]`、`:100` 注释逐字「仅在 sherpa feature 下引入（需要网络下载原生库 + 系统 libopus）」。Tauri crate 独立，需自行在 `src-tauri/Cargo.toml` 加，但参数口径可与服务端对齐。
- **Opus crate 编译现状（外部，修正预设）**：`audiopus_sys` 是 libopus FFI，「By default, we **statically link to Windows, MacOS** ... link dynamically for Linux」、构建需 **cmake**（pkg-config 优先）；Windows 有 vcpkg 路径（`choco install pkgconfiglite` + `vcpkg install opus:x64-windows` + `PKG_CONFIG_PATH`，外部 opus-rs），且社区 2025-10 仍有「Fix build for Windows by forcing release mode on libopus」fork（外部 mattlekrupa/audiopus_sys）。→ **Windows 不是必须「预编译 libopus」**，而是「需要 cmake/MSVC 工具链或 vcpkg 预装，且存在已知构建坑」，仍如实记为将来 Windows 支持的已知成本（不在本波范围）。
- **重采样是易被低估的一段**：cpal 采集通常 48k（旧端 `client/apps/macosApp/XiaoZhiModule+Audio.m:130` 逐字「通常 48000 → 2880」），协议要 16k 上行 / 24k 下行（`XiaoZhiModule.m:11-13`、W14 hello `audio_params`）。旧端 `+Audio.m:163-215` 手写 48k→24k；Tauri 侧需 48k→16k（上行）与 24k→输出设备（下行），要自写或引 rubato/dasp。
- **播放**：cpal output stream 直接喂 f32 PCM（流式，贴合 60ms 帧）；rodio 面向文件/解码，引入多余抽象，**不推荐**（外部 cpal README 也只把 rodio 当 higher-level 备选）。
- **波形**：从采集 PCM 算 RMS/峰值经本地桥推前端（旧端 `XiaoZhiModule.m:372-420` `getAudioState`/波形）。

**macOS 原生 Swift/ObjC 桥（不推荐）**
- 旧端 macApp 是 **Mac Catalyst**：`client/apps/macosApp/project.yml:33` `SUPPORTS_MACCATALYST: "YES"`、pbxproj `:396/:470`；`#if TARGET_OS_MACCATALYST` 的 `AVAudioSession` 分类切换见 `XiaoZhiModule+Audio.m:31-35`、`:51-66`、`:113-120`（`:111-112` 逐字「实测（本机 Catalyst 复现）在 Playback 分类下触碰 inputNode 会让引擎直接 startAndReturnError 失败」），`AppDelegate.m:14-15` 亦有 `AVAudioSession`。
- **Tauri 2 macOS 是 WKWebView + AppKit 原生窗口，非 Catalyst**；`AVAudioSession` 是 iOS/tvOS/Catalyst API → **旧端这套代码在 Tauri 原生 macOS 上整段不适用，必须重写，不能照搬**（印证 team-lead 预设）。走原生桥需 `objc2`/`objc2-av-foundation` 绑定 + FFI，仅 macOS 一份，Windows 还要 WASAPI 第二实现 → 双实现，工作量 ≥ 全 Rust。
- **结论：cpal 在 macOS 成熟 → 全 Rust 可免双实现，并天然为 Windows(WASAPI) 铺路。** 保留意见：cpal 在 Tauri macOS 真机的全双工稳定性 + 麦克风 TCC 授权，只能真机给（`AGENTS.md:208`）。

---

2/2 接续：验收策略（三层 + 明确哪些只能真机给 + 交付用 macOS 真机清单）、模块级工作量分解、是否拆波 W14b-1/W14b-2 的建议、以及「需 team-lead/用户拍板的架构项」清单（含推荐排序与理由）。

---

---

> **以下为「报告 2/2」原文，全量保留，未删减。**

---

W14b / Tauri 音频桥架构深化研究 **报告 2/2**（验收策略 + 工作量分解/波次建议 + 需拍板清单）。路径基准同 1/2（相对仓库根 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust`）。

---

## §3 验收策略（本机无 GUI/真机，`AGENTS.md:208` 逐字「GUI 启动 `pnpm tauri dev` / `pnpm tauri build`；编译侧只做 `cargo check` 与 `cargo test`」）

**L1 编译 / 单测层（本机可做：`cd src-tauri && cargo check` / `cargo test`，`AGENTS.md:14`）**
- 纯函数状态机单测（**不依赖硬件**）：60ms / 16k PCM 帧切分、48k→16k 重采样边界（尾帧、非整数倍）、Opus 编码→解码往返、帧封装对齐 `server/src/workflow/protocol/protocol.rs` 的 `unwrap_uplink`、`test_buf` 缓冲模拟（对应 `media.rs:147-151`）、超时状态机（30s ASR / 60s TTS / 10s NLU，对齐 `client/shared/src/macosArm64Main/kotlin/com/xiaozhi/admin/TestBenchState.kt:241-253/:259-304/:392-437`）。
- 本地桥单测：axum WS 帧路由、`Origin`/`Host` 校验拒绝路径、`bind(127.0.0.1:0)` 端口分配、健康检查、init script 端口常量注入格式。
- 既有 11 个单测保持不回归（`src-tauri/src/lib.rs:116-260`，URL scheme 白名单 + 窗口三分支）。
- 诚实提示：`AGENTS.md:208` 逐字「Tauri 首次编译会下载并编译大量依赖，耗时明显长于前端构建」。

**L2 前端 mock 层（本机可做，无浏览器）**
- 双 mock WS（本地桥 + server `/api/ws`）驱动 tab 状态机：连接、hello.test、录音 start/stop、Opus 帧转发顺序、`Stt`/`TtsTestResult`、播放、超时兜底、权限拒绝、缓冲不可用。
- `pnpm typecheck` / `pnpm build`（`AGENTS.md:11-12`）、`pnpm format:check`（`:22`）本机可做。
- 新增封装沿用 api 封装检查单的 fail-closed 双路径（`AGENTS.md:172`）：`SyntaxError → UnavailableError` 归一 +「JSON 解析成功但形状不符 → UnavailableError」。

**L3 真机手动清单（交付时给用户一份 macOS 真机验收清单；以下结论只能真机给）**
1. 麦克风 TCC 授权弹窗 + 拒绝后逐字文案（`src-tauri/Info.plist:10-11` `NSMicrophoneUsageDescription`「小智管理台需要使用麦克风进行设备语音调试」、`Entitlements.plist:11-12` `com.apple.security.device.audio-input` 正式引用是否生效）。
2. **WKWebView ATS 是否拦 `ws://127.0.0.1` 升级** → 验证 `NSAllowsLocalNetworking` 是否必需、`localhost` vs IP literal / IPv6 `::1` 双栈绑定（1/2 §1.1① 新增风险）。
3. cpal CoreAudio 全双工真实延迟 / 爆音 / 48k→16k 重采样质量。
4. Opus 60ms 帧经 JS 中转（本地桥 ↔ `/api/ws`）是否引入抖动 / 丢帧。
5. 端口冲突、多实例、app 退出后端口释放。
6. **远程场景**：页面 `http://LAN-IP` → 本地桥 loopback WS 是否被浏览器 LNA / WKWebView 权限拦（1/2 §1.1① 头号风险）。
7. 波形 / 试听真实听感。
8. `--dev` 下确认测试台整体不可用（`server/src/dev.rs:97-105`、`:16`），不误报成功。

**诚实锚点（逐字）**：`src-tauri/src/commands/mod.rs:11`「命令可用或页面返回 200 不等于真机硬件通过，硬件结论必须由真机测试给出」；`AGENTS.md:93`「接口返回 200 或页面存在不等于真机硬件通过，结论必须由真机测试给出；Web 不露出任何硬件动作」；`src/pages/TestbenchPage.tsx:88`「接口可用 ≠ 真机通过」；W14 的「测试≠功能正常」链（`benchmark/AGENTS.md:9-11`、`NodeTestPanel.kt:93-94`、`NodeTestsState.kt:104`）。
**明确：编译通过 / `cargo test` 通过 / mock 通过 / 接口 200，一律不等于真机硬件通过。**

---

## §4 工作量分解与波次建议（按路径 X 推荐方案，给模块与依赖、不虚报人天）

| 模块 | 内容 | 依赖 |
|---|---|---|
| M1 本地桥骨架 | tokio/axum WS + `bind(127.0.0.1:0)` + `initialization_script` 端口注入 + Origin/Host 校验 + 生命周期 | 无（先决） |
| M2 采集/重采样 | cpal input 48k → 16k | M1 |
| M3 Opus 上行编码 + 帧封装 | audiopus，16k / 60ms / 40kbps，对齐 hello 与 `protocol.rs` | M2 |
| M4 下行解码 + 播放 | Opus 解码 → 24k → cpal output stream | M1 |
| M5 波形状态机 | RMS/峰值 → 本地桥 | M2/M4 |
| M6 权限正式引用 | `Info.plist` 补 `NSAllowsLocalNetworking` + 麦克风键生效；`Entitlements.plist` 引用 + `com.apple.security.app-sandbox` 组合复核（模板 `:4-5` 逐字「启用前需按实际签名／沙箱策略复核」）；`tauri.conf.json` 增 bundle macOS plist 引用 | 无（可并行） |
| M7 capability | **路径 X 全走本地桥 → 不需新增 capability**，`capabilities/default.json:6` 保持只 `clipboard-manager:allow-write-text` | 无（相对 Y/Z 的额外优势） |
| M8 前端接线 | 连接 tab + ASR + TTS + 试听/波形；LLM/NLU 两 tab 纯文本帧、0 硬件 | M1 |
| M9 协议编排 | 唯一 `/api/ws` 会话 + `?token=`（`gateway/ws/mod.rs:391-392`）+ Opus 帧中转 + 文本帧处理 | M1 |
| M10 诚实化文案 | 权限拒绝 / 设备不可用 / 缓冲不可用 / 超时 /「测试≠功能正常」 | M8 |
| M11 验收 | L1 单测 + L2 前端 mock + L3 真机清单文档 | 各模块 |

**波次建议：建议拆两波（推荐采纳 team-lead 的 W14b-1 / W14b-2 设想）**
- **W14b-1 = M1 + M2 + M3 + M5(上行) + M6 + M7 + M8(ASR tab) + M9(上行) + M10 + L1 + L2**：先证伪/证实「路径 X 是否真的绕开 ACL 死结、且 Opus 能打上 server `/api/ws`」这一**最大架构风险**。
- **W14b-2 = M4 + M5(下行/波形) + M8(TTS/试听) + M10 + L3 真机清单**。
- LLM / NLU 两 tab 是纯文本帧、0 硬件、风险最低，可并入任一波或单独极小波。
- 理由：上行/下行是两个独立 Opus 方向；若 W14b-1 真机失败（ATS / LNA 拦 loopback WS），不会连带浪费下行实现；且 W14b-1 只需 L1/L2 即可先产出架构结论，不必等真机。

---

## §5 需 team-lead / 用户拍板的架构项（含推荐排序与理由）

**推荐排序：X > Y > Z（Z 维持否决）**。核心理由：X 用标准 WS 彻底绕开 IPC ACL，不改 server、不改 `api.ts`、不改生产默认服务器页面架构（方案 B）、不需新增 capability；Y 需 4 项改动且全撞 `AGENTS.md` 已明确「不做」清单；Z 只恢复 IPC 不解决音频流且扩大信任边界。

| # | 拍板项 | 我的推荐 | 理由 / 依据 |
|---|---|---|---|
| 1 | 架构路径 X / Y / Z | **X** | 1/2 §1.1–§1.3 全量核死 |
| 2 | X 子路径：前端持唯一 `/api/ws` 做协议编排 vs Rust 全托管 WS | **前端编排（子路径 B）** | `src-tauri/src/lib.rs:3-4`「业务逻辑一律留在前端」、token 归属 `api.ts:68/:78-109`、`test_buf` 会话唯一性（`media.rs:150`）；A 还拿不到 token |
| 3 | 端口传递机制 | **`initialization_script` 注入** | 生产服务器页面下 IPC 被 ACL 拒，`invoke("bridge_port")` 不可用；init script 无条件注入（docs.rs 官方 + `AGENTS.md:70`），需加 origin 守卫 |
| 4 | 本地桥协议形态 | **全 WS** | WS 不受 CORS（websocket.org）；HTTP 控制面需本地桥自返精确 ACAO + OPTIONS |
| 5 | 音频实现 | **全 Rust cpal + audiopus** | cpal macOS CoreAudio 成熟（外部 0.18.2 / 最低 macOS 14.2）；Tauri 非 Catalyst，旧端 `AVAudioSession` 代码整段不适用，原生桥需重写且 Windows 双实现 |
| 6 | 播放 / Opus crate 选型 | **cpal output（不引 rodio） + 复用 server 已用 `audiopus`** | 流式 PCM 直喂最贴合 60ms 帧；`server/Cargo.toml:102` 已有 audiopus 经验，口径一致 |
| 7 | 权限与 bundle 配置 | **接受正式引用**：`Info.plist` 补 `NSAllowsLocalNetworking`、`Entitlements.plist` 引用、复核签名/沙箱 | 当前 `Info.plist` / `Entitlements.plist` 是**未被 conf 引用的预留模板**（`AGENTS.md:92`、模板 `:3-6`）；ATS 拦 WS 升级需该键（外部 vibe-kanban） |
| 8 | CSP | **维持 `csp: null`（本波）**，若将来补 CSP 必须加 `connect-src ... ws://127.0.0.1:*` | `tauri.conf.json:17` 当前不拦；pudding 案例证补 CSP 后 loopback WS 会被拦，属安全/复杂度取舍 |
| 9 | 是否拆波 | **拆 W14b-1（上行/ASR）+ W14b-2（下行/播放）** | 先验证最大架构风险，失败不连带下行实现 |
| 10 | 范围边界（两项） | **本波不做 Windows、不承诺跨机 LNA 结论** | Windows 需 cmake/MSVC 或 vcpkg + 已知构建坑（外部 opus-rs / audiopus_sys fork）；跨机 `http://LAN-IP` → loopback WS 的 LNA 结论只能真机给（`AGENTS.md:208`）。若用户要求本波支持，需把这两项列入真机验收范围并相应加工作量 |

---

**收尾声明**：本波全程只读 —— 未改任何文件、未构建、未执行 git 操作；WebSearch 仅用于 Tauri 生态/官方文档/浏览器安全层核实，结论均已标注来源与日期（2026-11-11）。1/2 + 2/2 两段即本波完整交付，无未报告的附带结论。等待 team-lead 下一步派发（预期为 W14b 实施规格或本波报告归档落盘）。
