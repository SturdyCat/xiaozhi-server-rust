# W15 / 生产构建链前置只读研究 · 权威归档

> **文档性质**：W15 / 旧端下线前置研究「生产构建链」分路的**唯一权威契约**，供工程师实施与 QA 回归逐节核对。由研究阶段发送给 team-lead 的「报告 1/2 + 报告 2/2 + 勘误补充」**原文全量合并**而成，研究原文一字未删。
> **研究纪律**：研究全程只读——未创建/修改任何源码文件、未执行构建、未做 git 写操作（仅 `git ls-files` / `git check-ignore` / `cat` / `ls` 等只读查询）；本归档是经 team-lead 授权写入的单一文档。
> **证据口径**：全部结论带 `file:line`；根路径 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/`，下文 `file:line` 均为省略该前缀的相对路径。目录构成、产物字节数、入库/忽略状态均为 `ls` / `git check-ignore` / `git ls-files` 实测。
> **勘误口径（本归档以最新为准）**：报告 1/2 §1.1 的「em 源目录构成」段曾把下发产物 `web/em/static/` 的 3 文件与字节数误当成源目录、漏了 `static/dashboard.jsx`，隐含 12 文件口径；**最新口径为 13 文件全部入库**（根 4 + `static/` 4 含 `dashboard.jsx 648322` + `tests/` 5 个 `.cjs`），以文末「勘误补充」为准。原文按「零删减」保留，实施与 QA 一律按 13 口径读。
> **推断项待实测**：ServeDir 对 `/em/` 目录追加 `index.html`、`.dockerignore` 任意层级 `logo.png` 匹配、Vite 8 `public` 拷贝发生在 `emptyOutDir` 之后三处，是基于现有产物证据与文档口径的推断，报告中已标注「需实测」，实施前须用真实构建/运行确认。
> **形态结论（实施口径）**：用户已拍板形态 ②（软下线归档）+ 生产全量切换到 `client/admin-web/dist`，em 落点 **C publicDir**；team-lead 裁定 C 的实施口径为**白名单 4 文件**（`index.html` + `static/{adapter.js,dashboard.js,host.js}`），以规避 R13。下方 §4 三形态、§5 需拍板清单均为研究阶段「只给事实、不替下结论」的原文，拍板后按上表裁减即可。

## 已拍板结论（用户经 team-lead 确认，2026-11-12）

| 编号 | 拍板项 | 结论 |
|---|---|---|
| 1（形态） | 旧端下线形态 | **② 软下线归档**：切到 `admin-web/dist` 并归档/删除旧端目录（与研究另一路资产盘点衔接），不是形态①「旧端目录保留」、也不是形态③「维持 h5App/web 作默认」 |
| 2（em 落点） | em 资产落点 | **C publicDir**；**team-lead 裁定实施口径为白名单 4 文件**：只复制 `index.html` + `static/{adapter.js,dashboard.js,host.js}`，不整目录拷 `em/` 源目录 |
| 3（切换范围） | 生产切换范围 | **生产全量切换**：`[server].admin_dir` / `XIAOZHI_ADMIN_DIR` / Docker 镜像静态目录全量指向 `admin-web/dist` |
| 4（文档口径） | 47 个 md 文档 | **全量改**（口径同步范围由用户拍板全量，不裁剪） |

> **C 方案白名单裁定说明**：C 本身与 `emptyOutDir:true` 天然兼容（Vite 在同一次 build 内、清空 `outDir` 之后拷贝 `public/*` → `dist/*`，每次重拷，不存在手工拷贝被清掉）；但 `em/` 源目录含 `dashboard.jsx`(648 KB) 与 `tests/` 5 个 `.cjs`（见 R13），若整目录拷进 `public/em/` 会一并下发。**故裁定只落白名单 4 文件**：这既满足 `/em/` 官方页的运行时最小集（`em/index.html:587-591` 只引 `static/` 三 js + CDN 库），又规避 `dashboard.jsx` 源码、`LICENSE`/`README*` 与 `/em/tests/*.cjs` 扩散到 `dist/em`。三选项收口见「勘误补充」勘误 2。

> 以上为**实施裁剪依据**。下方 §1–§6 与「勘误补充」均为**研究原文全量保留、零删减**：§4 三形态仍按「不替下结论」给全量改动面与风险，§5 八项需拍板仍按研究阶段待拍板表述，§6 R1–R12 为原文风险登记、R13 由勘误补充新增。**如遇口径冲突，以本次落盘前逐行核读的源码 `file:line` 与文末勘误为准。**

## 目录与合并顺序

| 区段 | 内容 | 来源 |
|---|---|---|
| §1 当前生产构建链全链路 | Gradle `publishWeb` 任务依赖图与 `web/`、`em/` 目录构成；admin-web Vite/`dist` 产物与 `public/` 机制；Tauri 壳 `decide_window_url` 四分支与 `DEFAULT_SERVER_URL`；server `ServeDir`/`ServeFile` fallback、`admin_dir` 读取链与覆盖语法；Docker/CI/文档口径 | 报告 1/2 原文（一） |
| §2 切到 `admin-web/dist` 的完整改动清单 | em 落点三路径 A/B/C、`emptyOutDir` 冲突与清理口径、构建顺序、鉴权与相对路径、回滚路径、Docker/CI 改动面 | 报告 1/2 原文（二） |
| §3 验收矩阵 | 8 项路径清单（怎么验 / 正常预期 / 失败信号）+ `/em/` 返回官方 index.html 的可机器验证 shell 正负断言 | 报告 2/2 原文（三） |
| §4 形态选项 | ① 只切 admin_dir + 新增 em 拷贝 / ② 切 dist + 归档删除旧端 / ③ 维持 h5App/web 作默认 | 报告 2/2 原文（四） |
| §5 需拍板清单 | 8 项 | 报告 2/2 原文（五） |
| §6 风险登记 | R1–R12 | 报告 2/2 原文（六） |
| 勘误补充 | em 源目录 13 文件口径更正、`/em/` 下发面新增事实与收口三选项、R13、与另一路研究口径一致性 | 勘误补充原文 |

---

> **以下为「报告 1/2」原文，全量保留，未删减（章节标题按 §1–§6 规范化编号，正文一字未删）。**

【W15 生产构建链研究 1/2（当前构建链全链路 + 切到 admin-web/dist 改动清单）】只读研究，未改文件/未构建/未 git 写操作。仓库根 /Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust，以下 file:line 均为绝对路径省略该前缀。行号按当前文件实测（admin-web/AGENTS.md 后续 W 有增补，W12 文档引用的 :43/:51/:56 与现状有漂移，方案 B 表述现落在 :50/:51/:56-64）。

## §1 当前生产构建链全链路

■ 1.1 Gradle h5App 链（client/apps/h5App/build.gradle.kts，全文 124 行）
任务依赖图（publishWeb :106-123，dependsOn 在 :109）：
```
publishWeb (:106-123)
├─ :shared:verifyAdminIcons (:109，shared 外部任务)
├─ verifyEmBundle (:72-96，verification 闸门)
├─ syncBusinessBundle (:38-54) ─ dependsOn :shared:packLocalJSBundleRelease (:41)
│    从 shared/build/outputs/kuikly/js/release/local/nativevue2.zip 解出 nativevue2.js → web/ (:42-44)
│    doFirst 校验 zip 存在 + 无过时位图图标 (:45-53)
├─ syncShellBundle (:55-61) ─ dependsOn jsBrowserProductionWebpack (:58)
│    从 kotlin-webpack/js/productionExecutable 拷 h5App.js → web/ (:59-60)
├─ syncEmAssets (:98-104) → em/ → web/em/ (:65 emDir、:102-103 from/into)
│    exclude(README.md, README.sha256, LICENSE, static/dashboard.jsx) (:102)；**tests/ 未排除**（w12-devicehub-research.md:54 实测拷进 web/em/tests）
└─ doLast (:110-122) 校验 web/index.html 里两条 `./(nativevue2|h5App).js?v=` 引用 (:114-119) 并写入 Instant.now().epochSecond 时间戳 (:113/:120)
```
- webDir = layout.projectDirectory.dir("web") (:37)；emDir = dir("em") (:65)；js(IR) browser + webpackTask h5App.js (:14-24)。
- verifyEmBundle 闸门：dashboard.js 首行 `em-bundle src-sha256=` 须等于 dashboard.jsx 摘要 (:79-86)、须含「App auto-mount removed」(:87-89)、static/{adapter.js,host.js,dashboard.jsx} 存在 (:90-92)、LICENSE 存在（MIT 随分发，:94）。
- 注意 publishWeb doLast 会 **写回被 git 追踪的 web/index.html**（client/.gitignore:41-46 注释「index.html 入库（必须保留）」；client/.gitignore:44 `apps/h5App/web/*.js`、:46 `apps/h5App/web/em/` 才忽略；git check-ignore 实测 web/em/index.html 命中 :46）。
- 目录构成（ls 实测）：
  · client/apps/h5App/em/（源，入库）：LICENSE、README.md、README.sha256、index.html(28417)、static/{adapter.js 29400, dashboard.js 440653, host.js 12937}、tests/ 5 cjs。
  · client/apps/h5App/web/：index.html(1605，:6 `<title>XiaoZhi Admin</title>`、:24 `./nativevue2.js?v=1794128076`、:26 `./h5App.js?v=1794128076`)、nativevue2.js(3893207)、h5App.js(352570)、em/{index.html 28417, static/{adapter.js,dashboard.js,host.js}, tests/}。

■ 1.2 admin-web Vite 链
- client/admin-web/vite.config.ts（全文 51 行）：**无 base 配置**（默认 "/"）→ dist 资源是绝对路径 /assets/；build{ outDir:"dist" (:45), emptyOutDir:true (:46), target/minify/sourcemap 由 TAURI_ENV_* 控制 (:47-49) }；**无 publicDir 配置、无任何 em 拷贝**（全文）；dev proxy `/api`(ws:true,:38)、`/ws`(ws:true,:39)、`/em`(:40) → proxyTarget=VITE_PROXY_TARGET||http://127.0.0.1:8000 (:14)；:33-34 注释「proxy 中间件先于 SPA history fallback 生效，/em/（含尾斜杠）原样转发到 Rust server」。
- client/admin-web/package.json:6-14 scripts：build="tsc app --noEmit && tsc node --noEmit && vite build" (:8)；dev/preview/typecheck/format/tauri。**em|h5App|copy|publishWeb|dist 零匹配**（w12:404），即无任何把 em 拷进 dist 的脚本。依赖 vite ^8.3.7 (:41)，lock 为 pnpm-lock.yaml + pnpm-workspace.yaml（Glob 实测）。
- dist 产物（ls 实测）：assets/{index-BCpFm4NW.js 755226, index-ae9fZm_2.css 56459}、index.html(510)、logo.png(383356)；**无 dist/em/**。client/admin-web/dist/index.html:7-13：`<link rel="icon" href="/logo.png">`、`<title>小智管理台</title>`、`/assets/index-BCpFm4NW.js`、`<div id="root">`。
- **public/ 机制已存在**：client/admin-web/public/ 仅 logo.png(383356)；由 scripts/gen-icons.py:26 `PUBLIC = APP/"public"`、:207 写 public/logo.png。Vite 默认 publicDir="public"，build 时把 public/* 拷到 dist/*（dist/logo.png 与 public/logo.png 同尺寸佐证）。dist/ 被忽略（.gitignore:53、client/admin-web/.gitignore:2；git check-ignore 实测命中）；public/ 未被忽略。
- 用 HashRouter（src/App.tsx:2/:12、src/lib/nav.ts:6「对应 /#/{key}」、AGENTS.md:53）。

■ 1.3 Tauri 壳
- client/admin-web/src-tauri/tauri.conf.json:6-11：beforeDevCommand "pnpm dev"、beforeBuildCommand "pnpm build"、devUrl http://127.0.0.1:5173、`frontendDist: "../dist"`（相对 src-tauri = admin-web/dist）；bundle.active true/targets "all" (:20-22)。
- client/admin-web/src-tauri/src/lib.rs：DEFAULT_SERVER_URL="http://127.0.0.1:8000" (:28)、REMOTE_URL_ENV=XIAOZHI_ADMIN_URL (:21)、EMBEDDED_ENV=XIAOZHI_ADMIN_EMBEDDED 精确 "1" (:31-32)、EMBEDDED_ENTRY="index.html" (:18)。decide_window_url 四分支 (:70-88)：①is_dev→App("index.html") 映射 devUrl (:75-77/:63-65)；②embedded=="1"→App (:79-81)；③remote 非空白→parse_external_url (:83-84)；④否则→DEFAULT_SERVER_URL 的 External (:86)。:26-27 注释「服务器页面与 Rust server 同源，/api、/ws、/em/ 相对路径天然可用；内嵌 tauri://localhost 下只打到资产协议，API 不可达」。
- **关键事实**：方案 B（admin-web/AGENTS.md:56-64）生产壳窗口加载的是 http://127.0.0.1:8000 的**服务器页面**，即 server 的 admin_dir 目录；内嵌 frontendDist 副本只在 XIAOZHI_ADMIN_EMBEDDED=1 时用（且 API 不可达，AGENTS.md:64）。⇒ admin_dir 切到 dist 直接决定 Tauri 生产壳看到的页面与 /em/ 是否可达。

■ 1.4 server 静态托管链
- server/src/gateway/ws/mod.rs router (:149-341)：`api` Router 注册全部 /api/*（:152-324，含 /api/config :157-160、/api/admin/ws :156、/api/ws :154、/api/logs :240、/api/health :153、各 feature 门控 merge :245-324）。**server 侧无裸 /ws 路由**（grep 全 src 确认；api.ts:142-143 注释「Rust server 无裸 /ws 前缀」）。
- :326-334 核心分支：`Path::new(&admin_dir).is_dir()` → `ServeDir::new(&admin_dir).precompressed_gzip().not_found_service(ServeFile::new(admin_dir/index.html))` (:328-330) → `api.fallback_service(admin)` (:331)；否则 `api.fallback(admin_missing)` (:333)，admin_missing 返回 404 中文「管理页面未找到：请先构建 h5App（web）…」(:353-359)。
- 分流顺序（Axum：.route 具体路由优先，fallback_service 只接未匹配路径）：
  · /api/* 已注册 → API handler；**未注册的 /api/\***（pure-base 下 /api/echomuse/*、/api/soul/*、/api/ota/firmware 等）→ 落 ServeDir → 找不到文件 → not_found_service → **SPA index.html HTML 200**（AGENTS.md:169/:170/:171/:175、w12:124/:306 多处证实）。
  · /ws → 无注册路由 → 落 fallback → SPA index.html HTML 200（生产前端不用 /ws；getWebSocketUrl 固定 /api/admin/ws）。vite proxy /ws (vite.config.ts:39) 是 dev 期遗留转发。
  · /em/ → 无注册路由 → 落 ServeDir：admin_dir 含 em/index.html 时命中官方页；无 em/ 时 not_found_service → **SPA index.html HTML 200**（已知事实 + w12:409）。ServeDir 对目录请求默认追加 index.html（emEmbed.ts:9-10、em/README.md:182-183 的尾斜杠契约）。
  · /em/status → 是 echomuse-plugin 门控 **API 路由**（server/src/device/echomuse/routes.rs:68，与 /em/control/:63、/em/data/:64 同组）：feature 编入时 200 JSON `{"enabled":bool,"devices":[...]}` (:71-75，无鉴权)；pure-base/dev 下不挂载 → HTML 200（w12:124/:306、echomuseApi.ts:277/:290-293 归一为 EchomuseUnavailableError(not-compiled)）。**与 /em/ 静态目录同前缀但性质不同**。
- admin_dir 配置读取链（覆盖语法）：
  · server/src/config.rs:217-220 字段 + `#[serde(default="default_admin_dir")]`；:585-587 默认 `"../client/apps/h5App/web"`；:298-306 Config::default 用它；server/config.example.toml:18-21 示例同值、:20「Docker 部署无需修改——镜像内置 XIAOZHI_ADMIN_DIR=/app/web 环境变量覆盖」。
  · server/src/main.rs:15 文档；**load_config() 最后覆盖**：`if let Ok(dir)=env::var("XIAOZHI_ADMIN_DIR") { if !dir.is_empty() { config.server.admin_dir = dir } }` (:334-337)，注释 :332-333「env 只影响本次运行生效配置，不写回（GET /api/config 展示仍是库内原值）」，main.rs:14「仅影响本次运行，不写回」。优先级：**XIAOZHI_ADMIN_DIR > SQLite 库内值 > 内置默认**。
  · server/src/dev.rs:83-92：--dev 模式且 XIAOZHI_ADMIN_DIR 未设/为空时，设为 CARGO_MANIFEST_DIR/../client/apps/h5App/web 绝对路径。dev guard :107-143（blocked 列表 :97-105 含 /api/ws、/em/control、/em/data，不含 /em/ 与 /em/status）。
  · dev-server.sh:10 `WEB="${XIAOZHI_ADMIN_DIR:-$ROOT/client/apps/h5App/web}"`；:11 检查 index.html/nativevue2.js/h5App.js 三件套；:12-15 自定义目录缺资产直接 exit 1「Custom XIAOZHI_ADMIN_DIR has missing assets」；:16 只有默认目录触发 publishWeb；:18 export XIAOZHI_ADMIN_DIR="$WEB"；:31 `cargo run --no-default-features -- --dev`。
  · README.md:97「XIAOZHI_ADMIN_DIR 可指定已有 Web 目录」。
  · admin_dir 是配置库可写字段：plugins/registry/projection.rs:114 `"server.admin_dir"`；管理台 UI 可编辑（client/admin-web/src/components/config/ServerSection.tsx:63-69 path="server.admin_dir"，:72-74 警告「修改静态目录后当前管理页面可能无法再加载」）；改库内值需重启生效（mod.rs:46「管理页目录等仍需重启 server」），且 env 覆盖优先。
  · 覆盖语法：`XIAOZHI_ADMIN_DIR=/abs/or/rel/path`（运行 server 前 export，或 compose environment / Docker ENV）；或 PUT /api/config 改 server.admin_dir + 重启。

■ 1.5 Docker / CI / 文档口径
- Dockerfile：web stage `FROM gradle:8.14.5-jdk17-jammy AS web` (:138)；:217-222 第一条 RUN 编译 + 双 webpack；:231-240 第二条 RUN `NODE_OPTIONS="--max-old-space-size=${NODE_MAX_OLD_SPACE}" gradle :apps:h5App:publishWeb --no-daemon ... -PwebSourceMap=false` (:232-233) + test 校验 index.html/nativevue2.js/h5App.js/**em/index.html、em/static/dashboard.js、em/static/adapter.js** (:235-240)；:260 串行屏障 `COPY --from=web /src/client/apps/h5App/web/index.html /tmp/.web-stage-done`；:398-399 `COPY --from=web /src/client/apps/h5App/web /app/web`；:416/:423 `ENV XIAOZHI_ADMIN_DIR=/app/web`；:434 HEALTHCHECK /api/health。
- **Dockerfile 完全不碰 admin-web / pnpm / vite**（grep 零匹配）；dist/ 不入库 ⇒ 当前镜像里根本没有 admin-web/dist。
- .dockerignore：:2-4 三阶段说明「web（Gradle 构建 Kuikly H5 管理页）」；:33 `*.md`、:36 `logo.png`（按 :5 注释「不含 / 的模式匹配任意层级同名文件」，推断 client/admin-web/public/logo.png 也被排除，需实测）、:29 `scripts`（推断 client/admin-web/scripts 亦被排除）、:51 `**/node_modules`、:61-62 `client/apps/h5App/web/*.js`；**client/admin-web 目录本身未被排除**（源码会进构建上下文）。
- docker-compose.yml:44-56 environment 仅 XIAOZHI_CONFIG_DB / XIAOZHI_AUTO_DOWNLOAD_MODELS / GITHUB_PROXY / RUST_LOG(注释)，**不覆盖 XIAOZHI_ADMIN_DIR**，依赖镜像内置 /app/web (:423)；:43 network_mode: host。
- .github/workflows/docker-amd64.yml（唯一 workflow）：workflow_dispatch 手动 (:35-41)，docker/build-push-action@v6 file ./Dockerfile (:127-133)，build-args 仅内存/并行档 (:144-149)，:69-93 静态守卫 grep Dockerfile/docker-compose；无 publishWeb/admin_dir 直接逻辑。
- 文档口径：client/admin-web/AGENTS.md:50（「生产由 Rust ServeDir 同源托管（dev-server.sh 的 XIAOZHI_ADMIN_DIR 可指向本目录 dist/）」）、:51（dev proxy）、:52（/em/ 排除口径 + iframe `/em/?...` 接入）、:53（HashRouter）、:56-64（方案 B 三模式表）、:66（XIAOZHI_ADMIN_URL / 默认 127.0.0.1:8000）、:225（**W15 旧端下线：切 admin-web/dist 构建链、验收矩阵 + 灰度前置、归档 client/shared 管理台与 h5App/web**）；client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md:143/:146（构建 web 管理页 / admin_dir 默认 h5App/web）；client/AGENTS.md:17（publishWeb -PwebSourceMap=false）；docs/plans/admin-web-migration/w12-devicehub-research.md:399-419 补充勘误（与已知事实一致：:402 dist 无 em、:403 vite 无 publicDir/em 拷贝 + emptyOutDir、:404 package.json 零拷贝脚本、:405 两条独立构建链、:409 /em/ 落 SPA HTML 200、:410 emptyOutDir 清空手工拷贝、:414-417 改动清单与清理口径）。
- 本机坑：docs/plans/gearui-kit-t13-acceptance.md:9「涉 Node 的任务加 env -u NODE_OPTIONS」、:83 `cd client && env -u NODE_OPTIONS ./gradlew :apps:h5App:publishWeb --console=plain`。与 Dockerfile:232 显式 `NODE_OPTIONS="--max-old-space-size=..."` 是**不同语境**（Docker 干净 gradle 镜像需限制 V8 堆；本机用户环境注入的 NODE_OPTIONS 致 webpack EEXIST，故 unset）。

## §2 切到 admin-web/dist 的完整改动清单（逐项可行性事实）

■ 2.1 em 资产落点三路径
【A｜Vite 插件 / 自定义脚本（build 后拷 em → dist）】
- 改 client/admin-web/vite.config.ts（加 writeBundle/closeBundle 钩子）或 package.json:8（build 脚本串 `&& node scripts/copy-em.mjs`），新增拷贝脚本。
- 冲突：`emptyOutDir:true` (vite.config.ts:46) 每次清空 dist → 手工拷入的 dist/em 在下一次 `pnpm build` 必被清掉（w12:410）。解法：拷贝必须与 vite build 在**同一条命令里且排在其后**，或用插件在写盘后执行；「先拷后 build」「单独拷一次」都会丢。
- 资产来源须为筛选后的 em（沿用 build.gradle.kts:102 exclude：README.md/README.sha256/LICENSE/static/dashboard.jsx；tests/ 未排除），不能直接拷 em/ 源目录（会把 jsx 源码与仓库文档带进 dist）。

【B｜Gradle syncEmAssets 增加第二落点】
- 改 client/apps/h5App/build.gradle.kts（emDir :65、webDir :37；dist 在跨项目路径，需相对 `../../admin-web/dist/em` 与 Gradle layout 处理），把 syncEmAssets (:98-104) 增一个 into 或新增 Copy 任务并挂进 publishWeb (:109)。
- 好处：复用 verifyEmBundle 闸门 (:72-96) 与 exclude 口径，一处定义双落点。
- 冲突：emptyOutDir 会清空 dist ⇒ **构建顺序必须是 pnpm build 先、Gradle publishWeb 后**，且之后不能再跑 vite build；顺序耦合是最大隐性风险。Docker 里两条链分属不同 stage，需要把 dist 产物在 stage 间传递（现有 :260 屏障只从 web→build）。

【C｜publicDir 配置（em 放进 admin-web/public/em/）】
- 机制已具备（见 1.2）：public/ 存在且 Vite 默认自动拷 public/* → dist/*；**与 emptyOutDir:true 天然兼容**——public 拷贝发生在同一次 vite build 内、清空 outDir 之后，每次 build 都会重拷，不存在「手工拷了被清掉」。三路径中唯一不冲突者。
- 但需保证 public/em/ 内容是筛选后产物（同 A 的 exclude 口径），且不能覆盖 gen-icons.py:207 生成的 public/logo.png。
- 若改 vite.config.ts 的 publicDir 直接指向 `../apps/h5App/em`：Vite publicDir **无 exclude 能力**，会把 README.md/README.sha256/LICENSE/static/dashboard.jsx 与 tests/ 全部拷进 dist（build.gradle.kts:94 校验 LICENSE 存在；w12:54 tests/ 被拷）⇒ 不推荐直接指源目录。
- 副作用：public/em 当前**未被 gitignore**（admin-web/.gitignore 只忽略 dist/），若作为拷贝产物需新增 .gitignore 规则，否则 500KB+ 产物入库。

【em/README.md:184-185 清理口径同步（双落点）】
- 现文 :184-185「web/em/ 是 syncEmAssets 拷贝产物，已 gitignore；改 em/ 结构后先 rm -rf client/apps/h5App/web/em（publishWeb 不清理旧文件）」；gitignore 实测 client/.gitignore:46 `apps/h5App/web/em/` 命中，dist/ 由 .gitignore:53 / admin-web/.gitignore:2 命中。
- 双落点后须追加第二落点清理命令（如 `rm -rf client/admin-web/dist/em` 或 `rm -rf client/admin-web/public/em`），并声明「publishWeb 不清理旧文件」对两落点都成立（Copy 只增不删，w12:53/:417）；选 C 还要补 .gitignore。另须同步 em/README.md:182-183「缺尾斜杠 → 二次挂载 Kuikly」改为「二次挂载 React SPA」（w12:409 等价风险）。

■ 2.2 构建顺序
- 现状两链无依赖：publishWeb → h5App/web（含 em）；vite build → admin-web/dist（不含 em）（w12:405）。
- 选 A/C：em 资产就绪（em/ 源目录或 publishWeb 产出的 web/em）→ `pnpm build`（A 在 build 后拷、C 在 build 内拷 public）→ dist 含 em。Gradle publishWeb 可完全不跑（旧端目录保留但不构建）。
- 选 B：pnpm build 先 → Gradle publishWeb 后（抄 em 进 dist）→ 之后不可再跑 vite build。
- Docker 现有串行屏障 (Dockerfile:260) 只约束 web(Gradle)→build(Rust)；新增 admin-web node stage 需重设 DAG。
- 本机坑 `env -u NODE_OPTIONS ./gradlew :apps:h5App:publishWeb`（gearui-kit-t13-acceptance.md:83/:9）**只要仍跑 publishWeb 就仍适用**（选 B 或保留旧端构建时）；若形态①完全不跑 publishWeb（改从 em/ 源目录直接拷），则该链路不再触发。Dockerfile:232 的显式 NODE_OPTIONS 勿与本机 env -u 互相套用。

■ 2.3 鉴权与相对路径
- client/admin-web/src/lib/api.ts:24 DEFAULT_API_BASE_URL="/api"（相对）；getApiBaseUrl (:41-47) 仅 VITE_API_BASE_URL 覆盖；requestApi (:112-138) fetch 相对路径 → ServeDir 托管 dist 时同源打到同一 8000，**无 CORS**（AGENTS.md:50/:58）。
- ?token=：getAuthToken (:78-109) 从 URL query 读入 sessionStorage，注入 `Authorization: Bearer` (:119-122)，**不拼回任何请求 URL** (:75、:19-21)；ServeFile (mod.rs:330) 返回 index.html 不改写 URL/query，故深链刷新后地址栏 ?token= 仍在、前端仍可读。注意 token 只注入 requestApi，WS/XHR/下载链接透传是后续收尾项 (api.ts:21)。
- getWebSocketUrl (api.ts:145-163)：同源固定 `ws(s)://<host>/api/admin/ws` (:159/:162)；绝对 baseURL 模式 http→ws 且 pathname 同样落 /api/admin/ws (:151-155)。**不依赖裸 /ws**。
- 服务端 auth_ok (mod.rs:377-396)：expected_token 空免鉴权 (:382-384)；非空要求 Authorization: Bearer 或 ?token= (:385-395)。iframe 官方页的 ?token=（emEmbed.ts:73-77/:14-18）是给 host.js 请求 /api/echomuse/* 的管理台令牌，与管理台 Bearer 同源但两套凭据勿混（AGENTS.md:175）。
- Tauri 生产壳 DEFAULT_SERVER_URL=127.0.0.1:8000 (lib.rs:28) 加载 server 页面 ⇒ **与 admin_dir 是同一份目录**，切 dist 后 iframe/API 都跟随该目录；XIAOZHI_ADMIN_URL (lib.rs:21) 只改服务器地址、不改 admin_dir；frontendDist="../dist" (tauri.conf.json:10) 内嵌副本只在 EMBEDDED=1 用且 API 不可达 (AGENTS.md:64)。
- HashRouter：深链是 /#/config 形态，hash 不发给服务端，刷新只请求 / → 必返回 dist/index.html；切换 admin_dir 不破坏深链刷新。裸 /config、/settings 不是有效深链（会返回 index.html 但按缺省 hash 渲染首页），验收脚本别用错。

■ 2.4 回滚路径
- 开关：`XIAOZHI_ADMIN_DIR=../client/apps/h5App/web`（main.rs:334-337）/ PUT /api/config 改 server.admin_dir（ServerSection.tsx:63-69）+ 重启（mod.rs:46）；Docker 改回镜像 ENV (Dockerfile:423) 或 compose environment 覆盖。
- 旧产物：h5App/web/ 现含 index.html/nativevue2.js/h5App.js/em/（ls 实测），client/.gitignore:41-46 只忽略 *.js/*.js.map 与 web/em/，index.html 入库。形态①「旧端目录保留」→ 本地回滚即刻可用；形态②「归档/删除」→ 须重跑 publishWeb 才能恢复（且 env -u NODE_OPTIONS）。
- **易漏断点**：dev-server.sh:11-15 硬检查 nativevue2.js/h5App.js，切到 dist 后走 :12-15 直接 exit 1「Custom XIAOZHI_ADMIN_DIR has missing assets」，本地开发链需改检查条件或新增启动方式。

■ 2.5 Docker / CI 改动面（切 dist 必须）
- 新增 node/pnpm stage：`FROM node:... AS admin-web` + COPY client/admin-web/{package.json, pnpm-lock.yaml, pnpm-workspace.yaml, tsconfig*.json, vite.config.ts, index.html, src/, public/} + `pnpm install --frozen-lockfile` + `pnpm build`（与 tauri.conf.json:8 beforeBuildCommand 同款）。dist/ 不入库 (2.4) ⇒ 必须自建，不能 COPY 现成产物。
- .dockerignore 复核：:33 `*.md` 排除 AGENTS.md 无妨；:36 `logo.png` 按 :5 语义可能连带排除 public/logo.png → dist 缺 favicon（dist/index.html:7 引用 /logo.png），需实测；:29 `scripts` 可能排除 client/admin-web/scripts（gen-icons.py 不在 build 链，无妨）；:51 `**/node_modules` 正确。
- Dockerfile:399 改为 `COPY --from=admin-web .../dist /app/web`（或新目录 + 同步 :423 ENV）；:235-240 的 test 校验（尤其 :238-240 em 三项）改为校验 dist/index.html + dist/em/index.html + dist/em/static/adapter.js；:260 屏障锚点（现读 web/index.html）随 stage 变化需调整；:398 注释同步。
- docker-compose.yml 无需改；CI docker-amd64.yml:144-149 build-args 不变，若 :69-93 静态守卫新增 grep 断言需同步。

（2/2 续报：验收矩阵 + 形态选项 + 需拍板清单 + 风险登记）

---

> **以下为「报告 2/2」原文，全量保留，未删减（章节标题按 §1–§6 规范化编号，正文一字未删）。**

【W15 生产构建链研究 2/2（验收矩阵 + 形态选项 + 需拍板清单 + 风险登记）】承接 1/2。假设验收基准：server 以切换后的 admin_dir（client/admin-web/dist，且已含 em/）在 127.0.0.1:8000 启动；BASE=http://127.0.0.1:8000。行号同 1/2 约定。只读研究。

## §3 验收矩阵事实（每项：怎么验 / 正常预期 / 失败信号）

| # | 路径 | 怎么验 | 正常预期 | 失败信号 |
|---|---|---|---|---|
| 1 | `/` | `curl -sS -i $BASE/` | 200、Content-Type text/html、body 为 dist/index.html：含 `<title>小智管理台</title>`、`/assets/index-*.js`、`<div id="root">`（client/admin-web/dist/index.html:7-13） | 仍含 `<title>XiaoZhi Admin</title>` + `./nativevue2.js?v=`/`./h5App.js?v=`（client/apps/h5App/web/index.html:6/:24-26）= admin_dir 没切；或 404 + 中文「管理页面未找到：请先构建 h5App（web）…」（server/src/gateway/ws/mod.rs:353-359）= 目录不存在 |
| 2 | `/config`、`/settings`（SPA 深链） | 浏览器地址栏输入 `$BASE/#/config`、`$BASE/#/settings` 后硬刷新（curl 只能验 `/`，深链需浏览器/Playwright）；另 `curl -sS -i $BASE/config` 看裸路径响应 | hash 深链：服务端只收到 `/` → 返回 dist/index.html → HashRouter 渲染对应页（src/App.tsx:2/:12、src/lib/nav.ts:6、AGENTS.md:53）；裸 `/config` 也返回 SPA index.html HTML 200，但前端按缺省 hash 渲染首页 | 404 / 白屏 / 返回官方 em/index.html；注意**裸 /config、/settings 不是有效深链**（切 dist 前后都返回 SPA index.html），验收脚本别用错路径 |
| 3 | `/api/config` | `curl -sS -i $BASE/api/config` | 200 JSON（配置对象，Content-Type application/json）；不得是 HTML | 首字符 `<` / Content-Type text/html = 落到 SPA fallback（mod.rs:331）；注意 pure-base 下能力段 key 缺失属正常（AGENTS.md:169/:170），不是切换故障 |
| 4 | `/api/admin/ws` | WS 升级握手（wscat / 浏览器 devtools Network WS） | 101 Switching Protocols + hello 帧（mod.rs:156 注册；api.ts:145-163 getWebSocketUrl 固定该路径） | 返回 HTML 200 = 路由表未注册或被 fallback 抢；dev 模式 /api/admin/ws 不在 blocked（dev.rs:97-105 仅列 /api/ws），但 dev guard 会加 `x-xiaozhi-mode: dev`（dev.rs:140-143） |
| 5 | `/em/`（**尾斜杠必须**） | `curl -sS -i $BASE/em/`（emEmbed.ts:9-10、em/README.md:182-183） | 200 + 官方 em/index.html：`<title>EchoMuse 设备管理</title>`（client/apps/h5App/web/em/index.html:8）、`<script src="static/adapter.js">`（:587）、`static/dashboard.js`（:588）、`static/host.js`（:591）、`<meta name="referrer" content="no-referrer">`（:7）、CDN SRI 引用（:577-583） | `<title>小智管理台</title>` + `/assets/index-*.js`（dist/index.html:8-9）= **落 SPA fallback 的核心症状**（w12-devicehub-research.md:409）；或 404 中文 admin_missing |
| 6 | `/em/status` | `curl -sS -i $BASE/em/status` | 分两种：①echomuse-plugin 编入 → 200 JSON `{"enabled":bool,"devices":[{...}]}`，且 `enabled=false` 仍 200（server/src/device/echomuse/routes.rs:68/:71-75，无鉴权）；②pure-base / `--no-default-features`（dev-server.sh:31）→ **HTML 200**（fallback），前端归一 EchomuseUnavailableError(not-compiled)（client/admin-web/src/lib/echomuseApi.ts:277/:290-293） | 返回官方 em/index.html（说明 dist/em/status 恰好有同名静态文件，不应有）；或 JSON 但缺 enabled/devices → 前端「响应形状不符」（echomuseApi.ts:297-299）。**该端点与 /em/ 静态目录同前缀但性质不同，验收须按 features 分情况**（w12:124/:306/:411） |
| 7 | iframe 内 adapter.js 引用 | `curl -sS -i $BASE/em/static/adapter.js`；再在 devicehub 页实测 iframe（AGENTS.md:175、emEmbed.ts:73-77） | 200 + Content-Type application/javascript + body 以 JS 文本开头（adapter.js 29400 B，ls 实测）；iframe 内 adapter.js/dashboard.js/host.js 三个 200，官方 EchoMuse UI 正常挂载 | 返回 SPA index.html（HTML 200）→ 浏览器 `<script src="static/adapter.js">` 解析 HTML 报 SyntaxError；或 404；iframe 内出现 React 管理台 UI = 二次挂载；CDN 失败白屏应有 EM_EMBED_CDN_NOTICE 诚实文案（emEmbed.ts:47-49），不得伪装功能故障 |
| 8 | 静态资源缓存 | `curl -sS -I $BASE/assets/index-BCpFm4NW.js` 与 `curl -sS -I $BASE/` 对比 Cache-Control/ETag/Last-Modified；改前端后重验 | assets 为内容哈希文件名（dist/assets/index-BCpFm4NW.js，ls 实测）可长缓存；index.html 每次取最新、不强缓存（含新 hash 引用） | 浏览器拿到旧 index.html 引用已不存在的 assets → 404；或 index.html 仍带 `?v=` 时间戳（那是旧端 publishWeb 机制 build.gradle.kts:110-121，切 dist 后改用 Vite 内容哈希，缓存策略须重新核对）。注意 ServeDir 只有 `precompressed_gzip()`、**无显式 Cache-Control**（mod.rs:328-330），以实际响应头为准 |

