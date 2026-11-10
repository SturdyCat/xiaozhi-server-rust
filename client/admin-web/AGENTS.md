# client/admin-web 开发指引

React + shadcn/ui + Tauri 2 管理台开发环境。**本阶段只搭环境，不做页面迁移**：8 个页面均为「待迁移」占位骨架；业务逻辑一律留在前端 SPA，Rust 侧只有桌面壳与窄命令骨架。本目录不写 README.md，说明统一收敛到本文件。

## 工具链与命令

- 已验证：Node v22.22.2 / pnpm 12.15.0 / npm 10.9.7 / cargo 1.98.1 / Xcode。Vite 8 要求 Node `^20.19 || >=22.12`，本机满足。
- **不要用 yarn**（本机 corepack 会交互卡死）；下列 pnpm 命令均在 `client/admin-web` 目录执行。
- `pnpm install`：安装依赖。`pnpm-lock.yaml` 由真实安装生成；pnpm 12 会额外生成 `pnpm-workspace.yaml` 记录 `minimumReleaseAgeExclude`，属正常产物，不要删除。
- `pnpm dev`：Vite dev server，`http://127.0.0.1:5173`（`strictPort: true`；`server.host` 固定 `127.0.0.1`，与 `tauri.conf.json` 的 `devUrl` 一致，避免 `localhost` 在 IPv6 `[::1]` 与 IPv4 `127.0.0.1` 之间解析不一致导致 Tauri 连不上；不要改成 `true` 监听所有网卡）。
- `pnpm build`：`tsc` 双 project 类型检查 + `vite build`，产物 `dist/`（含 `dist/index.html`）。
- `pnpm preview`、`pnpm typecheck`。
- `pnpm tauri dev` / `pnpm tauri build`：Tauri CLI 走项目 devDependency `@tauri-apps/cli`；本机没有全局 `cargo-tauri`。
- `cd src-tauri && cargo check` / `cargo test`：Rust 侧编译校验与 URL scheme 白名单单元测试。
- 开发期先启动 Rust server（默认 `http://127.0.0.1:8000`），否则 `/api`、`/ws`、`/em/` 代理会失败。 Rust server 当前无 CORS 层，因此开发/生产都依靠同源访问。

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
    lib/      utils.ts(cn) / api.ts(API baseURL) / platform.ts(能力检测) / nav.ts(导航单一数据源)
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

- `src/lib/api.ts` 是唯一 API 入口：默认同源相对 `/api`，可用 `VITE_API_BASE_URL` 覆盖；`apiUrl()` / `requestApi()` / `getWebSocketUrl()` 都从这里走。生产由 Rust `ServeDir` 同源托管（`dev-server.sh` 的 `XIAOZHI_ADMIN_DIR` 可指向本目录 `dist/`），Tauri 远程 URL 模式同理，**均无 CORS 问题**。
- Vite dev proxy：`/api`、`/ws`（`ws: true`）、`/em` → `VITE_PROXY_TARGET`（默认 `http://127.0.0.1:8000`）。
- **`/em/` 排除口径**：proxy 中间件先于 SPA history fallback 生效，因此 `/em/`（含尾斜杠）会原样转发到 Rust server，不会被 `index.html` 吞掉。前端路由不得占用 `/em/` 前缀；后续官方页用 `<iframe src="/em/?...">`（带尾斜杠 + URL 参数）接入，**本阶段不实现**。微前端方向已否决。
- 路由使用 `HashRouter`：静态托管与 Tauri 内嵌 SPA 都没有服务端 history fallback，避免刷新／深链接 404。
- `vite.config.ts` 按 shadcn 官方 Vite 集成使用 `path.resolve(__dirname, "./src")` + `@types/node`；Vite 8 会提示 native configLoader 未来不支持 `__dirname`（建议 `import.meta.dirname`）。当前 `pnpm build` 通过，若后续升级到默认 native loader，改为 `import.meta.dirname` 并同步保持 alias 与 `tsconfig*.json` 的 `paths` 一致。

## Tauri 两种加载模式

