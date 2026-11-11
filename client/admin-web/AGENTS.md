# client/admin-web 开发指引

React + shadcn/ui + Tauri 2 管理台。**运行环境已就绪，业务页面正在分批迁移**：Overview / Devices / Console / Files 已接入真实 API，**Config 18 段已全部迁移（C0–C7）**，**Plugins 已迁移（含 A2UI 运行页与动态侧栏入口，nav 现 9 项）**；其余页面（Settings / Echomuse / devicehub / Testbench）仍是占位骨架（排期见「页面迁移（已启动）」）。业务逻辑一律留在前端 SPA，Rust 侧只有桌面壳与窄命令骨架。本目录不写 README.md，说明统一收敛到本文件。

## 工具链与命令

- 已验证：Node v22.22.2 / pnpm 12.15.0 / npm 10.9.7 / cargo 1.98.1 / Xcode。Vite 8 要求 Node `^20.19 || >=22.12`，本机满足。
- **不要用 yarn**（本机 corepack 会交互卡死）；下列 pnpm 命令均在 `client/admin-web` 目录执行。
- `pnpm install`：安装依赖。`pnpm-lock.yaml` 由真实安装生成；pnpm 12 会额外生成 `pnpm-workspace.yaml` 记录 `minimumReleaseAgeExclude`，属正常产物，不要删除。
- `pnpm dev`：Vite dev server，`http://127.0.0.1:5173`（`strictPort: true`；`server.host` 固定 `127.0.0.1`，与 `tauri.conf.json` 的 `devUrl` 一致，避免 `localhost` 在 IPv6 `[::1]` 与 IPv4 `127.0.0.1` 之间解析不一致导致 Tauri 连不上；不要改成 `true` 监听所有网卡）。
- `pnpm build`：`tsc` 双 project 类型检查 + `vite build`，产物 `dist/`（含 `dist/index.html`）。
- `pnpm preview`、`pnpm typecheck`。
- `pnpm tauri dev` / `pnpm tauri build`：Tauri CLI 走项目 devDependency `@tauri-apps/cli`；本机没有全局 `cargo-tauri`。
- `cd src-tauri && cargo check` / `cargo test`：Rust 侧编译校验；单测覆盖 URL scheme 白名单与窗口加载模式决策（dev / 生产服务器页面 / 内嵌预览三种分支）。
- 开发期先启动 Rust server（默认 `http://127.0.0.1:8000`），否则 `/api`、`/ws`、`/em/` 代理会失败。 Rust server 当前无 CORS 层，因此开发/生产都依靠同源访问；生产桌面端默认同样加载该服务器页面（见「Tauri 三种加载模式」）。

## 目录结构

```
client/admin-web/
  AGENTS.md            本说明
  package.json  pnpm-lock.yaml  pnpm-workspace.yaml  index.html
  vite.config.ts  tsconfig.json  tsconfig.app.json  tsconfig.node.json
  components.json      shadcn 配置（CLI/MCP 以进程 cwd 定位）
  .mcp.json            Claude Code shadcn MCP
  public/logo.png      Web favicon + 侧边栏品牌图（由根 logo.png 生成）
  scripts/gen-icons.py 从根 logo.png 重建全套图标，唯一来源见「品牌图标」
  src/
    main.tsx  App.tsx  router.tsx  index.css
    lib/      utils.ts(cn) / api.ts(API baseURL) / platform.ts(能力检测) / nav.ts(导航单一数据源) / clipboard.ts(copyText 三级降级)
    components/AppShell.tsx        浅色侧边栏 + 内容区
    components/ui/                 shadcn CLI 真实生成：button/card/badge/separator
    pages/                         8 个占位页 + 共用 PlaceholderPage
  src-tauri/
    Cargo.toml  build.rs  tauri.conf.json  capabilities/default.json
    src/main.rs  src/lib.rs  src/commands/mod.rs
    icons/                        由根 logo.png 生成的 AppIcon（PNG/ICO/ICNS）
    Info.plist  Entitlements.plist 硬件权限预留模板（当前未被 conf 引用）
```

## API baseURL 与 dev proxy

- `src/lib/api.ts` 是唯一 API 入口：默认同源相对 `/api`，可用 `VITE_API_BASE_URL` 覆盖；`apiUrl()` / `requestApi()` / `getWebSocketUrl()` 都从这里走。生产由 Rust `ServeDir` 同源托管（`dev-server.sh` 的 `XIAOZHI_ADMIN_DIR` 可指向本目录 `dist/`），Tauri 生产默认「加载服务器页面」同理，**均无 CORS 问题**。三种模式的 baseURL 语义写在 `api.ts` 文件头注释：dev = Vite proxy 同源、生产服务器页面 = 同源、`XIAOZHI_ADMIN_EMBEDDED=1` 内嵌 SPA 预览 = **API 不可达**；本轮不改请求实现，不改成插件 IPC。
- Vite dev proxy：`/api`、`/ws`（`ws: true`）、`/em` → `VITE_PROXY_TARGET`（默认 `http://127.0.0.1:8000`）。
- **`/em/` 排除口径**：proxy 中间件先于 SPA history fallback 生效，因此 `/em/`（含尾斜杠）会原样转发到 Rust server，不会被 `index.html` 吞掉。前端路由不得占用 `/em/` 前缀；后续官方页用 `<iframe src="/em/?...">`（带尾斜杠 + URL 参数）接入，**本阶段不实现**。微前端方向已否决。
- 路由使用 `HashRouter`：静态托管与 Tauri 内嵌 SPA 都没有服务端 history fallback，避免刷新／深链接 404。
- `vite.config.ts` 按 shadcn 官方 Vite 集成使用 `path.resolve(__dirname, "./src")` + `@types/node`；Vite 8 会提示 native configLoader 未来不支持 `__dirname`（建议 `import.meta.dirname`）。当前 `pnpm build` 通过，若后续升级到默认 native loader，改为 `import.meta.dirname` 并同步保持 alias 与 `tsconfig*.json` 的 `paths` 一致。

## Tauri 三种加载模式（生产默认加载服务器页面，方案 B，推荐）

> 背景：Rust server 是纯单二进制、同源托管静态页与 `/api`、`/ws`、`/em/`，**没有 CORS 层**。`src/lib/api.ts` 默认 `fetch('/api/...')` 是相对路径；若窗口从 `tauri://localhost` 加载内嵌 SPA，相对路径只会打到资产协议而到不了 Rust server，**API 根本不通**。因此生产默认改为加载服务器页面（方案 B），让 `/api`、`/ws`、`/em/` 天然同源。

| 模式 | 触发条件 | 窗口地址 | API / WS / `/em/` | 用途 |
| --- | --- | --- | --- | --- |
| **dev** | `cfg(dev)` 为真（`tauri dev` / `cargo check` / `cargo test`，即未启用 `custom-protocol` feature） | `WebviewUrl::App("index.html")` → `get_app_url()` 在 `cfg(dev)` 下映射到 `build.devUrl` = `http://127.0.0.1:5173` | **可达**：Vite dev server + proxy + HMR 不变 | 本地开发 |
| **生产默认：加载服务器页面（推荐，方案 B）** | 生产构建，且未设置 `XIAOZHI_ADMIN_EMBEDDED=1` | `WebviewUrl::External(server_url)` | **全可达**：同源，无 CORS | 日常运行 / 局域网 / 生产 |
| **内嵌 SPA 预览** | 仅 `XIAOZHI_ADMIN_EMBEDDED=1`（精确 `"1"`） | `WebviewUrl::App("index.html")` → `tauri://localhost` | **API 不可达**，仅预览 UI，**不作为默认运行方式** | 离线 UI 预览 |