■ 特别：「/em/ 返回官方 index.html 而非 SPA index.html」可机器验证断言（shell，正断言 + 负断言 + 配套资源断言）
```bash
BASE=http://127.0.0.1:8000
BODY="$(curl -fsS "$BASE/em/")" || { echo "FAIL: /em/ 非 2xx"; exit 1; }
grep -q '<title>EchoMuse 设备管理</title>' <<<"$BODY" \
  && grep -q 'static/adapter.js'             <<<"$BODY" \
  && grep -q 'static/dashboard.js'           <<<"$BODY" \
  && grep -q 'static/host.js'                <<<"$BODY" \
  && ! grep -q '<title>小智管理台</title>'   <<<"$BODY" \
  && ! grep -q '/assets/index-'              <<<"$BODY" \
  && ! grep -q '<div id="root">'             <<<"$BODY" \
  && echo "PASS: /em/ 是官方页" || echo "FAIL: /em/ 落到了 SPA index.html"
# 配套：三个相对脚本必须真是 JS，不是 HTML
for f in adapter.js dashboard.js host.js; do
  FIRST="$(curl -fsS "$BASE/em/static/$f" | head -c 60)"
  case "$FIRST" in '<'*|'<!doctype'*) echo "FAIL: $f 返回了 HTML";; *) echo "OK: $f";; esac
done
```
断言依据：官方页特征 = client/apps/h5App/web/em/index.html:8/:587-591；SPA 特征 = client/admin-web/dist/index.html:8-13；回退机制 = mod.rs:328-331（ServeDir.not_found_service(ServeFile(index.html))）。此断言同时是「em 拷贝是否真的落进 dist」的唯一硬判据（w12:416「生产 admin_dir 下直接请求 /em/ 必须返回官方 index.html，不得返回 SPA 的 index.html」）。

