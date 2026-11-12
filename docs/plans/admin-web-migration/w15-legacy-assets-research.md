# W15 / 旧端资产盘点与下线影响面研究 · 权威归档

> **文档性质**：W15 / 旧端下线前置研究「资产盘点与下线影响面」分路的**唯一权威契约**，供 W15-1 实施（归档范围、引用清理、文档口径）与 QA 回归逐节核对。由研究阶段发送给 team-lead 的「报告 1/2 + 报告 2/2 + 补充勘误」及与 `w15-build-researcher` 的协作勘误消息**原文全量合并**而成，研究原文一字未删。
> **研究纪律**：研究全程只读——未创建/修改任何源码文件、未执行构建、未做 git 写操作（仅 `git ls-files` / `git check-ignore` 只读查询）；本归档是经 team-lead 授权写入的单一文档。
> **证据口径**：全部结论带 `file:line`；根路径 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust/`，报告内 `xxx:行` 均相对该仓库根。目录构成、文件数、字节数、入库/忽略状态均为 `ls` / `Glob` / `find` / `git ls-files` / `git check-ignore` 实测。
> **分工边界**：生产构建链改造（Docker web 阶段 / `admin-web/dist` 切换 / em 落点实现）由 `w15-build-researcher` 分路负责，其归档见同目录 `w15-production-build-research.md`；本文按分工只登记引用事实，不深入构建链细节。
> **结论（先看）**：`client/shared/src/` 下 100% 是 `com.xiaozhi.admin` 包，整个 `shared` 模块即管理台；但 `contracts/` 与纯模型被 `adminCoreTests` / `bleProtocolTests` 源码级复用，删前须先确认这层共享引用面（§6）。`client/apps/h5App/em/` 与 Kuikly 管理台是两个独立文档、W12 已接入 admin-web devicehub iframe，**明确不下线**。

## 已拍板结论（用户经 team-lead 确认，2026-11-12）

| 编号 | 拍板项 | 结论 |
|---|---|---|
| 1（形态） | 旧端下线形态 | **② 软下线归档**：切到 `client/admin-web/dist` 并归档/删除旧端目录；不是形态①「彻底删除」、也不是形态③「并行灰度」。与研究阶段倾向一致（回滚成本最低、与「真机验收待验」不冲突、贴合 `client/admin-web/AGENTS.md:225` 既有「归档」用词） |
| 2（em 落点） | em 资产落点 | **C publicDir**；构建链侧实施口径（白名单 4 文件）见 `w15-production-build-research.md` |
| 3（切换范围） | 生产切换范围 | **生产全量切换**：`[server].admin_dir` / `XIAOZHI_ADMIN_DIR` / Docker 镜像静态目录全量指向 `client/admin-web/dist` |
| 4（文档口径） | 47 个 md 文档 | **全量改**（口径同步范围由用户拍板全量、不裁剪；分组清单见报告 2/2 §7.2） |

> **em/ 源目录明确不下线，13 文件全部保留**：`client/apps/h5App/em/` 是 `/em/` 官方设备管理页的**唯一来源**（`client/apps/h5App/build.gradle.kts:65` emDir、`:98-104` syncEmAssets、`:72-95` verifyEmBundle），与 Kuikly 管理台（`client/shared`）是两个独立文档、W12 已接入 admin-web devicehub iframe，**不随旧端 Kotlin 页面一起划入可删集合**。形态②归档下该源目录 13 个文件全部保留（`git ls-files client/apps/h5App/em/ | wc -l` = 13：根 4 `LICENSE` / `README.md` / `README.sha256` / `index.html` + `static/` 4 `adapter.js` / `dashboard.js` / `dashboard.jsx` / `host.js` + `tests/` 5 个 `.cjs`）。**注意区分两路口径**：本归档的「源目录全保留」是资产盘点口径；「生产下发只拷白名单 4 文件」是构建链侧裁定（`w15-production-build-research.md`），二者不冲突。

## 勘误与口径说明（本归档以最新为准）

| 项 | 原文口径（零删减保留） | 最新口径（实施 / QA 一律按此读） |
|---|---|---|
| `client/apps/h5App/em/` 源目录文件数 | 报告 1/2 §5 写「12 文件全部被 git 跟踪」 | **13 文件**：根 4 + `static/` 4（含 `dashboard.jsx`）+ `tests/` 5，全部入库；`w12-devicehub-research.md:380` 基线亦为 13。原文「12」为计数偏差，更正来源见文末「补充勘误」 |
| `client/apps/h5App/web/` 静态产物文件数 | 报告 1/2 §5「12 文件逐字节」 | **仍为 12 文件，无需更正**（`find client/apps/h5App/web -type f | wc -l` = 12）；与上一行的 `em/` 源目录是两个不同目录，**不要混淆** |
| `web/index.html` 可恢复性 | 报告 1/2 §5 / §9：「仅 `index.html` 被 git 跟踪，可恢复」 | 入库属实，但 `publishWeb` 的 `doLast` 会写回 `?v=` 时间戳（`client/apps/h5App/build.gradle.kts:110-121`），属「**入库但会被构建改动**」的文件；归档 / 回滚按此理解 |
| `em/` 浏览器运行时最小集 | — | `index.html` + `static/` 三 js 共 **4 文件**（`em/index.html:587-591` 只引 `static/adapter.js` / `dashboard.js` / `host.js`，第三方库走 CDN 带 SRI `:583-584`）；其余 9 个是构建源 / 许可 / Node 测试，非运行时所需 |

> 下方报告原文按「零删减」保留；如遇口径冲突，以**本次落盘前逐行核读的源码 `file:line`** 与文末「补充勘误」「协作勘误闭环」为准。报告 2/2 §8 三形态事实表、§10 九项拍板清单均为研究阶段「只给事实、不替下结论」的原文，拍板后按上表裁减即可。

## 目录与合并顺序

| 区段 | 内容 | 来源 |
|---|---|---|
| 报告 1/2 原文（§0–§6） | §0 一句话结论、§1 `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/` 153 个 `.kt` 分层、§2 其他源集（macosArm64Main / jsMain / androidMain / iosMain / commonTest）、§3 `client/gearuiKit/`（commonMain 256 + 4 平台各 7 actual）、§4 `client/apps/macosApp/`（2336 行自有 ObjC/C + Pods）、§5 `client/apps/h5App/web/` 12 文件构成与 gitignore、`em/` 隔离与 wasm 澄清、§6 三个测试模块与 4 宿主的共享引用面 | 报告 1/2 原文 |
| 报告 2/2 原文（§7–§10） | §7 引用清理清单（7.1 代码/构建脚本、7.2 全仓 47 个 md 分组、7.3 部署/CI 绑定）、§8 三形态事实表（①彻底删除 / ②软下线归档 / ③并行灰度）、§9 诚实化 / 风险登记与 git 可恢复性、§10 九项拍板清单、收尾声明 | 报告 2/2 原文 |
| 补充勘误 | #54 收尾对齐：`em/` 13 文件更正、`web/index.html` `?v=` 时间戳、两路结论一致性、边界声明、9 项待拍板清单 | 发给 team-lead 的消息原文 |
| 协作勘误闭环 | 与 `w15-build-researcher` 的两条衔接确认（em 归属、git 边界、13 口径、`tests/` 下发、运行时最小集佐证） | 发给 `w15-build-researcher` 的消息原文 |

---

---

> **以下为「报告 1/2」原文，全量保留，未删减。**

---

W15 / 旧端下线前置研究 A **报告 1/2（旧端资产全量盘点）**。2/2 接续：引用清理清单 + 三形态事实 + 诚实化/风险登记 + 需拍板清单。

路径基准：以下 `xxx:行` 均相对仓库根 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust`。全程只读（未改文件/未构建/未 git 写操作）。分工声明：生产构建链 / dist 切换 / em 资产落点由 `w15-build-researcher` 负责，本报告只在「引用清理清单」里点到 Docker/CI 的引用事实，不深入构建链。

## 0. 一句话结论（先给答案）