- **服务器地址来源**：`XIAOZHI_ADMIN_URL` 优先（合法 http/https），未设置时用默认常量 `http://127.0.0.1:8000`。生产/局域网部署需**显式设置** `XIAOZHI_ADMIN_URL`（例如 `http://192.168.1.20:8000`），后续可接 mDNS 发现或地址配置页（本轮未实现，列为待办）。`parse_external_url()` 先做 URL 语法解析、再做 `http` / `https` scheme 白名单校验（对应 tauri-utils 对 `WebviewUrl::External` 「Must use either the http or https schemes」的要求，直接构造枚举会绕过 Deserialize 分类），`ftp://`、`file://`、`data:`、空串与非法串都返回明确 Err（错误信息区分「语法错误」与「scheme 不支持」）。
- **`cfg(dev)` 已查证可用**：`tauri-build-2.7.4/src/lib.rs:714` 的 `cfg_alias("dev", is_dev())` 经本工程 `build.rs` 的 `tauri_build::build()` 暴露给业务 crate，且 `cfg_alias`（同文件 `:304-308`）同时发出 `cargo:rustc-check-cfg=cfg(dev)`，故 `cfg!(dev)` 不会触发 `unexpected_cfg` 警告，`build.rs` 无需补 check-cfg。`tauri-2.12.5/src/manager/mod.rs:345-359` 的 `get_app_url()` 在 `#[cfg(dev)]` 下返回 `build.devUrl`，否则返回 `tauri://localhost`（`frontendDist` 是目录而非 URL）。`cargo check` 实测无 `unexpected_cfg` 警告。
- `resolve_window_url()` 只负责读环境变量 + `cfg(dev)`；分流逻辑抽成不读环境变量的纯函数 `decide_window_url(is_dev, remote_raw, embedded_raw)`，避免单测间环境变量竞态。`cargo test` 覆盖：dev 恒为 App、未设置环境变量返回默认服务器 External、合法 http/https 沿用、精确 `"1"` 的纯分支（`"true"`/`"0"`/`" 1 "` 不生效）、非 http(s) 仍返回 Err。
- `tauri.conf.json` 的 `build.frontendDist: "../dist"`、`build.devUrl: "http://127.0.0.1:5173"`、`app.windows: []`、`app.security.capabilities: ["default"]` 全部**保留不动**；窗口仍由 `src/lib.rs` 的 `setup` 用 `WebviewWindowBuilder::new(app, "main", url)` 创建（label 固定 `main`）。`vite.config.ts` 的 `server.host` 同为 `127.0.0.1`，两者必须一起改。
- **远程模式的已知限制（本阶段未解决，不写成已实现）**：`capabilities/default.json` **未配置 `remote.urls`**（二选一结论与依据见下方「插件」小节）。Tauri 官方原文「By default the API is only accessible to bundled code shipped with the Tauri App」，且 `tauri-2.12.5/src/webview/mod.rs:2076-2080` 的 `!is_local` ACL 检查、`src/ipc/authority.rs:57-66` 的 `Origin::matches` 共同拒绝远程页面 IPC，因此 `invoke("app_capabilities")` 与剪贴板插件 IPC 都会失败；但 `src/manager/webview.rs:168-183` 的 init script 仍无条件注入 `isTauri`（与 `withGlobalTauri: false` 无关），`testbench` 路由照常显示，`getCapabilities()` 在远程 origin 下 invoke 被 ACL 拒绝会**抛错**，由 TestbenchPage 捕获并显示「IPC 通道不可用（远程 origin 未授权 remote.urls）」提示，不再伪装成 `platform: "web"` 的「通道正常」。**剪贴板在该模式下走 `navigator.clipboard` + `execCommand` 降级**（见「插件」）。本阶段不为远程 IPC 加异步能力探测或额外 UI，要用远程模式必须显式配置 `remote.urls`。
- `app.withGlobalTauri: false`（不注入 `__TAURI__`）保持不变，**不给 `/em/` iframe 注入任何 IPC 能力**。
- capability：`capabilities/default.json` = `identifier: "default"`、`windows: ["main"]`（与窗口 label 一致）、`permissions: ["clipboard-manager:allow-write-text"]`（本轮唯一新增）。通过 `invoke_handler` 注册的应用自定义命令仅对 **Local origin**（内嵌 `tauri://localhost`）默认可用；生产默认加载的远程服务器页面与 `tauri dev` 的 Vite URL 都是 **Remote origin**，IPC 会被 ACL 拒绝，因此本阶段不依赖远程 IPC，也不需 `core:default`；本地配置页 / 内嵌模式下若要用 IPC 须显式配 `remote.urls`（模板见下节）。`tauri.conf.json` 用 `app.security.capabilities: ["default"]` 显式引用，避免目录里其它文件被自动启用。
- 远程 URL 的 capability 授权模板（**默认不开启**，JSON 不支持注释，故只写在本文件，不落盘成 .json）：

  ```json
  {
    "$schema": "../gen/schemas/desktop-schema.json",
    "identifier": "remote-example",
    "windows": ["main"],
    "remote": { "urls": ["https://admin.example.com"] },
    "permissions": ["clipboard-manager:allow-write-text"]
  }
  ```

  `capabilities/` 目录内所有 .json/.toml 会自动启用，所以模板不要放进该目录。启用远程 IPC 前务必评估 Tauri 官方 remote API 安全提示（Linux/Android 无法区分 iframe 与主窗口请求）。

## 能力检测与硬件命令扩展点

- `src/lib/platform.ts`：`isTauri()` 优先调用 `@tauri-apps/api/core` 官方 `isTauri()`（实现为 `!!globalThis.isTauri`，由 Tauri 2 init script 无条件注入），再以 `__TAURI_INTERNALS__` 兜底双保险；同步返回 boolean，**不做 UA 判断**，且不受 `withGlobalTauri: false` 影响。`getCapabilities()` 封装 `invoke("app_capabilities")`：浏览器环境（isTauri 为假）返回 Web 兜底值；Tauri Local origin（内嵌 SPA）返回真实值；**Tauri Remote origin（生产服务器页面 / `tauri dev`）下 invoke 被 ACL 拒绝，`getCapabilities()` 抛错**（由 TestbenchPage 的 catch 捕获并显式提示「IPC 通道不可用」），不再用 `platform: "web"` 制造「通道正常」的误导。调用方必须自行 catch，本函数不再静默降级。
- `router.tsx` 与 `AppShell.tsx` 共用 `src/lib/nav.ts` 的 `visibleNavDescriptors(isTauri())`；`testbench` 的 `tauriOnly: true` 与现有 `NavDescriptor.macOnly` 同构，Web 环境既不注册路由也不显示入口。
- `src-tauri/src/commands/mod.rs` 目前只有示例窄命令 `app_capabilities`，返回 `{ platform, tauri, version }`，与前端 `AppCapabilities` 字段一致。后续麦克风采集 / Opus 编解码 / 音频播放 / mDNS 发现在此加 `#[tauri::command]`，并在 `src/lib.rs` 的 `generate_handler!` 注册，再按需开启最小 capability 权限。
- `src-tauri/Info.plist`、`Entitlements.plist` 是**预留模板，当前未被 tauri.conf.json 引用**，含 `NSMicrophoneUsageDescription`、`NSLocalNetworkUsageDescription`、`NSBonjourServices` 等键；启用前需按实际签名／沙箱策略复核。
- **硬件测试边界**：接口返回 200 或页面存在不等于真机硬件通过，结论必须由真机测试给出；Web 不露出任何硬件动作。

## 插件（本轮只装 clipboard-manager）

- **本轮唯一新增依赖**：前端 `pnpm add @tauri-apps/plugin-clipboard-manager`（v2）；Rust 侧 `src-tauri/Cargo.toml` 加 `tauri-plugin-clipboard-manager = "2"`；`src-tauri/src/lib.rs` 在 `tauri::Builder::default()` 链上 `.plugin(tauri_plugin_clipboard_manager::init())`（位于 `invoke_handler` 之前，窗口 `setup` 逻辑未动）。其余插件一律不装。
- **动机**：现有 `AdminPlatform.kt:40-62` 记录管理台常以 `http://LAN-IP` 访问，非 secure context 下 `navigator.clipboard` 不存在，旧实现用隐藏 textarea + `execCommand('copy')` 降级；em 官方页还有 `Clipboard blocked by the browser`；macOS 原生用 `UIPasteboard`。装插件是为了让 Tauri 环境有一条原生复制通道，同时保留浏览器降级链。
- **capability 最小权限**：`capabilities/default.json` 只加 `clipboard-manager:allow-write-text`。不加 `clipboard-manager:allow-read-text`、不加 `core:default`、不加任何通配 plugin 权限；`identifier: "default"`、`windows: ["main"]` 不变，`tauri.conf.json` 的 `app.security.capabilities: ["default"]` 不变。
- **`src/lib/clipboard.ts` → `copyText(text: string): Promise<void>`**（签名与行为写在此处，调用方无需 try/catch）：
  1. `isTauri()` 为真时先试插件 `writeText`；远程服务器页面模式下 IPC 被 ACL 拒绝（或其它插件异常）由 `try/catch` 吞掉并继续降级，**绝不让复制失败抛到 UI**。
  2. 否则试 `navigator.clipboard.writeText`（仅安全上下文存在）。
  3. 再失败用隐藏 `textarea` + `document.execCommand("copy")`，`finally` 中 `textarea.remove()` 清理 DOM 并恢复原焦点。
  4. 三级全失败也只 `console.warn` 并正常 resolve，不产生未处理的 Promise rejection。