## §4 形态选项（不替下结论，逐项改动面与风险）

【① 只切 admin_dir 指向 dist + 新增 em 拷贝（旧端目录保留）】
- 改动面：
  · em 落点实现 A/B/C 三选一（1/2 §2.1）：A 改 vite.config.ts 或 package.json:8 + 新脚本；B 改 client/apps/h5App/build.gradle.kts:98-104/:109；C 在 client/admin-web/public/em/ 放筛选产物（+ .gitignore）。
  · admin_dir 默认值：server/src/config.rs:585-587、server/config.example.toml:21、dev-server.sh:10（及 :11-15 检查条件）、Dockerfile:398 注释/:399/:423；可选只靠 XIAOZHI_ADMIN_DIR（main.rs:334-337）。
  · Docker：新增 admin-web node stage、改 :399 COPY/:423 ENV/:235-240 校验/:260 屏障锚点；.dockerignore :33/:36/:29 复核（1/2 §2.5）。
  · 文档：em/README.md:182-185（双落点清理 + Kuikly→React 口径）、client/shared/.../AGENTS.md:143/:146、client/AGENTS.md:17、client/admin-web/AGENTS.md:225/:50、README.md:97。
- 风险：emptyOutDir 与手工拷贝的顺序耦合（A）、双链顺序不可逆（B）、public/em 入库（C）；dev-server.sh:11-15 本地断点；dist 不入库必须 Docker 自建；已有部署的 SQLite 库内 admin_dir 旧值不会因改代码默认值而变（main.rs:322-331，见风险 R7）。
- 回滚：`XIAOZHI_ADMIN_DIR=../client/apps/h5App/web` 即刻切回（本地旧产物仍在，1/2 §2.4）。