- **默认内嵌 SPA（推荐）**：`build.frontendDist: "../dist"`、`build.devUrl: "http://127.0.0.1:5173"`；`tauri.conf.json` 里 `app.windows: []`，窗口由 `src/lib.rs` 的 `setup` 用 `WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))` 创建（label 固定 `main`）。开发模式下 Tauri 按 `devUrl` 加载 Vite dev server；`vite.config.ts` 的 `server.host` 同为 `127.0.0.1`，两者必须一起改。
- **加载服务器页面（远程 URL 模式）**：运行前设置 `XIAOZHI_ADMIN_URL=http://127.0.0.1:8000`，`resolve_window_url()` 调用 `parse_external_url()`：先做 URL 语法解析，再做 `http` / `https` scheme 白名单校验（对应 tauri-utils 对 `WebviewUrl::External` 「Must use either the http or https schemes」的要求，直接构造枚举会绕过 Deserialize 分类），通过后改用 `WebviewUrl::External`；`ftp://`、`file://`、`data:`、空串与非法串都会返回明确 Err（错误信息区分「语法错误」与「scheme 不支持」，`cargo test` 有单测覆盖）。窗口直接是服务器页面，`/api` 相对路径与 `/em/` iframe 天然同源。若某版本 dev 模式未自动映射 `devUrl`，也可用该变量显式指向 `http://127.0.0.1:5173` 达到同样效果。
- **远程模式的已知限制（本阶段未解决，不写成已实现）**：`capabilities/default.json` 未配置 `remote.urls`。Tauri 官方文档「By default the API is only accessible to bundled code shipped with the Tauri App」，且 `tauri` 源码 `webview/mod.rs` 的 `!is_local` ACL 检查会拒绝远程页面 IPC，因此 `invoke("app_capabilities")` 会失败；但 init script 仍无条件注入 `isTauri`，`testbench` 路由照常显示，`getCapabilities()` 静默降级为 Web 兜底值（带 `tauri: true`）。**默认推荐内嵌 SPA 模式**；确需远程模式必须按下方模板显式配置 `remote.urls` 并评估官方 remote API 安全提示，本阶段没有实现该配置，也不为此加异步能力探测或额外 UI。
- `app.withGlobalTauri: false`（不注入 `__TAURI__`）。
- capability：`capabilities/default.json` = `identifier: "default"`、`windows: ["main"]`（与窗口 label 一致）、`permissions: []`。按 Tauri 2 官方文档，通过 `invoke_handler` 注册的应用自定义命令默认对所有窗口可用，因此**不需要 `core:default`**；`tauri.conf.json` 用 `app.security.capabilities: ["default"]` 显式引用，避免目录里其它文件被自动启用。
- 远程 URL 的 capability 授权模板（**默认不开启**，JSON 不支持注释，故只写在本文件，不落盘成 .json）：

  ```json
  {
    "$schema": "../gen/schemas/desktop-schema.json",
    "identifier": "remote-example",
    "windows": ["main"],
    "remote": { "urls": ["https://admin.example.com"] },
    "permissions": []
  }
  ```

  `capabilities/` 目录内所有 .json/.toml 会自动启用，所以模板不要放进该目录。启用远程 IPC 前务必评估 Tauri 官方 remote API 安全提示（Linux/Android 无法区分 iframe 与主窗口请求）。

## 能力检测与硬件命令扩展点

- `src/lib/platform.ts`：`isTauri()` 优先调用 `@tauri-apps/api/core` 官方 `isTauri()`（实现为 `!!globalThis.isTauri`，由 Tauri 2 init script 无条件注入），再以 `__TAURI_INTERNALS__` 兜底双保险；同步返回 boolean，**不做 UA 判断**，且不受 `withGlobalTauri: false` 影响。`getCapabilities()` 封装 `invoke("app_capabilities")`，浏览器环境返回 Web 兜底值。
- `router.tsx` 与 `AppShell.tsx` 共用 `src/lib/nav.ts` 的 `visibleNavDescriptors(isTauri())`；`testbench` 的 `tauriOnly: true` 与现有 `NavDescriptor.macOnly` 同构，Web 环境既不注册路由也不显示入口。
- `src-tauri/src/commands/mod.rs` 目前只有示例窄命令 `app_capabilities`，返回 `{ platform, tauri, version }`，与前端 `AppCapabilities` 字段一致。后续麦克风采集 / Opus 编解码 / 音频播放 / mDNS 发现在此加 `#[tauri::command]`，并在 `src/lib.rs` 的 `generate_handler!` 注册，再按需开启最小 capability 权限。
- `src-tauri/Info.plist`、`Entitlements.plist` 是**预留模板，当前未被 tauri.conf.json 引用**，含 `NSMicrophoneUsageDescription`、`NSLocalNetworkUsageDescription`、`NSBonjourServices` 等键；启用前需按实际签名／沙箱策略复核。
- **硬件测试边界**：接口返回 200 或页面存在不等于真机硬件通过，结论必须由真机测试给出；Web 不露出任何硬件动作。

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

## 本阶段明确未做

1. 硬件命令实现（麦克风 / Opus / 播放 / mDNS）。
2. GUI 启动 `pnpm tauri dev` / `pnpm tauri build`；编译侧只做 `cargo check` 与 `cargo test`（URL scheme 白名单单元测试）。Tauri 首次编译会下载并编译大量依赖，耗时明显长于前端构建。
3. `/em/` iframe 接入。
4. 真实业务页面迁移（8 个占位页全部显示「待迁移」）。
5. 暗色主题、微前端、Rust server 源码改动与 CORS 层改动。

## 后续迁移待办

- 按 `src/lib/nav.ts` 的 8 个描述符逐页迁移 Kuikly adminCore 逻辑，数据源统一走 `@/lib/api`。
- 需要硬件能力时按上文扩展点添加窄命令与最小 capability 权限，并复核 macOS plist 权限声明。
- 若 IDE 从仓库根打开，按方案 (b) 把工作目录切到本目录后再用 shadcn MCP。