### 远程页面 IPC 边界：`remote.urls` 二选一结论 = **不配置**

- **查证依据 1（官方页面原文）**：Tauri v2 官方 Capabilities 文档「Remote API Access」章节原句：「By default the API is only accessible to bundled code shipped with the Tauri App. To allow remote sources access to certain Tauri Commands it is possible to define this in the capability configuration file.」示例为 `"remote": { "urls": ["https://*.tauri.app"] }`（仅支持子域通配），并明确警告：「On Linux and Android, Tauri is unable to distinguish between requests from an embedded `<iframe>` and the window itself.」「Please consider usage of this feature very carefully and read more into the specific security implications for your targeted operating system in the reference section of this feature.」
- **查证依据 2（本地 crate 源码）**：`tauri-utils-2.10.2/src/acl/capability.rs:237-249` 定义 `CapabilityRemote.urls` 并使用 [URLPattern 标准](https://urlpattern.spec.whatwg.org/)，文档示例为 `"https://*.mydomain.dev"` 与 `"https://mydomain.dev/api/*"`；`tauri-utils-2.10.2/src/acl/mod.rs:280-321` 的 `RemoteUrlPattern` 由 `urlpattern::UrlPatternInit::parse_constructor_string` 解析并 `test(url)` 匹配；`tauri-utils-2.10.2/src/acl/resolved.rs:283-291` 把每条 url 变成 `ExecutionContext::Remote`。
- **查证依据 3（用 Tauri 实际依赖的 `urlpattern 0.6.0` 离线实测，临时目录一次性验证、未写入仓库）**：`http://192.168.*.*:8000`、`http://192.168.*:8000`、`http://10.*.*.*:8000`、`http://192.168.(.*):8000`、`http://*.local:8000` 全部**不匹配任何局域网 IP**（URLPattern 的 hostname 只支持 `*.` 前缀子域通配，IP 段通配无效）。要覆盖局域网 IP 只能用 `http://*:8000` / `http://*:*` / `http://*.*.*.*:8000` / `http://*`，实测它们同时匹配 `http://evil.example.com:8000/`、`http://sub.evil.example.com:8000/`、`http://a.b.c.d.evil.com:8000/`，即**等价于任意 HTTP 源**的过宽规则；精确写死 `http://127.0.0.1:8000` 与 `http://localhost:8000` 倒是安全，但覆盖不了动态的局域网 IP 与任意 `XIAOZHI_ADMIN_URL`，等于放弃服务器页面模式的主要收益。
- **结论**：官方语法**无法安全限定到「本机 + 局域网服务器地址」**而不退化成任意 HTTP 源，因此**不配 `remote.urls`**，把插件限定为内嵌 SPA / 未来本地配置页可用。信任边界不扩大到「你连的那台服务器可以写你的剪贴板」，更不扩大到「任意 HTTP 源可以写你的剪贴板」。
- **明确写出口径（不是已实现）**：**生产默认服务器页面模式下插件 IPC 默认被 ACL 拒绝**，剪贴板走 `navigator.clipboard` + `execCommand` 降级；`withGlobalTauri: false` 保持关闭，不给 iframe 注入任何 IPC 能力。若将来确需在服务器页面启用插件 IPC，必须显式配置 `remote.urls`（模板见上节）并接受上述信任边界。

### 明确「不装」的插件类别与依据

| 类别 | 插件 | 不装依据 |
| --- | --- | --- |
| OTA | `updater` | 项目 OTA 是 ESP32 设备固件，不是桌面 App 自更新 |
| 存储 | `fs` / `store` / `sql` / `stronghold` | 配置在服务端 SQLite，文件走 `/api/files/*`，桌面端无本地状态需求 |
| 通知/日志/网络/系统 | `notification` / `log` / `websocket` / `os` / `process` | SSE + 服务端 tracing + 同源 `/ws` + 自研 `app_capabilities` 已覆盖 |
| 窗口/生命周期 | `autostart` / `single-instance` / `tray` / `global-shortcut` / `deep-link` / `window-state` | docs 全目录零命中，无需求 |
| 硬件 | 麦克风 / Opus / 播放 / mDNS 等社区插件 | 走自定义 `#[tauri::command]` 窄接口（见「能力检测与硬件命令扩展点」） |
| 社区集成 | 各类第三方集成插件 | 当前 React + fetch 栈无需求 |

- 本轮也**不装** `http-client` / `dialog` / `opener` / `fs`：`src/lib/api.ts` 保持原生 `fetch` 相对路径，不改成插件请求。

## shadcn MCP 配置与 cwd 限制（方案 b）

- 配置清单：本目录 `.mcp.json`（Claude Code，`mcpServers`）；仓库根 `.mcp.json`、`.cursor/mcp.json`（Cursor，`mcpServers`）；仓库根 `.vscode/mcp.json`（VS Code 使用 **`servers`** 键而非 `mcpServers`）；`~/.workbuddy/mcp.json`（合并 `shadcn`，`figma-console` 及其 token 原样保留）。Codex 未配置（官方是 `~/.codex/config.toml` 的 `[mcp_servers.shadcn]`）。
- 逐字采用官方 command/args：`"command": "npx"` + `"args": ["shadcn@latest", "mcp"]`；**不臆造** `cwd` / `env` 等未文档字段。
- **选择 (b)：仓库根不放 `components.json`。** 实测（2026-11-10）：
  - 在 `client/admin-web` cwd 执行 `CI=true pnpm dlx shadcn@latest add button card badge separator`，4 个组件真实落到 `src/components/ui/`。
  - 在项目 cwd 执行 `CI=true pnpm dlx shadcn@latest info` 作为对照：`framework Vite`、`tailwindVersion v4`、`importAlias @`、`Installed Components: badge, button, card, separator`，`Resolved Paths.cwd` 正确指向本目录。
  - 在仓库根 cwd 执行 `CI=true pnpm dlx shadcn@latest info`，输出 `Configuration: No components.json found.`、`framework Manual`、`importAlias -`，即根 cwd 既找不到配置也无法解析 `@/*` 别名。
  - 理由：shadcn CLI/MCP 以**进程 cwd** 定位 `components.json`，并依赖 cwd 下 `tsconfig.json` 的 `paths` 解析 `@/*`；仓库根是 Rust server + Gradle/KMP，没有 tsconfig.json。方案 (a) 要在根伪造 components.json 与 tsconfig/paths，组件可能落到仓库根 `src/`，污染 Rust 仓库并与现有 KMP 工程混淆，收益低于风险。
  - **限制**：从仓库根打开 IDE 时，三份根 MCP 配置能让 MCP **握手与 tools/list 正常**，但依赖 `components.json` 的工具会静默降级为空结果或失败（QA 复验实测 `get_project_registries` 静默返回空列表，并非明确报错）。需要把 IDE 工作区／终端 cwd 设到 `client/admin-web`（或用多根工作区把该目录作为根）后再使用 shadcn MCP。
- 官方故障排查：`npx clear-npx-cache`、确认 cwd 下 `components.json` 存在、检查网络。
- WorkBuddy：**不要运行该 MCP server**；需在连接器管理页右上角自定义连接器入口点击「信任」后才会生效。
- shadcn 4.23.0 生成的组件使用 `import { cn } from "cn"` 与 `import { Slot } from "radix-ui"`（新版官方 registry 输出），CLI 已自动补齐 `cn` / `radix-ui` 依赖；`src/lib/utils.ts` 仍保留标准 `cn()`（clsx + tailwind-merge）供业务代码使用。**不要手改 `src/components/ui/*`**，新增组件统一用 `pnpm dlx shadcn@latest add ...`。

## 品牌图标（唯一来源）

- **唯一视觉来源**：仓库根 `logo.png`（1024×1024，最终稿 · AI 聊天机器人 AppIcon，蓝紫渐变 + 白色螺旋对话气泡；设计稿记录见根 `xiaozhi-logo.miora`，同名副本在 `xiaozhi-logo_assets/f1784d34-icon_512x512@2x.png`）。**禁止再用自绘或纯色占位图标**。
- 更新 logo 后重跑：`python3 -S client/admin-web/scripts/gen-icons.py`（在仓库任意目录执行）。依赖 macOS 自带 `sips` / `iconutil` 与 Python 标准库，不引入第三方包；中间尺寸缓存在 `node_modules/.cache/`，已被忽略。
- ⚠️ **必须是 RGBA**：Tauri `generate_context!` 会校验 `bundle.icon` 的 PNG，非 RGBA 直接编译失败并报 `icon .../32x32.png is not RGBA`。根 `logo.png` 无 alpha，`sips` 缩放后仍是 8-bit RGB；脚本用标准库解 filter 后按 `filter=0` 重编码为 RGBA（alpha 全 255），再输出所有 PNG / ICO 内嵌图 / ICNS iconset。**不要删掉 `to_rgba_png()` 这一步**，否则 `cargo check` 会立刻挂。
- 产物与使用位置：
  - `src-tauri/icons/{32x32.png,128x128.png,128x128@2x.png,icon.png,icon.ico,icon.icns}` → `tauri.conf.json` 的 `bundle.icon`，供 macOS / Windows / Linux 安装包与窗口图标使用。ICNS 走 `iconutil` 的标准 iconset；ICO 用内嵌 PNG 的 Vista+ 格式（手写容器，避免额外依赖）。`128x128@2x.png` 按模板命名，实际为 256×256。
  - `public/logo.png`（512×512）→ Vite 原样复制到 `dist/`，供 `index.html` 的 favicon 与 `AppShell.tsx` 侧边栏品牌位使用；Rust server 同源托管与 Tauri 内嵌 SPA 都取 `/logo.png`。
- Web 与桌面端共用同一份图，不维护第二套视觉；图标资源不写进 Rust server，也不参与业务逻辑。

## 主题

`src/index.css` 首行 `@import "tailwindcss";`，其后是 shadcn 默认浅色 CSS variables（neutral）。不引入暗色主题切换，与现有 GearUI 浅色口径一致。

## 页面迁移（已启动）

- **Overview 已迁移**（数据源端点均为后端无条件存在、不依赖 cargo feature）：`/api/health`、`/api/stats`、`/api/health/engines`、`/api/devices`、`/api/server/geo`、`/api/plugins`；页面用 `useApiQuery` 并发拉取，统计卡网格 + 能力健康 / 在线设备 / 服务器出口 / 插件能力四张卡，含加载态（「加载中…」）与错误态（顶部红条 + 重试）。
- **Devices 已迁移**（数据源 `GET /api/devices` 列表 + `GET /api/devices/{id}` 详情 + `POST /api/devices/{id}/inject` 消息注入）：列表每行显示 device_id(mono) / 在线 Badge(绿·离线灰) / 测试·MCP Badge / board·firmware·client_id / 相对时间；行可展开调 `getDevice(id)` 显示会话与对话摘要；展开区内原生 `<input>` + 注入按钮调 `injectDevice(id, text)`，成功/失败提示行内显示、不抛顶层，成功后刷新详情。`adminApi.ts` 据此新增 `getDevice` / `injectDevice` 与 `DeviceDetailInfo` / `DeviceSession` / `DeviceConversation` / `InjectResult` 接口。
- 新增 **`src/lib/adminApi.ts`**：类型化只读 API 客户端，封装上述 6 个端点的 `getXxx()` 与响应 TS 接口，供后续页面复用；所有请求经 `requestApi`（自动拼 `/api` 前缀）发出。
- 新增 **`src/lib/useApiQuery.ts`**：轻量查询 hook（仅用 React 内置 hooks，无新依赖），返回 `{ data, error, loading, updatedAt, refetch }`，作为后续页面的数据获取样板。
- **Config C0 已迁移**（壳 + 能力总览 + 预设）：分区清单唯一来源 `src/components/config/configSections.ts`，18 项与本仓 `client/shared/.../ConfigSections.kt:28-50` 的 id/label/分组/order **逐项一致**（新增分区必须两边同步）；左菜单按「总览 / 语音主流程 / 上下文增强 / 服务与接入」分组，未迁移的 7 个分区点开是占位卡、不会改 draft。`ConfigDraftProvider`/`AdvancedModeProvider` 只在本页挂载（不挂 `App.tsx`，避免其它页面无端拉配置）。保存后按响应 `hot` 在 `SaveBar` 显示三档文案；**`ready={revision !== null}`**，配置没加载成功时文案是「配置未加载，无法保存」而不是「没有未保存的修改」。页面同时渲染 provider 的 `warning`（「已保存但回读失败」）与 `error`——**错误条与冲突条职责分离，不要再加 `!conflict` 抑制**，否则冲突之后的 reload 失败 / 400 / 500 全部对用户不可见。总览段的 phase 中文标签来自 `PHASE_LABEL`（wire 值是 `active`/`degraded`/`failed`/`disabled`），hot 文案复用 `SaveBar` 导出的 `HOT_LABEL` 单一来源。
- **Config 已迁移 18/18 段（全部完成，C0–C7）**（唯一依据：`configSections.ts` 的 `migrated: true` 计数）：C0 `overview` / `presets`；C1 `server` / `aiui` / `audio`（页尾拼 `PlaybackSection`）；C2 `vad` / `asr` / `command` / `wakeup`；C3 `llm` / `tts`；C4 `nlu`；C5 `soul` / `memory`；C6 `echomuse` / `tuya` / `mcp`；C7 `ota`。字段默认值与 advanced 归属以 `plugins/*/mod.rs`、`plugins/registry/projection.rs` 为准。Config 侧剩余工作只有运行时验证（浏览器 / 真 server）与后续增量字段。
- **C1/C2/C3/C4 契约摘要（详细见「配置域地基」与下方各 C3/C4 小节）**：一律 `diffPatch()` 最小叶子 patch（服务端 `merge_patch` 递归增量合并，**无整段替换分支**，旧 Kotlin 整段回传已废弃）；密钥走 `secretValues` + `withSecrets()` 深度展开，`server.expected_token` 是例外；`NumberField` 默认整数、f32 字段用 `integer={false}`；`command.match_mode` / `max_chars` 与 `llm.engine` / `llm.sentence_flush_ms` / `llm.fallback.providers` 禁止暴露（fallback 是对象数组，`writePath()` 不处理数组）；`playback.mode` 虽在 advanced schema 但 UI 常显，二次确认文案不得写「未在界面显示」。
- **WS URL 口径已修正**：`src/lib/api.ts` 的 `getWebSocketUrl()` 相对分支由 `/ws` 改为 `/api/admin/ws`（管理台实时通道；Rust server 无裸 `/ws` 前缀）。dev proxy 的 `/api` 规则已开 `ws: true`，`/api/logs` 与 `/api/admin/ws` 都能在 dev 下完成 WebSocket 升级。
- **Console 已迁移**（数据源 `WS /api/logs`，注意**不是** `getWebSocketUrl()` 指向的 `/api/admin/ws`）：`buildLogsWsUrl()` 单独推导 `/api/logs` 地址；2000 行环形上限 + 200ms 批量 flush（`bufferRef` 攒行、`setInterval` 合并），避免高频日志逐条 setState；按 `ERROR|WARN|INFO|DEBUG|TRACE` 分词着色与等级筛选、暂停/恢复自动滚动、复制全部、清空；断线走 **`onclose` 自动重连 + 指数退避**（1s 起、30s 封顶，`onopen` 归位），`retryRef` 定时器在 effect cleanup 里必清，`generation` 只由「重连」按钮推进，重连前先 close 旧连接所以不会多连接并存。`onclose` 里不能把 `error` 状态无条件覆盖成 `closed`，否则握手失败与正常断开分不清。
- **Files 已迁移**（数据源 `GET /api/files`、`/api/files/mkdir`、`/api/files/upload`、`/api/files/delete`、`/api/files/download`）：面包屑（根目录 + 逐段可点）+ 上级 + 刷新；**排序完全交给服务端**（`op_list` 已按「目录在前、字节序名称升序」排，客户端再按 locale 重排会与 Kuikly 版顺序不一致）；`protected`（`data.db*`）显示琥珀色「受保护」Badge 并禁用删除；删除走 `window.confirm` 二次确认（目录明确提示「及其全部内容」，服务端是递归删除）；上传为裸字节 + `Content-Type: application/octet-stream`，用 XHR 拿 `upload.onprogress` 显示百分比，浏览器侧先按 64MB 拦截，命中 409 时二次确认后带 `overwrite=1` **重试一次**。错误按 HTTP 状态翻译：403 受保护路径、409 已存在、413 超限、404 已消失，其余优先透出服务端 400 的 `{"error":"..."}` 原文。列表请求失败必须显式报错 + 重试，**不能落到「该目录为空」**。
- **Config C3 已迁移**（LLM / TTS 两段 + 全局 token 注入，W6）：新增 `src/lib/catalogueApi.ts`（四端点封装）与 `LlmSection.tsx`（8 字段；`llm.engine` / `sentence_flush_ms` / `fallback.providers` 不暴露）/ `TtsSection.tsx`（公共 5 + 按 `tts.engine` 条件渲染 kokoro 8 全 advanced / xfyun 4；`wasm-policy` 无私有段）。**token 注入**：`api.ts` 导出 `getAuthToken()`（`?token=` → sessionStorage `admin.authToken` → 回读），`requestApi` 非空时自动 `Authorization: Bearer`，token **不拼进 URL**；HashRouter 下 token 必须放在 hash 之前（`http://host/?token=xxx#/config`），写在 hash 后 `window.location.search` 读不到。**四端点一律判业务字段、不看 HTTP 200**：`POST test/llm/models` 判 `state==="ok"`（200 也可能 `state:"error"`；body 只能含非空 `api_base`/`api_key`，`deny_unknown_fields`）；`GET tts/voices` 必须校验「JSON 对象 + `voices` 数组 + `probing` 布尔」——`native-plugins` feature 未编入时会被 SPA fallback 接走返回 **HTML 200**，形状不符按「能力未启用」用 `UnavailableNotice`，禁止伪装空目录；音色 `refresh` 是异步后台任务（立即 200 `probing:true`），前端 2s 间隔、90 次上限轮询到 `probing===false` 再重拉；`test-credentials` 只测不存、200 也可能 `{ok:false,error}`。三个交互 401 统一明确报「需要管理鉴权」。**token 遗留口径**：Files XHR 上传与 `downloadUrl` 下载在 `expected_token` 非空时会 401（W0a 收尾项）；`/api/logs` 的 `logs_ws` **完全不鉴权**、不要拼 token；`getWebSocketUrl()` 当前零调用点、接实时通道时再处理。
- **Config C4 已迁移**（NLU 段 + 技能/槽位编辑器，W7）：新增 `src/lib/nluApi.ts`（五端点封装，统一错误归一）、`src/components/form/JsonTextField.tsx`（planJson 文本域 + 守卫）、`NluSection.tsx` + `NluIntentsEditor.tsx` + `NluSlotsEditor.tsx`。**整段写回**：`nlu.intents` / `nlu.slots` 是多层嵌套数组，`writePath()` 不处理数组，所以**禁止下标路径**（`nlu.intents.0.x` 会把数组中间段替换成 `{}`）——编辑器用本地数组副本 + `setField("nlu.intents", 整段数组)` 写回（全仓仅 2 处、0 下标路径）；副本同步 = draft 引用变化 + JSON 深比较不一致才重置（覆盖 409 重载 / 回读 / 丢弃三场景，自身 commit 不打断编辑中间态）。**planJson 守卫是客户端形态守卫，不是服务端校验**：≤32768 字节（UTF-8）→ 合法 JSON 对象 → conditions ≤16 → steps 1~16 → 每 step 为对象（与旧端 `ConfigNluPlanJson.kt:20-33` 文案逐字一致）；`/api/workflow/validate` 校验的是 `[workflow]` 段与 intent plan 无关、前端 0 调用；PUT 主链无 `validate_nlu`，唯一门禁是 serde 反序列化。**六端点一律判业务形状**：`GET/POST /api/nlu/model/build`（`BuildRequest` deny_unknown_fields，body 只 `{expected_revision}`；判 `started===true` + revision 回显一致）、`GET /api/nlu/model/builds`（历史+候选；`base_model` null → 「未记录（历史构建）」）、`POST /api/nlu/model/test`（60s 超时；强校验 `measurement==="real_onnx_inference" && executed===false`）、`GET /api/skills/*`（pure-base 构建下被 SPA fallback 接走返回 HTML 200 → `NluUnavailableError` → `UnavailableNotice`，禁止伪装空目录）；不接 `build/log` WS（一次性流、终态后服务端主动 Close 不可重连），构建日志用 HTTP 轮询 5s × 360（30 分钟上限，对齐旧端）。**候选不自动启用**：服务端 `auto_activated` 硬编码 false；「启用候选」= 把 `artifact.expected_config` 的 `bert_enabled/bert_model/bert_vocab/bert_labels` 四字段写进草稿再走既有 `PUT /api/config`。**不暴露**：`model_threshold` / `model_margin` / `bert_label_map` 0 处渲染；`base_model` 仅候选卡/历史表只读展示（纯展示常量，不参与请求）。**advanced 边界**：bert 5 字段（含 `bert_num_threads`）与 `projection.rs:98-102` 全等，`bert_max_len` 不标 advanced；`nlu.intents` / `nlu.slots` 在 `ADVANCED_EXTRA_KEYS`（`projection.rs:110`），简洁模式二次确认由 ConfigPage 自动覆盖；`bert_threshold` 是唯一小数通道 `integer={false}`。**设备工具名 TextField 仅 `kind==="mcp"` 时显示**（对齐旧端 `ConfigNluCard.kt:160`，end_session 语义由 kind 承载、无独立字段）。
- **Config C5 已迁移**（Soul / Memory 两段，W8）：新增 `src/lib/soulMemoryApi.ts`（7 端点封装）与 `SoulSection.tsx` / `MemorySection.tsx`。**与本段相反的三契约**（头注释固化，勿套用 W6/W7 口径）：7 端点全部 `native-plugins` 门控（pure-base 下 SPA fallback 返回 **HTML 200** → `SoulMemoryUnavailableError` → `UnavailableNotice`）；全部 handler **无鉴权**（不调 `auth_ok`，前端**不得出现 401/token 分支**，requestApi 自带的 Bearer 头无害）；请求体全裸 `Value` **无 `deny_unknown_fields`**。全部判 `ok===true` + 业务形状，HTTP 200 不算成功；错误归一读响应体 `message`（400 也是 JSON `{ok:false,message}`）。**控件陷阱**：soul 5 个列表字段（traits/values/boundaries/catchphrases/examples）是 `Vec<String>` 但 schema 误声明 `FieldKind::Text`——**必须 StringListField 写 `string[]`，禁 TextAreaField**（否则保存触发「列表字段必须是数组」400）；`memory.extractor.temperature` 是两段唯一 f32 → `integer={false}`；`api_key` 走 SecretField（`SECRET_PATHS` 已含，presence = `has_api_key`）。**写回**：一律最小叶子 patch，禁旧端 `ConfigContextJson.putJson()` 整段写回；字符串数组可叶子级 `setField("soul.traits", string[])`（不同于 W7 对象数组）。「载入预设内容到表单」= POST preview 回包 `effective` **逐键 setField**（SoulConfig 22 字段全 `serde(default)` 无 `skip_serializing_if` → effective 恒全字段序列化，与旧端整段 fill 等价且缺键不冲空）；`preset==="none"` 拦截。**Memory 字段全量暴露**（含 `db_busy_timeout_ms` 旧端缺口；`recall_enabled` 按 schema 归高级，旧端放简单模式不照抄）；rag 嵌套段仅 `engine==="rag"` 渲染。**动作诚实化**：maintain 单端点 + `dry_run`（演练文案「未删除任何数据」，**没有 apply 端点**；`memory.retention.dry_run` 是持久化配置项、与请求参数 dry_run 两回事）；clear **两段式纯客户端**（第一段零网络、无服务端凭据；改 scope / 切「清空全部设备」必须复位确认态；服务端强制 `{scope}` 或 `{all:true}` 显式，两者皆无 400）；rag/reindex **异步无进度端点**（只透出服务端 message，防重入时 `ok:true` 但 message 是「被挡下」——**不得报成功**；进度靠手动刷新 status 的 `rag.building`，无自动轮询）；recall 的 wire 是 `hits`/`matched_terms` **数字**、`injected` **布尔**、正文在 `preview`（D1 教训：**端点实现前逐行核读 `to_json`，不依赖二手摘要**）。状态卡诚实呈现 `lexical_only`（无 embedding：本地词法召回）/ `capability.last_error`（降级）/ `aiui_note` / `rag.building`。busy 门禁是 admin-web 统一增强（旧端无锁），文件头已声明「新增行为」；测试召回/清空的输入是本地 state **不进配置草稿**。
- **Config C6 已迁移**（EchoMuse / Tuya / MCP 三段，W9）：新增 `src/lib/mcpApi.ts` 与 `EchomuseSection.tsx` / `TuyaSection.tsx` / `McpSection.tsx`（只读表格）。**C6 形态是「2 可写表单 + 1 只读表格」**——`mcp` **不是配置段**（`Config` struct 无 `mcp` 字段），MCP 卡片 0 处写入（不接 ConfigDraftContext / 无 setField / 不挂 SaveBar）。**三条结构性证伪**（勿套用 W6/W7/W8 口径）：①「registered 门控」是 WASM 插件生命周期三态（staged→registered→active）与 `/api/plugins` 投影的语义，与 `/api/mcp/tools`、配置段显示均无关，C6 不得出现；②`/api/mcp/tools` 是**无条件基础路由**（`ws/mod.rs:225`，无 cfg 无鉴权），永远返回 JSON，**不存在 SPA fallback HTML 200**，判 `ok===true` + `devices` 数组即可，**空 devices 是正常态**（设备空闲不建连）显示服务端 `note` 原文；③echomuse/tuya 的 fail-closed 是**字段级**：两段是 `#[cfg(feature)]` 门控的 Config 字段，pure-base 下 GET /api/config 回包**无这两个 key**、schema capabilities 也不含——按「能力未编入当前构建」UnavailableNotice，不渲染空表单。**字段面**：EchoMuse 暴露 6/7（enabled/token/welcome/engine/ble_proxy/plain_port；`music_dir` 是 `FieldKind::Dir` 无控件，文件头登记补全清单）；Tuya 暴露 2/6（enabled/sync_interval_secs；**region/access_id/access_secret 一律不渲染**——凭证唯一录入路径是 devicehub 的 `POST /api/tuya/credentials`（不经 PUT 主链），两套 UI 不互相覆盖）；MCP 只读（devices/tools 投影，`inputSchema` 透传不解析）。**反先例**：三段**零 advanced**（projection 三表均无）、**零数组**、**零 f32**——无 StringListField / `integer={false}` / 整段写回；热档 echomuse=**RestartOnly**、tuya=**Live**（tuya 是唯一 live 的应用级段，`APP_SECTION_HOT` mod.rs:304-309）——档位由 schema `app_sections.hot_hint` 经 SectionHotHint 下发，0 处手写。**鉴权**：C6 本轮两个数据源（PUT /api/config、GET /api/mcp/tools）均无鉴权，0 处 401/token 分支；`/api/echomuse/*`、`/api/tuya/*`（全部经 auth_ok）、`/em/*` 是 W12 devicehub 运行面，本轮 0 调用、无「管理设备」跳转按钮、EchomusePage/nav 未动。**MCP 诚实化**：「已断开」Badge 必须带 `age_ms` 相对时间（ageText 用服务端相对毫秒，不把上次探测伪装成现在）；error 非空显示「最近一次探测未完成…（下面显示的是上次成功探测的结果）」；`user_only` →「仅管理员」徽标（= 为什么 AI 不会用这个工具的答案）；description 显示原文（= 喂给大模型的提示词）；手动刷新无自动轮询、错误行内不弹 toast。`sync_interval_secs <60` 与 region 非法是 normalize **静默收敛**（不是 400），文案说「服务端会自动收敛」。
- **Config C7 已迁移**（OTA 段，W10，**18 段迁移收官**）：新增 `src/lib/otaApi.ts` 与 `OtaSection.tsx`。**形态**：ota 是 18 段唯一「**0 配置字段纯动作面板**」——`Config` struct 无 ota 字段、projection 三表零 ota、无密钥、`APP_SECTION_HOT` 无 ota；**0 处 setField / 接 draft / SaveBar / dirtyCount / revision**，上传/删除走独立端点不经 `PUT /api/config`（解耦前言卡逐字保留旧端「保存配置不会动固件，上传/删除固件也不会动配置」）。**混合鉴权分区（C7 独有，勿一刀切）**：清单 `GET /api/ota/firmware` **无鉴权**（handler 不取 HeaderMap，**响应无 `ok` 键**——判形状 `{dir,count,entries[]}` + FirmwareMeta 十字段，**勿误判 `ok===true`**）；上传 POST 与删除 POST `/api/ota/firmware/delete?project=&model=&board=` **调 auth_ok**，**401 响应是纯文本 `"unauthorized"` 非 JSON**——错误按 `status===401` 区分。**fail-closed 双路径教训（D1，后续新端点封装的检查单）**：端点封装必须同时覆盖「**解析失败**」（`requestApi` 对 HTML 200 抛裸 SyntaxError——pure-base 下整组 `native-plugins` 门控被 SPA fallback 接走）与「**形状不符**」两条路径，各归一 `OtaUnavailableError`（对齐 `soulMemoryApi.ts:90-92` 模板），禁止把 HTTP 200 当可用、禁止透抛英文异常原文。**上传**：裸字节 `application/octet-stream`（非 multipart 非 JSON）、坐标 query 覆盖（留空键不进 query；4 个手动覆盖字段是 query 参数的**本地 state**，不是配置字段）、XHR `upload.onprogress` **真实百分比**（旧端「不确定态进度」是 fetch 无 onprogress 的 UI 妥协，非服务端能力——研究已证伪）、64MB 上限 413、`metadata_from` 提示**留余地**（服务端只按包内是否解析成功判定，手填覆盖时仍可能报 "package"）；上传失败**常驻 danger Alert**（不压成 3 秒 toast）+ 401 专项指引。**删除**：query 三键定位（0 处 {id} 路径参数）、destructive 弹层**点遮罩不关闭**（必须明确点取消/删除）、文案含「此操作不可撤销」「已升级到该版本的设备不受影响」。**边界登记**：固件默认 `/data/firmware` 在 W2 Files root（`/data`）内且不受 `PROTECTED_NAMES` 保护——UI 不得暗示「固件受 Files 保护」，风险登记不修复；`/api/ota/firmware/latest` 与裸 `/api/ota` 是设备侧契约，管理台 0 调用。
- **api 封装检查单（强制前置自查，QA/工程师 W10 联合沉淀；W6/W8/W10 三轮回归验证的稳定判据）**：新增 api 封装文件（`catalogueApi` / `nluApi` / `soulMemoryApi` / `mcpApi` / `otaApi` 已成模板）时自查两条——① **门控端点（feature-gate、SPA fallback 会接走）的请求 catch 链必须同时覆盖两条失败路径**：`SyntaxError → UnavailableError` 归一（`requestApi` 对 HTML 200 抛裸 SyntaxError，`api.ts:141`）与「JSON 解析成功但形状不符 → UnavailableError」，**缺一不可**（模板 `soulMemoryApi.ts:82-114`；W10 D1 正是缺 ① 的 major 实例）；② **混合鉴权先逐 handler 列清单**（是否调 `auth_ok`、401 是 JSON 还是纯文本），再决定各方法写不写 401 分支——无鉴权方法须在头注释**显式声明**「不写 401/token 分支」，有鉴权方法按 W0a 口径 Bearer 注入、401 响应体形状如实透出。
- **Plugins 页（已迁移，W11，nav 第 9 项）**：新增 `lib/pluginsApi.ts` / `lib/pluginUi.ts`（A2UI v0.1 解析器纯函数）/ `components/plugins/PluginUiRenderer.tsx` / `pages/PluginsPage.tsx`（含 `PluginRuntimeRoute`）；改 nav/router/AppShell。**fail-closed 双路径形态与 OTA 相反**：`GET /api/plugins` 是无条件基础路由**永远 JSON**（判 `ok===true` + 形状；`wasm` 键缺失/损坏 → **null 静态基线 fail-open**，UI 层 UnavailableNotice「能力未启用」+ 隐藏全部写操作——**空清单 ≠ 未启用**）；写端点（upload/raw、生命周期 4 个、actions）受 `wasm-plugins` 门控，pure-base 下 HTML 200 → **SyntaxError 必须归一 `PluginsUnavailableError`**（normalizeWriteError，模板同构）；503 纯文本「初始化失败已降级」单独文案。**鉴权与 OTA 相反**：11 个 wasm handler **全部不调 `auth_ok`**（注释声称的全局 layer 实为 `dev::guard` 非鉴权层；真实管理鉴权边界在 WS RPC 握手 `admin_ws.rs:400`）——0 处 401 分支、token 不拼 URL（旧端 `?token=` 服务端不读）。**「registered」双语境**：WASM 生命周期三态之一（`staged→registered→active`，本页真语境）≠ Kotlin `device_plugins[].registered` 历史门控键（**服务端源码零匹配不下发，不实现不消费**）。**生命周期按钮矩阵**：staged→注册+移除；registered→启用（`canActivate` = registered && requires 全 satisfied——**satisfied 服务端求值下发，客户端 0 重算**）+移除；active→停用（`canDeactivate` = 无 Active 依赖者）；remove 二次确认 destructive、deactivate 非 destructive、均点遮罩不关闭；409 的依赖缺口中文原文透出。**上传**：XHR octet-stream + onprogress 真实百分比；`?id=` 不传（服务端 probe_id 从 manifest 探测）；**体积不写死 8MB**（应用层 8MB 但 wasm 路由未挂 `DefaultBodyLimit`，axum 默认 2MB 可能先触发——以服务端 413 原文为准）；成功文案逐字「已上传，待注册：{id}」——**上传 ≠ 启用**。**A2UI**：数据源**内联消费 `registration.ui`**（同一次拉取，`/{id}/ui` 端点旧端零调用、不实现）；七条 fail-closed——版本非显式 `"0.1"` 整份作废、引用白名单逐条丢弃且元数据只取注册声明（danger 洗白防护）、input.id 正则+页内唯一、未知 type 丢节点不丢文档、深度≤4/页≤8/叶≤64、text≤500 超出丢弃不截断、全无效→null→声明式回退页；danger 动作二次确认；`field`/`form` 只展示元数据 0 可编辑输入框（secret 不渲染密码框）；input 页内临时态不进 draft；运行页 `plugin/:pluginId/:entryId` Route 在 `path="*"` 之前、非 Active 回退 /plugins（对齐 AdminShell.kt:429-433）。**动态侧栏入口**：`usePluginNavEntries` 消费 `registration.menu[]` 的 `slot=="nav.main"`（仅 Active 有 registration）——**静态基线 fail-open**（wasm 缺失不追加动态项、无空「插件」组）；不做 `unload`（当前服务端恒下发 lifecycle；null 兼容分支是旧服务端的事，文件头登记）、不做 `extensions/{point}/preference`（旧端 UI 无消费）。**code-splitting W11 结论**：**仍不做**（QA 三次结论条件「页面全迁完 + 有浏览器验收」后者仍不满足；本机无浏览器无法验收分包运行时加载，Tauri 远程加载分包反增 RTT）；A2UI 解析器/渲染器已按独立模块组织（结构上可 lazy），待验收环境就绪时切分。

## 配置域地基（已接线，C0–C3 已落地）

配置页是 18 个**手写**配置段（不是 schema 驱动），因此先落地状态层与原语，再逐段迁移页面。

- **`src/lib/configApi.ts`**：`getConfig` / `putConfig` / `getConfigSchema` / `getConfigMeta` / `listPresets` / `savePreset` / `applyPreset` / `deletePreset`。wire 契约以服务端为准，**不要按直觉改**：
  - `PUT /api/config` 成功体是**扁平**字段 `{ok,message,config_path,revision,hot,hot_hint,changed_sections}`，没有嵌套 `impact`，也**不返回** `paths`（`paths` 只写服务端日志）。
  - 409 体是 `{ok:false,message,config_path:null,revision:<当前指纹>}`，`putConfig` 抛 `ConfigConflictError`（带 `currentRevision`），**不要吞掉**。
  - 400 有两种：合并/解析失败是**纯文本**，workflow 预编译失败是 **JSON** `{ok:false,message}`。
  - 预设列表 wire 是 `{presets: string[]}`；删除统一走 `POST /api/config/presets/{name}/delete`（管理台不用 DELETE）。
  - `GET /api/config` 的密钥字段**已摘除**，只有 `has_<field>` presence 布尔。
- **`src/lib/configDiff.ts`**：`diffPatch(baseline, draft)` 只产出变化叶子（对象递归、数组/标量整体替换、新增键出现），`countDirty` 给脏计数。服务端**没有删键语义**（出现即覆盖，未出现保持），所以 draft 删键不会进 patch，需要清空字段只能显式写 `null`。
- **`src/context/ConfigDraftContext.tsx`**：单草稿 + 一次 PUT。`save()` 只在 dirty 非空时发请求、带 `revision` 做乐观并发、409 时**保留 draft 且不自动重载**。
  - **保存成功后必须回读 `getConfig()`**，不能用本地 draft 冒充 baseline：GET 会重新打码（`has_*` 才准），服务端还会 normalize 并**丢弃空串密钥**（`sanitize_patch`）。拿 draft 当 baseline 会让「清空密钥」显示成功而实际仍生效、`has_*` 永远停在旧值——这是典型的假装成功。回读失败时置 `warning`（保存本身成功），不要报成保存失败。
  - 请求在途期间用户可能继续输入，回包后**只在 draft 未被并发修改时**才重置（`setDraft(current => current === submittedDraft ? ... : current)`），否则吞输入。
- **`src/lib/useConfigSchema.ts`**：`/api/config/schema` 只给字段元数据（`advanced_fields`、`app_sections`、`capabilities[].common_fields`、`implementations[].fields[]`），段结构本身仍是手写。
- **`src/context/AdvancedModeContext.tsx`**：简单/高级模式，localStorage `admin.config.advancedMode`，默认 `false`，读写异常只在本次会话降级。
- **UI 原语**：`src/components/ui/` 已补齐 shadcn `input/label/textarea/select/switch/tabs/dialog/alert-dialog/tooltip/checkbox/radio-group`；`src/components/config/` 有 `FieldRow`（高级模式门控，hint 不做次级灰）、`SectionCard`、`SaveBar`（`live`/`next_session`/`restart_only` 三档文案固定为「立即生效 / 对新会话生效 / 需重启 server 生效」）；`src/components/feedback/` 有自建 `ToastHost`（右下堆叠、4s、最多 4 条）、`DataState`、`UnavailableNotice`、`ConfirmDialog`。`ToastProvider` 已挂在 `App.tsx`。
- **`src/lib/useApiQuery.ts` 带请求序号**：只有最新一次请求能写回 state，避免快速切换目录/筛选时旧响应覆盖新响应。
- **PATCH 口径（W5 已反证）**：服务端 `merge_patch` 是递归增量合并，**没有按 section 整段替换分支**（`server/src/gateway/ws/config.rs:400-430`，`merge_client_patch` 无段特判）。所有段一律提交 `diffPatch()` 最小叶子 patch；旧 Kotlin「整段回传」是历史 UI 覆盖逻辑，**禁止照搬**。数组（如 `command.keywords`）整体替换。
- **表单原语 `src/components/form/*`**：`TextField` / `NumberField` / `SecretField` / `SwitchField` / `SelectField` / `TextAreaField` / `StringListField` / `FormNotice`。`NumberField` 默认 `integer: true`，VAD 3 个 f32 显式 `integer={false}`；非法/空值绝不写 draft，失焦回退草稿值。`SelectField` 遇未识别值**只告警不自动纠正**（`coerce` 保证数字枚举写回 number）。`StringListField` 用换行编辑 `string[]`。
- **密钥路径**：服务端 `SECRET_PATHS`（`server/src/gateway/ws/config.rs:74-86`）列出的路径明文只存 `secretValues`、不进 draft；`save()` 经 `withSecrets()` 用 `writePath()` 深度展开成嵌套对象（避免 `{"aiui.api_key": "..."}` 扁平 key 与浅合并覆盖同段普通字段）。空串 `setSecret` 删除该键、不进 patch；`dirtyCount = countDirty + 密钥数`。**前端没有 `SECRET_PATHS` 常量清单**——密钥是通用点分路径机制，`SecretField` 传 `path` + `hasPath` 即可，服务端已含 `llm.api_key`、`tts.xfyun.api_key` / `api_secret`，新增段无需改任何清单。`has_*` 键名是 `has_<叶子名>`（如 `llm.has_api_key`、`tts.xfyun.has_api_secret`）。**例外**：`server.expected_token` 与 `tts.xfyun.app_id` 不在 `SECRET_PATHS`，GET 明文返回；前者留空＝关闭鉴权（`TextField type="password"`），后者留空会被 `merge_patch` 当真值清空，两者都不能套 SecretField 的「留空保留」。`writePath()` **不处理数组**（`isJsonObject` 对数组返回 false，会替换成 `{}`），所以 `llm.fallback.providers` 这类对象数组段禁止用现有点分路径原语暴露。
- **advanced 与 hot**：`FieldRow` 按 `advanced` 门控；简洁模式 `dirtyPaths` 命中 schema 高级字段时二次确认，文案固定「本次改动含高级字段（…），请确认保存？」——不要写「未在界面显示」。advanced 归属以 `/api/config/schema` 顶层 **`advanced_fields` 并集**为准（`projection.rs:208` = `ADVANCED_KEYS` ∪ `ADVANCED_EXTRA_KEYS` ∪ `ADVANCED_APP_KEYS`），前端 `useConfigSchema.isAdvancedField(path)` 按**完整点分路径**比对：W4/W5 的 server/audio/playback 项来自 `ADVANCED_APP_KEYS`（`audio.downlink_sample_rate` 是、`audio.channels` 不是），**llm/tts 的能力字段来自 `ADVANCED_KEYS`**（`tts.cache_entries`、`tts.kokoro.*` 8 个、`llm.max_history`/`temperature`/`stream`）——不要只看 `ADVANCED_APP_KEYS`。`SaveBar` 导出的 `HOT_LABEL` 是 hot 三档文案唯一来源；`SectionHotHint` 只取 schema `app_sections.hot_hint`（`APP_SECTION_HOT` 无 llm/tts/vad/asr/command/wakeup，这些段返回 `null`），取不到返回 `null`，不自造文案。
- **弃用字段与默认值**：`command.match_mode` / `command.max_chars` 是兼容字段，**禁止 UI 暴露**（0 处渲染）；`command` 只开放 `enabled` / `keywords` / `reply`。`vad.engine` / `asr.engine` 的 `Config::default()` 是空串，但 `cfg.normalize()` 会经 `engine_id()` 规范成 `silero` / `sensevoice` **写盘并出现在 GET 回包**（`plugins/vad/mod.rs:65`、`plugins/asr/mod.rs:75`）；前端用 placeholder 展示这两个 id、不硬写 draft、不自动纠正未知值。
- **可选能力降级 ≠ 保存被拒绝**（W5 QA 反证，后续 NLU / Soul / Memory 等段同样适用）：`PUT /api/config` 全链路**无 `validate()`**——`save_patch_with`(`server/src/gateway/ws/config/save.rs:127-182`) → `merge_client_patch`(`server/src/gateway/ws/config.rs:327-342`) → `cfg.normalize()`(只 trim/clamp/去重，不重置默认) → `write_doc`(只序列化写盘)。所以 `command.keywords: []` 这类「对可选能力不合法」的配置**会 PUT 200 保存成功，不是 400**。`CommandConfig::validate` / wakeup 空池的 `bail!` 只在插件 host `plan()` 阶段的描述符钩子里调用（`plugins/registry/validate.rs:162-163`、`plugins/host.rs:125-140`），失败把可选能力标为 `Phase::Degraded`、`/api/plugins` 上报降级原因。**UI 文案必须说「运行时降级 / 不生效」，禁止写「服务端会拒绝保存」**；`WakeupSection.tsx:49` 是准确口径，`CommandSection.tsx:45` 曾写反、已按 QA D1 修正。
- **状态补充**：`reload(): Promise<boolean>`（预设应用后必须等 reload 成功再报成功）；`warning`（已保存但回读失败）必须在 `ConfigPage` 渲染；`error` 与 `conflict` 条职责分离。

## 本阶段明确未做

1. 硬件命令实现（麦克风 / Opus / 播放 / mDNS）。
2. GUI 启动 `pnpm tauri dev` / `pnpm tauri build`；编译侧只做 `cargo check` 与 `cargo test`（URL scheme 白名单 + 窗口加载模式决策单元测试）。Tauri 首次编译会下载并编译大量依赖，耗时明显长于前端构建。
3. `/em/` iframe 接入。
4. 真实业务页面迁移：Overview / Devices / Console / Files 已迁移，**Config 18 段全部迁移**（C0–C7），**Plugins 已迁移**（见「Plugins 页（已迁移）」）；Settings / Echomuse / devicehub / Testbench 仍待迁移。
5. 暗色主题、微前端、Rust server 源码改动与 CORS 层改动。
6. `remote.urls` 远程 IPC 授权：本轮查证后二选一明确**不配置**（见「插件」），生产服务器页面模式下插件 IPC 被 ACL 拒绝，剪贴板走浏览器降级链；不为此加异步能力探测或额外 UI。
7. 除 `@tauri-apps/plugin-clipboard-manager` 外的任何插件（清单与依据见「插件」），以及 `clipboard-manager:allow-read-text` 读权限。
8. mDNS 发现与服务器地址配置页：局域网/生产部署需显式设置 `XIAOZHI_ADMIN_URL`，本轮只提供默认值 `http://127.0.0.1:8000`。
9. `src/lib/clipboard.ts` 的 `copyText()` 已接入 Console「复制全部日志」，返回 `boolean`（三级降级全失败返回 false，不抛到 UI），失败时有 Toast 明确提示；其它占位页无真实复制动作，不做无意义接入。
10. bundle code-splitting：**W5 QA 明确结论——延后到页面全迁完、有浏览器可做真实首屏测试时统一处理；W11 复核：页面已全迁但浏览器验收环境仍不具备，继续维持单包**。九波增量 500→506.24→514.5→537.6→579.42→613.57→624.68→638.34→**669.02 kB**（gzip 210.21；W7 +41.9 / W8 +34.12 连续放大，W9 +11.11 / W10 +13.43 回落，W11 +30.68——Plugins 含 A2UI 解析器/渲染器）；`>500 kB` 是 warning 非 error、build Exit 0；本机无浏览器 / 远程部署环境，无法验收分包后的运行时加载与远程静态资源路径，且 Tauri 生产加载远程页面时分包反增 RTT。将来切入点：`router.tsx` 把 `PAGE_COMPONENTS` 各页面改 `React.lazy` + `<Suspense>` 按路由整体懒加载（研究推荐方案，一次改动收益最大），或 Vite `build.rolldownOptions.output.codeSplitting` 手工分包（**实施前须实测本 Vite 版本实际支持的配置键**）；A2UI 解析器/渲染器可作二级懒加载（只在进插件运行页时 `import()`）。

## 后续迁移待办

- 按 `src/lib/nav.ts` 的 **9 个描述符**逐页迁移 Kuikly adminCore 逻辑，**数据源统一走新增的 `@/lib/adminApi`**：Overview / Devices / Console / Files / Config(18 段) / **Plugins** **已完成**；后续顺序 devicehub → Settings → Testbench(仅 Tauri 硬件) → 旧端下线。
- 页面里需要复制的交互统一调用 `@/lib/clipboard` 的 `copyText()`（三级降级、不会抛错到 UI），不要各自摸 `navigator.clipboard`。
- 生产/局域网部署前显式设置 `XIAOZHI_ADMIN_URL`；后续可接 mDNS 发现或地址配置页，替代默认 `http://127.0.0.1:8000`（本轮未实现）。
- 若确实需要在服务器页面启用 Tauri 插件 IPC，按「插件」小节的模板显式配置 `remote.urls` 并复核官方 remote API 安全提示；同时重评剪贴板信任边界。
- 需要硬件能力时按上文扩展点添加窄命令与最小 capability 权限，并复核 macOS plist 权限声明。
- 若 IDE 从仓库根打开，按方案 (b) 把工作目录切到本目录后再用 shadcn MCP。