【② 切 dist + 归档/删除旧端（与另一路资产盘点衔接）】
- 在①基础上追加：归档/删除 client/apps/h5App/web/、client/shared 管理台源码、相关宿主文件；**必须先决定 client/apps/h5App/em/ 源目录归属**（build.gradle.kts:65）——若随旧端一起删，/em/ 资产失去唯一来源与 verifyEmBundle 闸门（:72-96），需把 em 源迁到 admin-web 侧并重建拷贝链。
- Docker 可整体移除 web stage：连带 Dockerfile:138、:217-240 两条 Gradle RUN、:29-73 头部 OOM 记录、:260 屏障、:235-240 em 校验；CI docker-amd64.yml:144-149 的 GRADLE_XMX/KOTLIN_DAEMON_XMX/NODE_MAX_OLD_SPACE/GRADLE_MAX_WORKERS 失效；.dockerignore:41-68 大段（非 web 宿主端 / h5App web 产物 / Gradle 构建脚本）失效。
- 风险：回滚须重跑 publishWeb 才能恢复旧端产物（且 env -u NODE_OPTIONS，gearui-kit-t13-acceptance.md:83）；删 Gradle 链前要确认没有其它依赖（:shared:verifyAdminIcons 等四端构建指引 client/AGENTS.md:13-23）；em 源与闸门归属未定就删 = /em/ 断供。
- 衔接点：与另一路「旧端 Kotlin 源码资产盘点」对齐——哪些 em/ 宿主文件（w12:380-385 列的清单）必须保留、哪些可归档。