**`client/shared/src/` 下 100% 是 `com.xiaozhi.admin` 包——整个 `shared` 模块即管理台，没有任何设备端 UI 混在里面**（`find client/shared/src -name '*.kt' | grep -v '/com/xiaozhi/admin/'` 零结果）。但「管理台专属」不等于「可整目录删除」：① 纯逻辑/契约层被同仓 **3 个独立模块 + 4 个原生宿主**复用；② `client/apps/h5App/em/` 是 **EchoMuse 上游 vendor 的设备管理页、与 Kuikly 管理台是两个独立文档**（`client/apps/h5App/em/README.md:5-6`），且 server 在 `/em/` 静态托管、W12 已接入 admin-web 的 devicehub iframe——**它不是管理台资产，下线时绝不能随 `h5App/web` 一起误删**；③ `client/apps/h5App/web/` 12 文件里**只有 `index.html` 被 git 跟踪**，其余是 gitignore 的构建产物。

## 1. `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/` 全量（153 个 .kt）

**定量**（`find` 实测）：commonMain 共 **153** 个 `.kt`，全部落在 `admin/` 包下；分层为 root 129 + `contracts/` 5 + `kit/` 16 + `icons/` 2 + `state/` 1。子目录：`contracts/`、`kit/`、`state/`、`icons/`。

**入口（全仓仅两个 `@Page` 注解）**：
- `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AdminPage.kt:65` `@Page("admin")`——跨端统一管理台根页面；`:27` 逐字「管理后台根页面（`page_name=admin`）——**跨端统一的多页管理台**（T06 起是 Compose 壳）」；`:31-37` 列 13 个 `XxxState(this)` 与 `didInit → setContent → App(GearUI) → AdminShell` 组合根。
- `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigPage.kt:63` `@Page("config")`——`:39` 逐字「管理后台 · 配置页（`page_name=config`），位于 shared 公共层，跨端复用（web / Android / iOS / macOS）」；`:41` 逐字「统一管理台（`@Page("admin")`）之外的单页模式兜底入口：其它平台没有侧边栏壳」。
- `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md:5` 逐字「**单一壳**：web 与 macOS 共用 `@Page("admin")`…全仓只注册 `admin` 与 `config` 两个页面——KSP 生成的 `KuiklyCoreEntry` 锁定，**⛔ 不新增 `@Page`**（旧 `router` 双壳与 Compose 试点 `admin-compose` 均已删除）」。KSP 声明见 `client/shared/build.gradle.kts:13-14`「Kuikly KSP：扫描 @Page 注解生成页面注册代码（KuiklyCoreEntry，按编译目标注入）」。

**一级导航（口径需登记，team-lead 背景说「8 个」，实际有三套数）**：
- `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/contracts/AdminNav.kt:36-64` `AdminNav.all` 实为 **9 项**：overview(0) / testbench(5, `macOnly=true`) / devices(10) / devicehub(15) / console(47) / config(50) / settings(55) / plugins(57) / files(60)；`:67` `DEFAULT = "config"`；`:88-92` `visible(macos)` 过滤 `macOnly` → **web 端 8 项**。
- `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md:13` 逐字「[AdminNav]（一级导航常量表，**9 项（web 8 项，`testbench` 仅 macOS）**…）」；但同文件 `:5` 又写「一级导航 **10 项 + 动态插件组**」——**同文档内部口径不一致（9 / 10）**，下线后这些文档都会变过期口径。
- `client/admin-web/AGENTS.md:225` 另有「按 `src/lib/nav.ts` 的 **10 个描述符**逐页迁移…**全部 10 项迁移完成**」——那是 admin-web 侧 nav（含 echomuse 占位），与旧端 AdminNav 的 9 项不是同一张表。

**其余 129 个 root 文件的归类（管理台专属）**：Config 域（`ConfigFormState.kt` / `ConfigFormDraft.kt` / `ConfigFormFill.kt` / `ConfigFormActions.kt` / `ConfigFormCatalogue.kt` / `ConfigSections.kt` / `ConfigDraftChanges.kt` / `ConfigContextJson.kt` / `ConfigNluPlanJson.kt` / `ConfigModeDraftPolicy.kt` 及各 `ConfigXxxState/Card`）、工作流域（`PipelineFlow.kt` / `PipelinePanels.kt` / `PipelineWorkflow.kt` / `PipelineWorkflowView.kt` / `WorkflowEditorModel.kt` / `WorkflowEditorState.kt` / `ConfigWorkspaceViews.kt`）、EchoMuse 域（`EchoMuse*.kt` 约 40 个，含 `EchoMuseBleCodec.kt` / `EchoMuseOfficialView.kt` / `EchoMuseConsole*.kt` / `EchoMuseNative*.kt`）、页面/卡片（`AdminShell.kt` / `ConsolePage.kt` / `PluginsManagePage.kt` / `PluginsManageDetailDialog.kt` / `TuyaDetailPage.kt` / `TuyaDeviceDialog.kt` / `McpToolsCard.kt` / `EngineHealthCard.kt` / `NodeTestPanel.kt` 等）、状态与协议（`AdminRealtime.kt` / `ConsoleState.kt` / `FilesState.kt` / `FilesModel.kt` / `OtaState.kt` / `NodeTestsState.kt` / `NodeTestJson.kt` / `NodeTestModels.kt` / `FormSchemaState.kt` / `EngineHealthState.kt` / `McpToolsState.kt` / `SecretMask.kt` / `TimeFormat.kt` / `ToastHost.kt` / `AdminAccess.kt` / `AdminInterfaceMode*.kt` / `MacServerSwitch.kt`）。**全部是管理台 UI/状态/编排代码，无设备端 UI**。