【③ 维持 h5App/web 作默认、dist 仅 Tauri 内嵌】
- 改动面：几乎零（保持现状）；只需澄清文档口径冲突：admin-web/AGENTS.md:56-64「生产默认加载服务器页面（方案 B）」+ :50「XIAOZHI_ADMIN_DIR 可指向 dist/」与实际 config.rs:585-587 默认 h5App/web 不符。若想让 Tauri 用 admin-web UI 只能 XIAOZHI_ADMIN_EMBEDDED=1 内嵌 dist，但那时 /api 相对路径打到 tauri://localhost 资产协议、API 不可达（lib.rs:26-27、api.ts:11-13、AGENTS.md:64）——需另解（remote.urls / VITE_API_BASE_URL 调试手段）。
- 优势：/em/ 同源可达（h5App/web/em 存在，ls 实测），无 emptyOutDir/双链/ Docker 新增 stage 任何改动。
- 风险：admin-web 全量迁移却不上生产，W15 目标（AGENTS.md:225）落空；方案 B 表述长期失真；Tauri 生产壳看到的仍是旧端 Kuikly 页。

## §5 需拍板清单
1. em 落点选 A / B / C（决定改 vite.config.ts、package.json、build.gradle.kts、public 中哪些，以及构建顺序）——建议优先评估 C（唯一与 emptyOutDir 天然兼容），但需接受 public/em 的 gitignore/同步成本。
2. admin_dir 默认值是否改代码（config.rs:585-587 + config.example.toml:21）还是只靠 XIAOZHI_ADMIN_DIR / Dockerfile ENV 覆盖：**注意已有部署的 SQLite 库内值优先于代码默认值**（main.rs:322-331 load_config_inner 从库读；env 才最后覆盖 :334-337），仅改代码默认值对存量实例无效。
3. Docker 是否新增 admin-web node stage（dist 不入库，必须自建）；Gradle web stage 是否保留（只为产 em，还是完全移除改由 em/ 源目录直接拷）。
4. 旧端 h5App/web 是否归档/删除（形态① vs ②），与另一路资产盘点结果对齐；em/ 源目录归属先定。
5. 文档口径同步范围：em/README.md:182-185 双落点清理、:182-183「二次初始化 Kuikly」→「二次挂载 React」、各 AGENTS.md 的 admin_dir 默认值、AGENTS.md:225 W15 表述、README.md:97。
6. dev-server.sh:11-15 三件套检查如何处理（改检查条件 / 保留旧端链 / 新增 --admin-web 启动模式）。
7. 缓存策略：Vite 内容哈希 assets 与 index.html 的 Cache-Control（ServeDir 无显式设置，mod.rs:328-330）；旧端 `?v=` 时间戳机制（build.gradle.kts:110-121）作废后的失效口径。
8. /em/status 在 pure-base 下 HTML 200 的诚实文案是否随 admin_dir 切换调整（w12:411：切 dist 后 /em/ 与 /api/echomuse/* 都落同一 SPA HTML 200，前端难区分「能力未启用」与「加载了一个 SPA」）。

## §6 风险登记
- R1 `emptyOutDir:true`（vite.config.ts:46）清空手工拷入的 dist/em（w12:410）→ 拷贝顺序强耦合。
- R2 选 B 时 pnpm build 与 publishWeb 先后不可逆（否则 em 被清空）；Docker 双 stage 传递 dist 需新 DAG。
- R3 /em/ 无 em 目录 → SPA fallback HTML 200 → iframe 内二次挂载 React（w12:409、em/README.md:182-183）；与 pure-base 的 /api/echomuse/* HTML 200 叠加使前端诚实文案失真（w12:411）。
- R4 dist/ 与 web/em/ 均被 gitignore（.gitignore:53、client/.gitignore:44-46；git check-ignore 实测），Docker 必须自建 dist，不能 COPY 现成产物。
- R5 .dockerignore:36 `logo.png` 按 :5「不含 / 匹配任意层级」语义可能连带排除 client/admin-web/public/logo.png → dist 缺 favicon（dist/index.html:7 引用 /logo.png），需实测。
- R6 dev-server.sh:11-15 检查 nativevue2.js/h5App.js，切 dist 后 :12-15 exit 1 → 本地开发链断点。
- R7 admin_dir 是配置库可写字段（plugins/registry/projection.rs:114、ServerSection.tsx:63-69）；改代码默认值不影响存量实例已持久化的库内值；env 覆盖虽优先但不写回（main.rs:332-337），GET /api/config 展示值与运行值会不一致。
- R8 裸 /config、/settings 与 HashRouter 语义（App.tsx:12、nav.ts:6），验收脚本别用错路径；SPA 深链刷新本身不依赖服务端 history fallback。
- R9 /em/status 是 echomuse-plugin 门控 API 路由（routes.rs:68）且与 /em/ 静态目录同前缀，pure-base 下 HTML 200（w12:124/:306），验收须按 features 分情况，勿与 /em/ 静态页混判。
- R10 Tauri 生产壳加载 http://127.0.0.1:8000（lib.rs:28）= server 的 admin_dir，切 dist 后 iframe/API 都跟随该目录；EMBEDDED=1 内嵌 dist 时 API 不可达（AGENTS.md:64）；XIAOZHI_ADMIN_URL 只改地址不改 admin_dir。
- R11 本机 publishWeb 需 `env -u NODE_OPTIONS`（gearui-kit-t13-acceptance.md:9/:83）与 Dockerfile:232 显式 `NODE_OPTIONS="--max-old-space-size=..."` 语境不同，勿混用；只要还跑 publishWeb 本机坑仍适用。
- R12 形态②若删除旧端，em 源目录（build.gradle.kts:65）与 verifyEmBundle 闸门（:72-96）归属须先定，否则 /em/ 失去一致性校验；syncEmAssets exclude 口径（:102，tests/ 未排除、w12:54）在换落点时须原样保留或显式收窄。

—— 1/2 + 2/2 报告完毕。研究全程只读：未创建/修改任何文件、未执行构建、未做任何 git 写操作（仅 git ls-files / check-ignore / cat 等只读查询）。上文所有目录构成、产物字节数均为 ls 实测；ServeDir 对 /em/ 目录追加 index.html、.dockerignore 任意层级 logo.png 匹配、Vite 8 public 拷贝发生在 emptyOutDir 之后三处，是基于现有产物证据与文档口径的推断，均已标注「需实测」，请在实施前用真实构建/运行确认。

---

> **以下为「勘误补充」原文，全量保留，未删减。**

【W15 生产构建链报告 — 勘误补充（接 1/2 §1.1、2/2 需拍板/风险登记）】与 w6-researcher #54 对齐时发现我 1/2 报告一处目录构成偏差，已只读实测核实，请合并两路报告时以本条为准。研究仍全程只读。

■ 勘误 1：1/2 §1.1「em 源目录构成」按 13 文件口径
- 我 1/2 原文写的是「client/apps/h5App/em/（源，入库）：LICENSE、README.md、README.sha256、index.html(28417)、static/{adapter.js 29400, dashboard.js 440653, host.js 12937}、tests/ 5 cjs」——这段**把下发产物 web/em/static/ 的 3 文件与字节数误当成源目录构成，漏了 static/dashboard.jsx，隐含 12 文件口径**。
- 实测（`git -C ROOT ls-files client/apps/h5App/em/ | wc -l` = **13**，13 个文件全部入库；与 docs/plans/admin-web-migration/w12-devicehub-research.md:380 基线 4+4+5=13 一致）：
  · 根 4：LICENSE(1066)、README.md(12059)、README.sha256(170)、index.html(28417)
  · static/ **4**：adapter.js(29400)、dashboard.js(440653)、**dashboard.jsx(648322，源里最大文件)**、host.js(12937)
  · tests/ 5 个 .cjs：console.browser.cjs(6531)、host.test.cjs(11600)、native-console.browser.cjs(11027)、native-management.browser.cjs(10678)、native-volume.browser.cjs(7609)
- 下发产物 client/apps/h5App/web/em/：static/ 仅 3 个（dashboard.jsx 被 build.gradle.kts:102 exclude 排除）、tests/ 5 个（字节与源略有差异，是稍旧拷贝，w12:53 已记）。
- 无需改的部分：1/2 的 verifyEmBundle 段（build.gradle.kts:79-86 校验 dashboard.js 首行 src-sha256 == dashboard.jsx 摘要、:90-92 校验 static/{adapter.js,host.js,dashboard.jsx} 三文件存在、:94 LICENSE）与 :102 exclude 口径本身正确。仅「em/ 源目录构成」一段按 13 口径更正。

■ 勘误 2（新增事实）：/em/ 下发面含 tests/ 5 个 .cjs，换落点时可能扩散到 dist
- syncEmAssets (client/apps/h5App/build.gradle.kts:99-104) 的 exclude 仅 README.md / README.sha256 / LICENSE / static/dashboard.jsx（:102），**不含 tests/** ⇒ 5 个 .cjs 随 /em/ 下发（web/em/tests/ 实测 5 个，w12:54 同样记录）。
- 这 5 个是 Node 测试脚本，**不是浏览器运行时所需**：官方页只引用 static/{adapter.js,dashboard.js,host.js}（client/apps/h5App/web/em/index.html:587-591）与 CDN 库（:577-583）。即运行时真正下发的 4 文件（index.html + static 三 js）之外，tests/ 是冗余面（合计约 47KB）。
- 若选 1/2 §2.1 的 A/C 路径把 em 拷进 dist、或 B 的第二落点，且沿用同一 exclude 口径，这 5 个 .cjs 会一并进入 dist/em/tests/，生产静态目录多约 47KB 非运行时文件、且 `/em/tests/*.cjs` 可被同源请求到（信息面）。这同时印证我 1/2 §2.1 已提的「不能直接拷 em/ 源目录（会带 jsx 源码与仓库文档）」——dashboard.jsx(648KB) 与 tests/ 都属此类。
- 收口选项（归构建链，不替下结论，请并入 2/2 需拍板清单第 1 项）：①在 build.gradle.kts:102 的 exclude 增加 `tests/`，且 A/C 路径的拷贝脚本须同步（源目录直接拷仍会带）；②A/C 路径改为**只拷白名单 4 文件**（index.html + static 三 js），彻底绕开 em/ 源目录其余文件，同时解决 jsx/文档混入；③维持现状，登记为已知冗余面。
- 请一并补进 2/2 风险登记：R13「em 下发面含 tests/ 5 个 .cjs 与源 dashboard.jsx；换落点时若无白名单会扩散到 dist/em，并暴露 `/em/tests/*.cjs`」。

■ 与 w6-researcher #54 口径一致性（供合并拍板）
- em/ 13 文件全部入库、是 /em/ 产物唯一来源（build.gradle.kts:65/:98-104）且带 verifyEmBundle 闸门（:72-96），**不能随旧端 web 产物一起删**；对方 1/2 已单列、2/2 需拍板第 2 项已列「h5App/em/ 是否随旧端下线」，与我构建链侧「必须保留或迁移到 admin-web 侧」（1/2 §2.4、2/2 形态②）独立同源、无冲突。
- 其余四点（w12:380 基线、git 入库边界、部署面失效清单归我分路、NODE_OPTIONS 本机坑）双方已逐条核过一致；对方补的细化「web/index.html 入库但会被 publishWeb doLast 写回 ?v= 时间戳（build.gradle.kts:110-121），归档/回滚按此理解」与我 1/2 §1.1 一致。

1/2 + 2/2 + 本勘误为完整交付。未创建/修改任何文件、未构建、未 git 写操作（仅 git ls-files / ls 只读查询）。

---

> **以下为「W15-1 实施勘误补充」（2026-11-12，software-engineer 于 W15-1 实施中发现；本归档作者逐条只读复核）**，追加于归档之后。上方 §1–§6 与「勘误补充」三段研究原文**仍按零删减保留**；如与上方口径冲突，**以本勘误与 `tools/admin_web_switch_check.sh` 为准**。做法对齐同目录 `w12-devicehub-research.md:397-419` 的文末补充勘误惯例。

## §3 可机器验证断言的事实勘误（影响验收可执行性）

- **错误**：报告 2/2 §3「`/em/` 返回官方 index.html 而非 SPA index.html」shell 断言中的 `&& ! grep -q '<div id="root">'`（本归档 :183）。
- **事实**：官方页 `client/apps/h5App/em/index.html:571` 自身就是 `<div id="root"></div>`（React UMD 挂载点），**不是 SPA 独占特征**。正确部署下 `/em/` 返回官方页，该负断言必然命中 → 整条断言链恒 FAIL。
- **只读复核**（本归档作者）：
  · `client/apps/h5App/em/index.html`：:8 `<title>EchoMuse 设备管理</title>`、:571 `<div id="root"></div>`、:587 `static/adapter.js`、:588 `static/dashboard.js`、:591 `static/host.js`；全文**无** `<title>小智管理台</title>`、**无** `/assets/index-`、**无** `<script type="module"`。
  · `client/admin-web/dist/index.html`（W15-1 重新构建后）：:8 `<title>小智管理台</title>`、:9 `<script type="module" crossorigin src="/assets/index-CvrNL8PP.js">`、:10 `/assets/index-ae9fZm_2.css`、:13 `<div id="root"></div>`。
  · ⇒ 三特征 `<title>小智管理台</title>` / `/assets/index-` / `<script type="module"` 才是 SPA 独占，可作 `/em/` 负断言；`<div id="root">` 两端都有，只能用于 `/` 的**正**断言（即 §3 表格第 1 项）。
- **实施采用的正确断言**（`tools/admin_web_switch_check.sh`，实为可执行权威）：
  · 正断言（:87-90）：`<title>EchoMuse 设备管理</title>` + `static/adapter.js` + `static/dashboard.js` + `static/host.js` 四个 `grep -qF`。
  · 负断言（:96-98）：`grep -qF '<title>小智管理台</title>' || grep -qF '/assets/index-' || grep -qF '<script type="module"'` → 命中任一即 FAIL「`/em/` 落到了 SPA index.html（ServeDir fallback / dist/em 缺失）」，否则 PASS（:101）。
  · `/` 正断言（:62-64）仍用 `<div id="root">`，那是正确的；配套三份 JS 首字节非 HTML 检查（:108-121）、`/api/admin/ws` 101 检查（:148）、汇总退出码（:159-165）。
  · 脚本头注释已记录该勘误原因（:12-14）。**实施与 QA 一律以脚本为准，勿照抄 §3 原文的负断言。**

## 头部「推断项待实测」三处状态更新（W15-1 真实 `pnpm build` 实测）

| # | 推断 | 状态 |
|---|---|---|
| ① | Vite 8 `public` 拷贝发生在 `emptyOutDir:true` 清空 `outDir` 之后 | **已证实**：真实 `pnpm build` 后 `client/admin-web/dist/em/` 每次构建都在，恰含 4 文件（本归档作者 `ls` 复核：`dist/em/index.html` 28417、`static/adapter.js` 29400、`static/dashboard.js` 440653、`static/host.js` 12937；无 `tests/`、无 `dashboard.jsx`），与 C 方案白名单 4 文件裁定一致。`tools/admin_web_switch_check.sh:6` 注明 build 内含 `scripts/sync-em.mjs`。 |
| ② | `ServeDir` 对 `/em/` 目录追加 `index.html` | **仍待验**：需真 server 才能验，本机无真 server（software-engineer 已标待验），保持待验。 |
| ③ | `.dockerignore:36` `logo.png` 按 `:5` 语义匹配任意层级同名文件 | **仍待实测**：未变。 |

另：W15-1 已重新构建 `dist/`（文件时间 2026-11-12 10:15），Vite 产物 hash 由研究时点的 `index-BCpFm4NW.js` 变为 `index-CvrNL8PP.js`（CSS 仍 `index-ae9fZm_2.css`）；§1.2 原文中的 hash 与字节数按**研究时点实测**保留，不回改。

> 本勘误补充为**追加内容，未删改上方任何研究原文**；本次仅按 team-lead 原授权维护该单一归档文件，未构建、未做 git 写操作、未修改其他任何文件。