**子目录**：
- `contracts/` 5：`AdminNav.kt`、`FieldHelpState.kt`、`PluginSlots.kt`、`PluginUi.kt`、`UiContracts.kt`——`AGENTS.md:13` 逐字「**契约层在 contracts/**（零 Kuikly 依赖）」。**注意：这是唯一被外部测试模块直接 import 的层**（见 §6）。
- `kit/` 16：GearUI 之上的自建零件库（`GearLayout.kt` / `GearSelect.kt` / `GearFieldHelp.kt` / `GearWaveform.kt` / `GearLedRing.kt` / `GearTerminal.kt` / `GearTerminalScreen.kt` / `GearUiSpace.kt` / `AdminScope.kt` / `AdminScopeHost.kt` / `AdminSpacing.kt` / `AdminSystemAppearance.kt` 等）；`AGENTS.md:10` 有逐零件索引。
- `icons/` 2：`IconVectors.kt`、`AdminIcons.kt`（内嵌 SVG pathData，`:15` 逐字「图标一律 `GearIcon(IconToken.X)`…不回退资源文件/PNG」）。
- `state/` 1：`StateVar.kt`（`AGENTS.md:11` 逐字「状态一律 Compose 快照…stateVar=496/stateListOf=51、legacy=0」）。
- 文档：`client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md`（本人 60+ 行维护指引，`:143-146` 有「构建 web 管理页…`cd client && ./gradlew :apps:h5App:publishWeb`」「启动 server（admin_dir 默认 ../client/apps/h5App/web）」）。

**判定**：`commonMain` 153 个文件 **管理台专属**（UI 层），但 `contracts/`、纯模型文件（`EchoMuseBleCodec.kt`、`TimeFormat.kt`、`SecretMask.kt`、`PipelineFlow.kt`、`ConfigDraftChanges.kt`、`NodeTestModels.kt`、`FilesModel.kt`、`NluPlanJson` 等）被 §6 的测试模块以源码级复用——**删目录会连带打断这些测试**。

## 2. `client/shared` 内其他源集（归属：全部管理台专属）

| 源集 | .kt 数 | 文件 | 归属 |
|---|---|---|---|
| `macosArm64Main` | 12 | `AdminFonts.kt`、`AdminPlatform.kt`（真实现）、`AdminRealtime.kt`、`OtaFilePicker.kt`、`UploadFilePicker.kt`、`kit/AdminSystemAppearance.kt`、**`TestBenchPage.kt` / `TestBenchAsrTtsPage.kt` / `TestBenchLlmNluPage.kt` / `TestBenchState.kt`**、**`XiaoZhiModule.kt`**（原生桥 Kotlin 侧）、`VoiceCatalog.kt` | **macOS 测试台真实现集中地**，W14 已核 |
| `jsMain` | 6 | `AdminFonts.kt`、`AdminPlatform.kt`、`AdminRealtime.kt`、`OtaFilePicker.kt`、`UploadFilePicker.kt`、`kit/AdminSystemAppearance.kt` | Web 空 actual（W14 已核 `jsMain/AdminPlatform.kt:15-17` 空 + `XiaoZhiModule.kt:15-16`「web 包根本不含测试功能」） |
| `androidMain` | 6 | 同上 + `AdminRealtimeAndroid.kt` | 空 actual |
| `iosMain` | 6 | 同 jsMain | 空 actual |
| `commonTest` | 1 | `state/StateVarTest.kt` | 管理台状态单测 |

`client/AGENTS.md:6` 逐字「commonMain 管理台跨端复用，web 与 macOS 共用单一 `@Page("admin")` 壳（原 macOS 双壳 AdminShell 已废除，平台差异走 AdminPlatform expect/actual）；**麦克风/Opus 语音测试只在 macosArm64Main 与 macOS 原生桥，不把测试依赖带进 Web/Android/iOS/OHOS**」；`:7`「Web 双 bundle：shared 持有 JS 目标产业务包 nativevue2.js，h5App 壳产 h5App.js，入口先业务后壳。**不要移走 shared 的 JS 目标**」。

## 3. `client/gearuiKit/`（vendored GearUI Kit）

- **定量**（`find` 实测）：`commonMain` **256** 个 `.kt`；`macosArm64Main` / `iosMain` / `androidMain` / `jsMain` **各 7 个** actual（`CalendarPlatform.*.kt`、`Clock.*.kt`、`InterfaceStyle.*.kt`、`LineHeight.kt`、`NativeGenericFont.kt`、`NumericKeyboard.*.kt`、`PlatformKeyboard.*.kt`）——即 team-lead 背景「7 个 macosArm64 actual」实为 **4 平台 × 7 = 28 个 actual**。
- **依赖面**：全仓 `project(":gearuiKit")` 仅 **1 处**——`client/shared/build.gradle.kts:114` `implementation(project(":gearuiKit"))`；`settings.gradle.kts:66` `include(":gearuiKit")`、`:63-65` 逐字「GearUI Kit 组件库（Apache-2.0，源码 vendored）：上游只发布 android/ios/js，缺 macosArm64，本项目 macOS(Mac Catalyst) 是主力端，故源码引入并自补 macosArm64 目标」。**判定：仅 `:shared` 使用，随管理台一起下线，无其他模块依赖**。
- **许可义务**：`client/gearuiKit/THIRD_PARTY_NOTICES.md:1-14` 逐字「本模块全部源码为**上游 vendored**…**commonMain 源码零改动**…上游仓库 https://github.com/gearui/gearui-kit … commit `e5e32e930b…`（2026-10-29）…许可 **Apache License 2.0**…（见本目录 `NOTICE`）」——删除时 Apache-2.0 的 NOTICE 随分发义务需一并复核（见 2/2 §9）。

## 4. `client/apps/macosApp/`（Xcode 工程 + 原生桥）

- **构成**（`find` 实测）：自有 ObjC/C 约 **2336 行**（`*.m/*.h/*.mm` 排除 `Pods/`）；`Pods/` 2.4M（vendored `OpenKuiklyIOSRender` 等，127 个 `.h`、115 个 `.m` 主要来自此处）；Xcode 工程 `macosApp.xcodeproj/project.pbxproj` + `macosApp.xcworkspace`、`project.yml`（xcodegen）、`Podfile`/`Podfile.lock`、`Info.plist`、`Assets.xcassets/`、`AppDelegate.m`/`SceneDelegate.m`/`main.m`、`KuiklyRenderViewController.m`/`KRRouterHandler.m`、`XiaoZhiModule.m` 及 `+Audio.m`/`+Opus.m`/`+AdminWs.m`/`+Discovery.m`/`+Internal.h`、`XzAudioView.m`、`EchoMuseConsoleKeyboardHost.m`、`EchoMuseOfficialViewHost.m`、`AGENTS.md`。
- **判定：管理台壳 + macOS 语音测试台宿主（专属）**。`client/apps/macosApp/AGENTS.md:13` 逐字「宿主启动即打开 shared 的统一管理台 `@Page("admin")`（[SceneDelegate]，pageName 已从旧 `router` 切换）；macOS 与 web 同壳，平台差异（测试台 / 服务器设置 / 服务器基址）由 shared 的 AdminPlatform expect/actual 承载」；`:30`「`web` 受浏览器麦克风 / WebSocket 二进制等限制，不适合语音链路测试。macOS App（`apps/macosApp`，Mac Catalyst，复用 iOS 渲染器 `OpenKuiklyIOSRender`）承载端到端语音测试」；`:32`「**测试代码只在 `client/shared/src/macosArm64Main`**：web/Android/iOS/OHOS 的包不含测试代码」；`:34` 列 WS/采音/播放/Opus 原生桥已实现。
- **与 W14b 新 Tauri 壳的关系（team-lead 问「是否仍需保留作真机 Testbench 对照」）**：新壳在 `client/admin-web/src-tauri`（**不下线**）；旧端是**目前唯一已实现、可运行的 macOS 真机 Testbench**（五 tab + `XiaoZhiModule*.m` 原生桥，W14 已核 ~1600 行 ObjC/C + ~1600 行 Kotlin）。新 Tauri 壳 W14b-1/W14b-2 的硬件命令与本地桥 `client/admin-web/AGENTS.md:212-213` 仍属「本阶段明确未做」：`- 硬件命令实现（麦克风 / Opus / 播放 / mDNS）`、`- GUI 启动 pnpm tauri dev / pnpm tauri build；编译侧只做 cargo check 与 cargo test…本机无 GUI/真机环境`；`docs/plans/admin-web-migration/w14-testbench-architecture.md:169`「**L3 真机手动清单（交付时给用户一份 macOS 真机验收清单；以下结论只能真机给）**」共 8 项、均待验。→ **事实：删除 macosApp 会同时删掉唯一可用于真机对照的旧实现；新壳真机结论尚不存在**（详见 2/2 §8/§9，不下结论）。

## 5. `client/apps/h5App/web/`（静态产物）

- **12 文件逐字节**（`wc -c` 实测，合计 4.6M）：`index.html` 1,605；`nativevue2.js` **3,893,207**（shared 业务包）；`h5App.js` **352,570**（壳）；`em/index.html` 28,417；`em/static/adapter.js` 29,400；`em/static/dashboard.js` 440,653；`em/static/host.js` 12,937；`em/tests/{console,host,native-console,native-management,native-volume}.*.cjs` 共 5 个（6,531 / 11,600 / 11,023 / 10,627 / 7,555）。
- **git 跟踪状态（关键）**：`git ls-files client/apps/h5App/web/` → **仅 `index.html`**；`client/.gitignore:42` `apps/h5App/web/*.js`、`:46` `apps/h5App/web/em/`。即 `nativevue2.js`、`h5App.js`、`em/` 全部是 **gitignore 的构建产物**（详见 2/2 §9 可恢复性）。
- **生成来源**：`client/apps/h5App/build.gradle.kts:38-44` `syncBusinessBundle`（从 `shared/build/outputs/kuikly/js/release/local/nativevue2.zip` 取 `nativevue2.js` → `web/`）；`:98-104` `syncEmAssets` 逐字「拷进 web/em/：不含 jsx 源码与仓库文档（下发只需要可执行的三份 js，第三方库走 CDN）」，`from(emDir) { exclude("README.md","README.sha256","LICENSE","static/dashboard.jsx") } into(webDir.dir("em"))`；`:106-123` `publishWeb` 依赖 `:shared:verifyAdminIcons` + `verifyEmBundle` + `syncBusinessBundle` + `syncShellBundle` + `syncEmAssets`，并给 index.html 两个 bundle 引用打时间戳。
- **`em/` 源目录不是管理台资产**：`client/apps/h5App/em/` 12 文件**全部被 git 跟踪**（`git ls-files`）；`client/apps/h5App/em/README.md:1-6` 逐字「# 官方 EchoMuse 设备管理页（仅 web）…从上游 [EchoMuse controller] **原样 vendor**…由 Rust 服务端的静态目录同源托管在 `/em/`，**与 Kuikly 管理台（`/`）是两个独立文档，全局变量零冲突**」；`:184-185` 逐字「`web/em/` 是 `syncEmAssets` 的拷贝产物，已 gitignore；改 `em/` 结构后先 `rm -rf client/apps/h5App/web/em`（`publishWeb` 不清理旧文件）」。生成链另有 `tools/build_em_dashboard.sh:2/:19/:72`（产 `em/static/dashboard.js` 与 `README.sha256`）。**W12 已把 `/em/` iframe 接入 admin-web devicehub**（`client/admin-web/AGENTS.md:214`「`/em/` iframe 接入——**已在 W12 实现**（DevicehubPage + `emEmbed.ts`）」）→ 下线时 `em/` 必须单独处置，不能随 web 一起删。
- **纠正 team-lead 背景中的「wasm 资源」**：`find . -name '*.wasm'` 全仓命中只在 `plugins-src/*/target/wasm32-unknown-unknown/release/`（WASM 插件构建产物），**`h5App/web/` 下 12 个文件无 `.wasm`**。

## 6. Gradle 模块清单与共享引用面（判定「专属/共享」的核心）

`client/settings.gradle.kts:58-83` 全文：
- `:60-61` `include(":shared")`「跨端业务逻辑（Kotlin Multiplatform）：android / ios / js(web) 共用」；`:63-66` `include(":gearuiKit")`；`:68-70` `include(":bleProtocolTests")`；`:72-73` `include(":adminCoreTests")`「Production UI contracts and navigation models, without a Kuikly renderer or KSP」；`:77-78` `include(":apps:androidApp")` / `include(":apps:h5App")`「各平台宿主 App 统一汇聚在 apps/ 下」；`:79-83` 注释列 `apps/iosApp`（Podfile 链接 shared.framework）、`apps/macosApp`（Mac Catalyst，复用 iOS 渲染器，**提供 ASR/TTS 测试页，仅 Mac 可用**）、`apps/ohosApp`（HarmonyOS 宿主）为**非 Gradle 模块的原生宿主目录**。
- **`:sharedCompose` 已删除**：`settings.gradle.kts` 无该 include；`Dockerfile:51` 逐字「`638849b`（GearUI 全量迁移）**已删除整个 :sharedCompose 模块**」；`docs/plans/gearui-kit-migration-architecture.md:513` 逐字「⇒ `:sharedCompose`（19 文件…）**当前就是死模块**」。即 team-lead 问的「死模块是否已清理」= **已清理**，但 `Dockerfile:26/:43-73`、`docs/plans/gearui-kit-migration-*.md` 等历史文档仍留有大量 `sharedCompose` 事故/迁移口径（2/2 §7 过期文档）。

**依赖与复用面**（逐项给 file:line）：
- `client/apps/androidApp/build.gradle.kts:35` `implementation(project(":shared"))`；`client/apps/androidApp/src/main/kotlin/com/xiaozhi/admin/KuiklyRenderActivity.kt`、`proguard-rules.pro` 引用 `com.xiaozhi.admin`。
- `client/shared/build.gradle.kts:114` `implementation(project(":gearuiKit"))`（唯一 gearuiKit 引用）。
- `client/apps/h5App/build.gradle.kts:29` jsMain 只依赖 `com.tencent.kuikly-open.core-render-web:h5:$kuiklyVersion`，**不直接 project(":shared")**（业务包经 `:38-44` `syncBusinessBundle` 汇入）；`client/apps/h5App/src/jsMain/kotlin/com/xiaozhi/admin/` 6 个 .kt：`Main.kt`、`KuiklyWebRenderViewDelegator.kt`、`WebCursorPropHandler.kt`、`EchoMuseOfficialViewHost.kt`、`EchoMuseConsoleKeyboardHost.kt`、`utils/Ui.kt`（`client/AGENTS.md:8` 逐字「h5App [Main.kt] 设 `KuiklyProcessor.preventDefaultSelect = false`」）。
- `client/adminCoreTests/` **23 个 `.kt` 测试**，全部 `package com.xiaozhi.admin` 并直接 import 生产纯模型：`TimeFormatTest` / `EchoMuseWakeWordTest` / `UiContractsTest` / `PluginSlotsNullRegistrationTest` / `EchoMuseVolumeTest` / `AdminInterfaceModeTest` / `NluPlanJsonTest` / `FilesModelTest` / `EchoMuseConsoleTextTest` / `DevicePluginNavPenetrationTest` / `NodeTestModelsTest` / `SecretMaskTest` / `PluginLifecycleModelTest` / `PluginSlotsModelTest` / `PipelineWorkflowTest` / `PluginUiTreeTest` / `EchoMuseConsoleClosedTest` / `PipelineFlowTest` / `WorkflowEditorModelTest` / `EchoMuseTerminalScreenTest` / `PluginUiModelTest` / `ConfigDraftChangesTest` / `EchoMuseConsoleKeysTest`（`client/shared/src/commonMain/.../admin/AGENTS.md:61` 亦引这些测试）。
- `client/bleProtocolTests/` **1 个 `.kt`** `EchoMuseBleCodecTest.kt`；`client/AGENTS.md:21` 逐字「`bleProtocolTests` 是无页面、无 Kuikly/KSP 的普通 KMP 测试模块，只引用生产 `EchoMuseBleCodec.kt` 源文件并运行纯协议/校验回归，**不复制实现**，也不参与生产 bundle」；`docs/plans/gearui-kit-migration-prd.md:210` 逐字「只删 `ui/` 不改这两个文件，`:apps:h5App:publishWeb` 与 `:adminCoreTests:jvmTest` 会直接失败」。
- 原生宿主：`client/apps/iosApp/{Podfile,KuiklyRenderViewController.m,KRRouterHandler.m}`、`client/apps/ohosApp/entry/src/main/ets/pages/Index.ets:14`（逐字「Kuikly 页面容器：pageName 对应 shared 中的 `@Page("router")`」——**已是过期口径，现为 `admin`**）、`client/apps/macosApp/{project.yml:2/:113,macosApp.xcodeproj/project.pbxproj}`。
- 其他：`client/scripts/codemod_compose_state.py`（`verifyComposeStateMigration` 护栏脚本）、`tools/admin_ui_browser_check.mjs:4/:15`（逐字「Requires a freshly published `client/apps/h5App/web` artifact」、`const built = path.join(root, 'client/apps/h5App/web')`）。

**判定小结**：管理台 **UI 层专属**（shared 全量 + gearuiKit + macosApp + h5App web 产物）；**契约/纯模型层共享**（adminCoreTests 23 + bleProtocolTests 1 + h5App jsMain 6 + 4 宿主）；`h5App/em/` 与 `plugins-src/` 的 wasm **不属于管理台**。

---

2/2 接续：全仓引用清理清单（代码/文档/CI/部署）、三形态选项逐项事实、诚实化与风险登记（含「真机验收 10 项」候选口径的诚实说明）、需拍板清单。

---

---

> **以下为「报告 2/2」原文，全量保留，未删减。**

---

W15 / 旧端下线前置研究 A **报告 2/2（下线影响面 + 形态选项事实 + 诚实化/风险登记 + 需拍板清单）**。路径基准同 1/2（相对仓库根 `/Users/admin/Desktop/notip.com.cn/xiaozhi-server-rust`）。分工：构建链细节（Docker web 阶段改造 / dist 切换 / em 落点）归 `w15-build-researcher`，本节只登记「哪些文件引用了旧端」这一清理面。

## 7. 下线影响面：引用清理清单

### 7.1 代码 / 构建脚本引用
| 引用 | file:line | 清理动作 |
|---|---|---|
| `@Page("admin")` | `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AdminPage.kt:65`（唯一注解） | 随文件删；注释引用另见 `ConfigPage.kt:41/:270`、`ConfigWorkspaceViews.kt:167`、`kit/AdminScopeHost.kt:26`、`ConfigFormState.kt:353/:360`、`AdminPlatform.kt:14-15` |
| `@Page("config")` | `client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ConfigPage.kt:63` | 同上 |
| KSP 注册 `KuiklyCoreEntry` | `client/shared/build.gradle.kts:13-14` 逐字「Kuikly KSP：扫描 @Page 注解生成页面注册代码（KuiklyCoreEntry，按编译目标注入）」、`settings.gradle.kts:21-22` KSP 插件版本 | 删 `:shared` 后 KSP 宿主插件与 `com.google.devtools.ksp` 声明一并复评 |
| `patchFrameworkForCatalyst` | `client/shared/build.gradle.kts:337`（任务定义）、`:372`（`finalizedBy`）；补丁脚本 `client/scripts/patch_framework_macabi.py`（`client/apps/macosApp/AGENTS.md:11`） | 随 `:shared` macosArm64 目标删 |
| `linkReleaseFrameworkMacosArm64` | `client/apps/macosApp/AGENTS.md:5/:38`、`client/apps/macosApp/project.yml:2/:113`、`docs/plans/gearui-kit-t13-acceptance.md:183` | 随 macosApp 删 |
| `publishWeb` / `syncEmAssets` / `syncBusinessBundle` / `verifyAdminIcons` / `verifyEmBundle` / `packLocalJSBundleRelease` | `client/apps/h5App/build.gradle.kts:38-44/:98-104/:106-123`、`:109`（dependsOn `:shared:verifyAdminIcons`）、`client/shared/build.gradle.kts:15`（`packLocalJSBundle*`）、`client/AGENTS.md:17-18` | 构建链改造由 w15-build-researcher 负责 |
| `implementation(project(":shared"))` | `client/apps/androidApp/build.gradle.kts:35` | 删 `:shared` 必改 |
| `implementation(project(":gearuiKit"))` | `client/shared/build.gradle.kts:114`（全仓唯一） | 随 `:shared` 删 |
| `:sharedCompose` 历史口径 | `Dockerfile:26/:43-73`、`:51` 逐字「`638849b`（GearUI 全量迁移）**已删除整个 :sharedCompose 模块**」、`docs/plans/gearui-kit-migration-{architecture,prd,scope-reduction,t13-acceptance}.md` 多处 | **模块本体已清理**；仅剩文档/注释里的历史事故口径，删除后自然消失 |
| 纯模型复用（不能误删） | `client/adminCoreTests/` 23 个 `.kt`、`client/bleProtocolTests/src/commonTest/kotlin/com/xiaozhi/admin/EchoMuseBleCodecTest.kt`、`client/AGENTS.md:21` | 若删 `shared`，这两个模块要么一并删、要么把被引用纯模型另迁；`docs/plans/gearui-kit-migration-prd.md:210` 逐字「只删 `ui/` 不改这两个文件，`:apps:h5App:publishWeb` 与 `:adminCoreTests:jvmTest` 会直接失败」 |

### 7.2 文档引用（全仓 `*.md` grep 命中 47 个文件，分组）
- **根**：`AGENTS.md:3` 逐字「小智 ESP32 语音服务端（Rust）与 **Kuikly 多端管理台**」；`README.md:3`「内置 Kuikly 多端管理台（Web 直开 + macOS 原生测试台）」、`:28` 架构图 `ADMIN[Kuikly 管理台<br/>Web + macOS 测试台]`、`:88`「`./dev-server.sh # 真实 Rust server + 已构建的 Kuikly 管理台`」、`:97`「`XIAOZHI_ADMIN_DIR` 可指定已有 Web 目录…UI 修改后需重建并刷新浏览器」、`:134-137` 目录树「`client/ # Kuikly 多端管理台`…`apps/ # h5App(Web) / macosApp(Catalyst) / iosApp / androidApp / ohosApp`」。
- **client 侧**：`client/AGENTS.md:3-9`（含 `:8` h5App `Main.kt` 口径）、`:13-23`（验证命令与 publishWeb 指引）、`:33`「[Admin Web](<admin-web/AGENTS.md>)：React + shadcn/ui + Tauri 2 管理台开发环境（**占位骨架，不参与 Gradle 构建**）」——**已过期**（W0–W14b 已全迁完）；`client/shared/src/commonMain/kotlin/com/xiaozhi/admin/AGENTS.md:5/:13/:61/:109-116/:143-146`；`client/apps/macosApp/AGENTS.md:5/:13/:24-49`（整篇 macOS 测试台使用说明）；`client/apps/h5App/em/README.md:1-6/:58/:117/:153-154/:184-185`。
- **admin-web 侧**：`client/admin-web/AGENTS.md:50/:175`（旧端对照口径）、`:214`（`/em/` 已在 W12 实现）、`:225` 逐字「后续顺序 **W15 旧端下线**（切 admin-web/dist 构建链、验收矩阵 + 灰度前置、**归档 client/shared 管理台与 h5App/web**）」——**这是 W15 的既有规划口径，且用词是「归档」而非「删除」**；`:210-221`「本阶段明确未做」10 项；`client/admin-web/src/components/config/ServerSection.tsx:64-68` `path="server.admin_dir"` 的 hint 逐字「server 在 / 直接托管该目录；为空或目录不存在时返回友好提示。**默认 ../client/apps/h5App/web**」+ placeholder 同值——**新前端里也硬编码了旧端路径，需同步改**。
- **server 侧**：`server/src/config.rs:217-219` 逐字「管理页面（h5App 构建产物）静态目录；server 在 `/` 直接托管该目录…默认 "../client/apps/h5App/web"」；`server/src/dev.rs:83-91`（`XIAOZHI_ADMIN_DIR` 未设时指向 `../client/apps/h5App/web`）；`server/src/config/AGENTS.md`、`server/src/config/store/AGENTS.md`、`server/src/gateway/ws/AGENTS.md`、`server/src/gateway/files/AGENTS.md`；`plugins/registry/projection.rs:114`（`"server.admin_dir"` 投影键）。
- **docs/**：`docs/README.md`、`docs/architecture.md`、`docs/voice-pipeline.md:85/:105`、`docs/echomuse.md:312`、`docs/nlu-model-validation-20261101.md:195-196`、`docs/designs/admin-ui-design.md:59/:348`、`docs/designs/plugin-architecture-unification.md:83/:272/:298`、`docs/designs/soul-and-graph-memory-plan.md:82/:171`、`docs/designs/wasm-plugin-platform.md:454/:478`、`docs/designs/gap-implementation-design.md`、`docs/plans/` 下 `gearui-kit-migration-prd.md`（:109-111/:210/:362/:394-404/:435-436）、`gearui-kit-migration-architecture.md`（:15/:17/:71/:80/:334-338/:489-515/:696/:735/:765-770/:831/:897-898/:1070-1073/:1131-1138/:1198/:1305/:1333）、`gearui-kit-scope-reduction.md:45-46`、`gearui-kit-t13-acceptance.md:83-91/:122/:133/:183`、`echomuse-admin-parity-{prd,design,qa-report}.md`、`p1-round1-design.md:314`、`p1-features-prd.md`、`p2-round2-design.md`、`p3-round3-design.md`、`overall-implementation-plan.md`、`nlu-iot-voice-architecture.md`、`nlu-skill-domain-architecture.md`、`tuya-integration-{prd,architecture}.md`、`admin-web-migration/w12-devicehub-research.md:48-58/:152/:380-417`、`w13-settings-research.md`、`w14-testbench-architecture.md`。
- **plugins / tools**：`plugins/AGENTS.md:113`、`plugins/soul/AGENTS.md:94`、`plugins/registry/AGENTS.md:254`；`tools/admin_ui_browser_check.mjs:4/:15`（逐字「Requires a freshly published `client/apps/h5App/web` artifact」）；`tools/build_em_dashboard.sh:2/:12/:19/:72`；`client/scripts/codemod_compose_state.py`。
- **docs/research/** 历史研究文档：`code-simplification-analysis.md`、`arch-proposal-review.md`、`architecture-optimization-analysis.md`、`wyoming-provider.md`（泛提 Kuikly/旧端，属历史归档，通常不改）。

### 7.3 部署 / CI 绑定（只列事实，不深入构建链）
- **`dev-server.sh:10-18`**（逐字）：`WEB="${XIAOZHI_ADMIN_DIR:-$ROOT/client/apps/h5App/web}"` → `:11` 检查 `index.html`/`nativevue2.js`/`h5App.js` → `:16` `(cd "$ROOT/client" && ./gradlew :apps:h5App:publishWeb -PwebSourceMap=false)` → `:18` `export XIAOZHI_ADMIN_DIR="$WEB"` → `:31` `cargo run --no-default-features -- --dev "$@"`。**这是 dev 模式与旧端产物最硬的绑定**：默认目录指向 h5App/web、缺文件即触发 Gradle 旧端构建。
- **`server/src/config.rs:217-219`** `default_admin_dir()` 默认 `../client/apps/h5App/web`；**`server/src/dev.rs:83-91`** 同路径兜底；`gateway/ws/mod.rs:326-334` `ServeDir::new(&admin_dir)` + `api.fallback_service(admin)`（W12 报告 `:57` 已核）；目录不存在时 `:353-359` 中文提示逐字「管理页面未找到：请先构建 h5App（web）并将 [server].admin_dir 指向其包含 index.html 的输出目录。」
- **Dockerfile**（引用事实）：`:150-152` COPY 三个 `build.gradle.kts`；`:177/:179/:180` `COPY client/shared ./shared` / `client/gearuiKit ./gearuiKit` / `client/apps/h5App ./apps/h5App`；`:189-190` 断言 `apps/h5App/src/jsMain/kotlin/com/xiaozhi/admin/Main.kt`；`:219` 两个 `jsBrowserProductionWebpack`；`:233` `:apps:h5App:publishWeb`；`:235-240` 断言 `web/index.html`、`nativevue2.js`、`h5App.js`、`em/index.html`、`em/static/dashboard.js`、`em/static/adapter.js`；`:260` `COPY --from=web …/web/index.html /tmp/.web-stage-done`；`:399` `COPY --from=web /src/client/apps/h5App/web /app/web`。`:122` 逐字「web —— Gradle 构建 Kuikly H5 管理页（client 工程：shared 业务包 + h5App 壳）」。→ **删除旧端必须同步改 Dockerfile 的 web 阶段与 `/app/web` 落点**（具体改造归 w15-build-researcher）。
- **`.github/workflows/docker-amd64.yml`**：`:1`「xiaozhi-server-rust 镜像构建（linux/amd64 → 阿里云 ACR）」；`:10-12` 逐字「⛔ 与 ACR「源代码自动构建」互斥…本仓库 Dockerfile 头部记录的现行发布链路是 ACR 控制台的源码自动构建——每次 push 重建 amd64 :latest」；`:27` 注释「阶段整段命中，只改 client/ 时 Rust 阶段命中，清单不动时依赖层命中」；`:52-54` `ACR_REGISTRY: registry.cn-hangzhou.aliyuncs.com` / `ACR_NAMESPACE: notip-com-cn`。**工作流本身不直接引用旧端产物路径，但触发的 Dockerfile 引用；且存在「ACR 控制台源码自动构建」这条仓库外并行链路，需 team-lead 确认是否一并收口**（仅列事实）。
- `README.md:97` `XIAOZHI_ADMIN_DIR` 说明；`client/admin-web/src/components/config/ServerSection.tsx:64-68` 默认路径 hint。

## 8. 形态选项事实（不替下结论，逐项给「需改哪些文件 / 回滚难度 / 与真机验收的前置关系」）

| 维度 | ① 彻底删除旧端源码与产物 | ② 归档保留目录但停构建（软下线，可回滚） | ③ 并行灰度（admin_dir 开关切换新旧产物） |
|---|---|---|---|
| **需改文件** | `client/settings.gradle.kts`（删 `:shared`/`:gearuiKit`/`:bleProtocolTests`/`:adminCoreTests`/`:apps:androidApp`/`:apps:h5App` 的 include）、`client/build.gradle.kts` 与根 `build.gradle.kts`（Kuikly 插件/版本）、`client/gradle.properties`（`kuiklyVersion`）、`client/kotlin-js-store/yarn.lock`、`client/scripts/`、`client/apps/` 全部旧宿主目录、`client/shared/`、`client/gearuiKit/`、`client/adminCoreTests/`、`client/bleProtocolTests/`、`dev-server.sh:10-18`、`server/src/config.rs:217-219`、`server/src/dev.rs:83-91`、`Dockerfile` web 阶段与 `:260/:399`、`.github/workflows/docker-amd64.yml`（若同期收口）、`tools/{admin_ui_browser_check.mjs,build_em_dashboard.sh}`、§7.2 全量文档、`ServerSection.tsx:64-68`；**另须单独保留并改挂 `client/apps/h5App/em/`**（1/2 §5，它不是管理台资产） | 只改「生产不再构建/不再指向旧端」的部分：`dev-server.sh:10`（默认 WEB）、`server/src/config.rs:219`（默认值）、`Dockerfile` web 阶段（停触发，具体归 w15-build-researcher）、`.github/workflows/docker-amd64.yml`（若涉及）、§7.2 文档加 deprecated 标注；`settings.gradle.kts` 与各源码目录**可原样保留**（仍能本地重建） | 需让 server 同时/可切换两套静态目录：现状是**单一** `admin_dir`（`config.rs:220`、`dev.rs:83-91`、`gateway/ws/mod.rs:326-334`）；现成开关是 `XIAOZHI_ADMIN_DIR`（`dev-server.sh:10`、`README.md:97`），可在部署层用环境变量分流而不改 server 源码；但 `/em/` 必须在两套产物下都可达（W12 报告 `docs/plans/admin-web-migration/w12-devicehub-research.md:408-416` 逐字「默认部署下 `/em/` 同源可达…若生产用 admin-web/dist 作 admin_dir，必须先新增「把 em/ 拷进 dist」的构建步骤（当前零）」——该步骤归 w15-build-researcher）；另需改 `ServerSection.tsx:64-68` 默认值/文案 |
| **回滚难度** | **最高**：源码可由 git 历史恢复（§9），但 gitignore 产物（`web/*.js`、`web/em/`、`shared/build/`、`*.framework`、`Pods/` 集成）需重建；且 `:shared` 一旦删，§7.1 的 KSP/Catalyst/Gradle 链与 §6 的 3 个测试模块、4 个宿主全部断链，回滚要成组恢复 | **最低**：改回 `admin_dir` / 恢复 CI 阶段即可，源码与 macosApp 工程都在 | **最低**：切环境变量即可回旧；风险在两套产物的 `/em/` 与静态资源一致性 |
| **与「真机验收待验」的前置关系** | **冲突最直接**：删 `client/apps/macosApp/` = 删掉目前**唯一已实现可运行的 macOS 真机 Testbench**（1/2 §4）；新 Tauri 壳硬件命令/本地桥 `client/admin-web/AGENTS.md:212-213` 仍「明确未做」、`w14-testbench-architecture.md:169` L3 真机清单 8 项全待验、`admin-web/AGENTS.md:213` 逐字「本机无 GUI/真机环境」。→ 若无旧端对照，新壳真机出问题时无法 A/B 区分是新壳缺陷还是环境/服务端问题 | **最契合待验状态**：旧端源码 + macosApp 工程原位保留，可随时重建作真机对照；新壳 8/10 项验收期间旧端仍可用 | **可让新壳在灰度环境真机验**、旧端保持生产可用；代价是要同时维护两套产物的 `/em/` 与 admin_dir 口径 |

## 9. 诚实化 / 风险登记

**删除后永久缺失的能力**：
1. **旧端 macOS 原生 Testbench 真实现**：`client/shared/src/macosArm64Main/.../TestBench*.kt`（4 文件）+ `XiaoZhiModule.kt` + `client/apps/macosApp/XiaoZhiModule.m` 及 `+Audio.m`/`+Opus.m`/`+AdminWs.m`/`+Discovery.m`（自有 ObjC/C 2336 行）——W14 已核五 tab（连接/ASR/TTS/LLM/NLU）、AVAudioEngine 直采、AudioToolbox Opus、mDNS `_xiaozhi-admin._tcp` 发现。**新 Tauri 壳的等价实现尚在 W14b-1/W14b-2 且真机未验**（`w14-testbench-architecture.md:169/:200-204`）。
2. **Kuikly 跨端真机对照**：Android（`client/apps/androidApp`）、iOS（`Podfile` 链接 shared.framework）、OHOS（`Index.ets`）四个宿主——新方案是 `Tauri 2 + React`，只覆盖桌面；`client/settings.gradle.kts:79-83` 列的三个原生宿主目录随旧端一起失去意义。
3. **em/README 相关链**：若连同 `client/apps/h5App/em/` 一起删，会连带丢上游 vendor 的 `LICENSE`（MIT 随分发义务）、`README.sha256`、`dashboard.jsx` 源与 `tools/build_em_dashboard.sh` 生成链，且 W12 已接入的 `/em/` iframe（`client/admin-web/AGENTS.md:214`）失效——**明确列为「不能随旧端删」项**。
4. **gearuiKit 许可义务**：`client/gearuiKit/THIRD_PARTY_NOTICES.md:1-14` Apache-2.0 + `NOTICE` 随分发义务；删除目录前应确认没有遗留分发物仍含该组件（`nativevue2.js` 是编译产物，若仍被分发需保留声明）。

**会变成过期口径的文档**（择要，全量见 §7.2）：根 `AGENTS.md:3` / `README.md:3/:28/:88/:134-137` 仍称「Kuikly 多端管理台」；`client/AGENTS.md:33`「Admin Web…占位骨架，不参与 Gradle 构建」；`client/apps/ohosApp/entry/src/main/ets/pages/Index.ets:14`「pageName 对应 shared 中的 `@Page("router")`」（现为 `admin`）；`client/shared/.../admin/AGENTS.md:5` vs `:13` 的「10 项 / 9 项」自相矛盾；`client/admin-web/AGENTS.md:225` 的「W15 旧端下线」完成后本身即过期；`ServerSection.tsx:66-68` 默认 `../client/apps/h5App/web`；`server/src/config.rs:217-218` 注释「h5App 构建产物」；`Dockerfile:26/:43-73` 的 `sharedCompose` 事故注释；`docs/plans/gearui-kit-*` 系列整组迁移文档。

**git 历史可恢复性（关键）**：
- **可恢复**：`client/shared/`、`client/gearuiKit/`、`client/apps/macosApp/`（除 Pods 构建物）、`client/apps/h5App/em/` 12 文件、`client/apps/h5App/web/index.html`、`client/adminCoreTests/`、`client/bleProtocolTests/`、各 `build.gradle.kts`、全部文档 —— 均 `git ls-files` 在册。
- **不可恢复，需重建**：`client/apps/h5App/web/nativevue2.js`（3.89 MB）、`h5App.js`（352 KB）、`web/em/` 整个目录 —— `client/.gitignore:42` `apps/h5App/web/*.js` 与 `:46` `apps/h5App/web/em/`；`client/shared/build/`、`*.framework`、`client/apps/macosApp/Pods/` 集成、`client/apps/macosApp/macosApp.xcworkspace` 等构建产物。`client/apps/h5App/em/README.md:184-185` 逐字「`web/em/` 是 `syncEmAssets` 的拷贝产物，已 gitignore…`publishWeb` 不清理旧文件」——即产物丢失只能靠 `:apps:h5App:publishWeb` 重建，而该任务依赖 `:shared`（若已删则**无法重建**）。→ **彻底删除前必须先确认不再需要任何旧产物，或先把 `em/` 源目录迁到新落点**（落点方案归 w15-build-researcher）。

**「真机验收清单 10 项」口径诚实说明**：本波全仓检索**未定位到独立命名为「真机验收清单 10 项」的文档**。可对应的三处候选，请 team-lead 对齐所指：
- **A**：`client/admin-web/AGENTS.md:210-221`「## 本阶段明确未做」共 **10 项**，其中 `:212`「硬件命令实现（麦克风 / Opus / 播放 / mDNS）」、`:213`「GUI 启动 `pnpm tauri dev` / `pnpm tauri build`；编译侧只做 `cargo check` 与 `cargo test`…本机无 GUI/真机环境」——最贴近「待验」语义。
- **B**：`docs/plans/admin-web-migration/w14-testbench-architecture.md:169-180`「L3 真机手动清单」共 **8 项**（TCC 授权 / ATS 拦 loopback WS / cpal 全双工 / Opus 帧中转抖动 / 端口冲突 / 跨机 LNA / 波形试听 / `--dev` 不可用），`:180` 逐字「编译通过 / `cargo test` 通过 / mock 通过 / 接口 200，一律不等于真机硬件通过」。
- **C**：同文档 `:208-223`「§5 需 team-lead / 用户拍板的架构项」共 **10 项**（已由 lead 确认采用，非待验）。
本报告 §8 的「与真机验收的前置关系」按 **A + B 均待验**这一事实陈述，不擅自把 8 项说成 10 项。

## 10. 需 team-lead / 用户拍板的清单（研究侧只给事实与倾向，不替下结论）

| # | 拍板项 | 事实依据 | 研究侧倾向（非结论） |
|---|---|---|---|
| 1 | **形态 ①/②/③** | §8 全表；`client/admin-web/AGENTS.md:225` 既有规划用词是「**归档** client/shared 管理台与 h5App/web」 | 倾向 ②（软下线）作 W15 第一步：与「真机验收待验」不冲突、回滚最低、且贴合既有「归档」口径；③ 可作 ② 的灰度验证手段；① 建议等真机验收通过后再单独一波 |
| 2 | **`client/apps/h5App/em/` 是否随旧端下线** | 1/2 §5：`em/README.md:1-6` 逐字「与 Kuikly 管理台是两个独立文档」、`client/admin-web/AGENTS.md:214` `/em/` 已接入 devicehub、W12 报告 `:408-416` | **明确不删**，需单独给新落点（归 w15-build-researcher） |
| 3 | **`client/apps/macosApp/` 是否保留作真机对照** | 1/2 §4、§8「前置关系」：旧端是唯一可运行真机 Testbench，新壳 8 项 L3 全待验 | 倾向至少保留到 W14b 真机验收出结论再决定 |
| 4 | **`client/adminCoreTests`（23）/`bleProtocolTests`（1）如何处置** | 1/2 §6、`client/AGENTS.md:21`、`gearui-kit-migration-prd.md:210` | 删 `:shared` 必连带；若要保留纯模型回归，需先把被引用的 `contracts/` 与纯模型另迁（工作量需单列） |
| 5 | **`XIAOZHI_ADMIN_DIR` 默认值与 `dev-server.sh` 是否本波改** | `dev-server.sh:10-18`、`server/src/config.rs:217-219`、`server/src/dev.rs:83-91`、`:16 DISABLED` | 属「切 dist 构建链」范畴，与 w15-build-researcher 结果绑定，需两路对齐后一并拍板 |
| 6 | **ACR 控制台「源码自动构建」链路是否收口** | `.github/workflows/docker-amd64.yml:10-12` 逐字、`:1` | 仓库外链路，仅列事实，需确认归属 |
| 7 | **文档口径更新范围** | §7.2 47 个 md；`AGENTS.md:3`、`README.md:3`、`client/AGENTS.md:33`、`ServerSection.tsx:66-68`、`ohosApp/Index.ets:14` 已过期 | 需定「全量改 / 只改入口文档 + 历史文档加 deprecated 标注」 |
| 8 | **「真机验收 10 项」对齐** | §9 候选 A/B/C | 请明确所指清单，作为形态 ① 的前置门禁 |
| 9 | **旧产物可恢复性兜底** | §9 gitignore 事实、`em/README.md:184-185` | 若选 ①，删除前需确认 `em/` 源已迁新落点、且不再需要 `nativevue2.js`/`h5App.js` 任何回滚 |

---

**收尾声明**：本波全程只读 —— 未改任何文件、未构建、未执行 git 写操作（仅 `git ls-files` / `git check-ignore` 只读查询）。1/2 + 2/2 两段即本任务完整交付，无未报告的附带结论。与 `w15-build-researcher` 的重叠面（Docker web 阶段 / dist 切换 / em 落点）已按分工只登记引用事实、未深入构建链。等待 team-lead 下一步派发。

---

---

> **以下为「补充勘误」原文（#54 收尾对齐，发给 team-lead），全量保留，未删减。**

---

W15 收尾 + 与构建链分路的衔接对齐（补充 #54，非新研究，全程只读）：

**任务状态**：#54 已 completed；刚 `TaskList` 复核 54/54 全部 completed、0 in_progress、0 pending。W15 两段报告（1/2 资产盘点 + 2/2 影响面/形态/风险/拍板清单）即本任务完整交付。

**收到 w15-build-researcher 的构建链分路衔接同步，核过之后：两路结论独立一致、无冲突**，可放心合并拍板：
- **em 归属**：我从资产盘点侧、他从构建链侧都判定 `client/apps/h5App/em/` 必须保留或迁移到 admin-web 侧，**不能随旧端 Kotlin 页面一起划入可删集合**（emDir build.gradle.kts:65、syncEmAssets :98-104、verifyEmBundle :72-95；W12 已接入 devicehub iframe）。我 2/2 拍板清单第 2 项「`h5App/em/` 是否随旧端下线」正是为此设的拍板项。
- **git 边界一致**：`web/index.html` 入库可恢复；`nativevue2.js` / `h5App.js` / `web/em/` / `admin-web/dist/` 忽略（`git check-ignore` 实测）。

**两处诚实化/细化，请拍板时以更正后口径为准**：
1. `em/` 实测 **13 个文件全部入库**（`git ls-files client/apps/h5App/em/ | wc -l` = 13；`w12-devicehub-research.md:380` 基线 4+4+5 = 13，含 `tests/` 5 个 `.cjs`）—— 我 W15 1/2 写的「12 文件」是计数偏差，更正为 13。w15-build-researcher 已同步按 13 口径。
2. `web/index.html` 虽入库，但 `publishWeb` 的 `doLast` 会写回 `?v=` 时间戳（build.gradle.kts:110-121），属「入库但会被构建改动」的文件，建议并入 W15 风险登记的可恢复性口径。

**边界声明**：Dockerfile web stage / ACR build-args / .dockerignore / NODE_OPTIONS 等构建链改造细节归 w15-build-researcher 分路，按分工我只登记引用事实、不深入；其中 `client/AGENTS.md:13-23` 的 `:shared:verifyAdminIcons` 与我 1/2 登记的「`contracts/` 被 `adminCoreTests` / `bleProtocolTests` 源码级复用」是同一条「删 `:shared` 前须确认复用」约束。

**等待下一步派发**：2/2 的 9 项需拍板清单（形态 ①/②/③、em 归属、macosApp 真机对照、adminCoreTests/bleProtocolTests 处置、`XIAOZHI_ADMIN_DIR` 默认值与 `dev-server.sh`、ACR 链路收口、文档口径、真机验收 10 项对齐、旧产物可恢复性兜底）仍待对齐。如需我把「em/ 单列资产 + 引用清理清单」合并成一份可直接拍板的附件，或对齐「真机验收 10 项」候选口径（A/B/C），随时派发。本波未改任何文件、未构建、未执行 git 写操作（仅 `git ls-files` / `git check-ignore` 只读查询）。

---

---

> **以下为「协作勘误闭环」原文（发给 w15-build-researcher 的两条衔接消息），全量保留，未删减。**

---

### 衔接事实已核过，两路结论一致（L2479）

W15 衔接同步收到，五点逐条核过，结论与我 #54「旧端资产盘点与下线影响面」独立一致、没有冲突。逐条对齐：

1. **em 归属**：完全印证。我 1/2 已把 `client/apps/h5App/em/` 单列为「与 Kuikly 管理台是两个独立文档、W12 已接入 admin-web devicehub iframe、不能随旧端 web 产物一起删」；2/2 需拍板清单第 2 项也列了「`h5App/em/` 是否随旧端下线」。你从构建链侧（emDir :65 / syncEmAssets :98-104 / verifyEmBundle :72-95）得出「必须保留或迁移到 admin-web 侧」，两边独立同源，这个一致性我同步给 team-lead 供合并两路报告拍板时直接用。

2. **w12 基线**：确认 `w12-devicehub-research.md:379-385` 就是逐文件清单基线，我 #54 盘点与它一致，未重复枚举。

3. **git 边界**：`git check-ignore` 实测 `web/nativevue2.js` / `web/h5App.js` / `web/em/index.html` / `admin-web/dist/index.html` 命中忽略，`web/index.html` 未忽略且 `git ls-files client/apps/h5App/web/` 仅返回 `index.html` 入库 —— 与你一致。补一处细化：`publishWeb` 的 `doLast` 会写回 `index.html` 的 `?v=` 时间戳（build.gradle.kts:110-121），所以它是「入库但会被构建改动」的文件，归档/回滚按此理解。

4. **部署面失效清单**：Dockerfile / ACR workflow build-args / .dockerignore / client/AGENTS.md:13-23 这些构建链改造归你分路，按 team-lead 分工我只登记引用事实、不深入。其中 `client/AGENTS.md` 的 `:shared:verifyAdminIcons` 与我 1/2 登记的「`contracts/` 与纯模型被 `adminCoreTests` / `bleProtocolTests` 源码级复用」是同一条约束：删 `:shared` 前须先确认这层复用，两边一致。

5. **NODE_OPTIONS 本机坑**：构建链细节，归你，不重复。

**一处诚实化修正（重要）**：`em/` 实测是 **13 个文件全部入库**（`git ls-files client/apps/h5App/em/ | wc -l` = 13；`w12:380` 基线 4+4+5 = 13，含 `tests/` 5 个 `.cjs`），我 W15 1/2 报告写的「12 文件」是计数偏差，以 13 为准。我会在给 team-lead 的对齐里更正，也请你的构建链报告按 13 口径。另外 `syncEmAssets` 的 exclude（:102）不含 `tests/`，5 个 `.cjs` 会随 `/em/` 下发 —— 这属构建链口径，归你判断是否收口，我只登记。

有需要我把 em/ 逐文件字节或引用清理清单展开随时说。

---

### 勘误闭环确认，运行时最小集与你的白名单选项吻合（L2492）

勘误闭环确认，谢谢。三点收口：

1. **13 口径两路一致**：`git ls-files` 根4 + static/4（含 dashboard.jsx）+ tests/5 = 13；`verifyEmBundle`(:79-94) 与 `exclude`(:102) 口径本身无误这点我认同 —— 偏差只在「源目录构成」的计数，闸门与拷贝逻辑不用动。

2. **盘点侧补一条运行时佐证**（只读核实，不展开逐文件字节）：`client/apps/h5App/em/index.html:587-591` 只引 `static/adapter.js`、`static/dashboard.js`、`static/host.js` 三 js（第三方库走 CDN 带 SRI，:583-584）。即 em/ 浏览器运行时最小集 = `index.html` + `static/` 三 js 共 **4 文件**，其余 9 个（`dashboard.jsx` 构建源 + `LICENSE` + `README.md`/`README.sha256` + `tests/` 5 个 `.cjs`）是源/许可/Node 测试，非运行时所需 —— 与你的「只拷白名单 4 文件」选项完全吻合，也再次印证「5 个 .cjs 属非必要下发」。

3. **分工确认**：`tests/` 下发扩散到 `dist/em/tests/`、三选项收口、风险 R13、需拍板清单第 1 项全部归你分路，我不重复登记、不另发 team-lead（我上一条对齐消息已含我这边的 13 更正，你已发专项勘误，两边信息对称，避免刷屏）。逐文件字节按你说的不再展开。

两路结论一致、无冲突。我 #54 已 completed、TaskList 54/54 全部完成，全程只读待命，等 team-lead 下一步派发。
