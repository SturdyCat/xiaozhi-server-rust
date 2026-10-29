# Client 管理台 UI 全量迁移到 GearUI Kit · 迁移规格（PRD）

> 版本：v1.0　作者：产品经理 许清楚　日期：2026-10-27
> 上游输入：主理人已确认的调研事实（GearUI Kit 组件清单 / 本项目现状盘点 / 用户三项决策）。
> 本文档**替代**此前 `compose/AdminComposeShell.kt` 里"逐段搬运 + 旧版出口"的渐进路线，改为**一次性全量替换**的规格基线。
> 约束：本文只定义"换成什么、换到什么程度算完"，不含代码；不修改 `client/` 下任何源文件。

---

## 1. 项目信息

| 项 | 值 |
|---|---|
| 产品 | xiaozhi-server-rust 自托管小智语音助手服务端 · Client 管理台（Kuikly KMP） |
| 本期范围 | 管理台全部 UI 表达层从**自研传统 Kuikly DSL 组件层**迁移到 **GearUI Kit（Kuikly Compose DSL）** |
| 目标套件 | `com.gearui:gearui-kit:1.0.0`（Apache-2.0，82 组件，iOS 风格默认主题） |
| 技术前置 | KuiklyUI **2.30.0** + Kotlin **2.1.21** + Compose MP **1.7.3** |
| 端范围 | web(H5) / Android / iOS / macOS(Mac Catalyst) 四端统一，**UI 单份实现于 `commonMain`** |
| 语言口径 | 全中文注释与错误文案；`.kt` 逻辑文件 ≤ 600 行；Markdown 只落根 `docs/`；不自动 git add/commit |
| 原始需求复述 | "把 client 管理台的 UI 全部换成 GearUI Kit。macOS 用源码引入 + 自补 macosArm64 目标；视觉完全采用 GearUI 默认主题，弃用自研 token；一次性全量替换，不再分阶段验收。业务逻辑、状态、网络不动。" |

### 1.1 用户已决策（规格前提，不再讨论）

| # | 决策 | 对规格的约束 |
|---|---|---|
| D1 | macOS 走**源码引入** gearui-kit 并自补 `macosArm64` 目标（同 `sharedCompose` 做法） | 四端同源，不出现"macOS 保留旧 UI"分支 |
| D2 | 视觉**完全采用 GearUI 默认主题**，弃用 `AdminColors` / `AdminType` 等自研 token | 品牌绿 #07C160、暗色底、自研字号体系全部退场 |
| D3 | **一次性全量替换**，不分阶段验收 | 实现可分批提交，但验收一次性按"全量达成"判定；不保留长期双轨 |

### 1.2 现状基线（实测数字，可核对）

| 指标 | 数值 | 统计口径 |
|---|---|---|
| `com/xiaozhi/admin/` 总规模 | **110 个 .kt / 22,951 行** | `find … -name '*.kt'` |
| 根目录 | 85 文件 / 17,365 行 | `ls *.kt` |
| `ui/`（待退役自研组件层） | **18 文件 / 3,783 行** | `ls ui/*.kt` |
| `compose/`（早期 Compose 试点） | **7 文件 / 1,803 行** | `ls compose/*.kt` |
| 其中 **UI 表达层**（要迁移） | **61 文件 / 13,017 行** | 根 36 + `ui/` 18 + `compose/` 7 |
| 其中 **状态/逻辑层**（不动） | **49 文件 / 9,934 行** | 根 85 − 36 |
| `observable` 标量字段 | **411 处 / 35 处 observableList，分布在 50 个 .kt** | `grep -cE "by (scope\.)?observable\("` |
| 平台源集 | macosArm64Main 2,408 行 / iosMain 238 / jsMain 275 / androidMain 131 | `find … \| wc -l` |
| Kuikly 版本 | `2.29.2-2.1.21`（`client/gradle.properties`）→ 目标 `2.30.0-2.1.21` | 需与宿主渲染层版本同步 |

> 更正一条上游措辞：根目录是 **85** 个 .kt（不是 89），`com/xiaozhi/admin/` 共 110 个 .kt 与 22,951 行吻合。

---

## 2. 迁移目标与非目标

### 2.1 目标（3 个，正交）

| # | 目标 | 可判定的达成信号 |
|---|---|---|
| G1 | **UI 表达层单一化**：管理台 UI 只由 GearUI Kit 组件 + Kuikly Compose 构成，自研 `ui/` 组件层彻底退役 | `com/xiaozhi/admin/ui/` 目录不存在；admin UI 代码中 `ViewContainer<` / `vif(` / `vfor(` / `vbind(` 引用为 0 |
| G2 | **四端视觉同源**：web / Android / iOS / macOS 渲染同一套 GearUI 组件与同一份主题 | 四端编译+链接通过；四端截图同版式、同配色 |
| G3 | **迁移零行为回归**：页面可达性、导航归属、提交路径、状态语义、图标语汇与迁移前等价 | 现有 `adminCoreTests` 全绿 + 三张清单（§3）逐项核对通过 + 视觉差异仅限 §5 明确接受项 |

### 2.2 非目标（明确不动）

| 层 | 范围 | 说明 |
|---|---|---|
| 网络层 | 全部 `GET/POST/PUT/DELETE` 请求、token/基址处理、`expected_revision` 冲突语义 | 一行不改 |
| 领域模型 / 协议 | `FilesModel` / `NodeTestModels` / `PipelineFlow` / `PipelinePanels` / `PipelineWorkflow` / `ConfigSections` / `NodeTestJson` / `EchoMuseConsoleProtocol` / `EchoMuseConsoleText` / `EchoMuseWakeWord` / `TimeFormat` / `FilesModel` | 一行不改（含 ESP 固件兼容约束） |
| 业务规则 | 菜单归属（`ConfigSections.isGlobal`）、每个 section 唯一归属（`UiContractsTest`）、设备 overlay 白名单差量、OTA 上传校验、行内二次确认的"不可逆动作需二次确认"规则 | 语义不变 |
| 服务端 | `server/` 下 Rust 代码、API 形状 | 不在本期 |
| 状态**字段与语义** | 411 个 observable 的**名字、类型、默认值、联动关系** | 不改 |
| 平台原生桥 | `XiaoZhiModule` / `AdminRealtime` / 文件选择器 expect-actual | 不改 |

### 2.3 必须修正的一条"非目标"边界（重要）

> **"状态层不动" 需精确化为：状态层"字段与语义"不动，状态层"观察机制"必须改。**

依据：`compose/AdminConfigDraft.kt` 文件头已明确记录试点结论 ——

> "旧表单是单个巨型 observable 状态……**且是 Kuikly DSL 的观察系统；Compose 的快照系统读它不会触发重组**。"

因此若坚持"状态类一行不改"，Compose 页面将**读不到状态变化**（点了开关不刷新、加载完成不回显）。试点为此付出的代价是"每段一份草稿"（`AdminConfigDraft`），它带来了**第二套提交路径**——与现有"一次 `POST /api/config`""设备 overlay 差量"的既有语义直接冲突，且会造成 17 段重复请求骨架。

**规格决策（P0）**：状态类只允许改**状态容器机制**（`by scope.observable(x)` → Compose `mutableStateOf` / `mutableStateListOf`），字段名、类型、默认值、方法签名、联动逻辑零改动；**禁止**再走"每段一份草稿"的第二套提交路径。迁移完成后 `AdminConfigDraft` 及 `compose/` 全部试点代码一并删除。

---

## 3. 页面与 section 全量清单（迁移范围，逐项可核对）

> 三张主表 + 两张附表覆盖**全部 UI 落点**。行数列为该渲染入口所在文件的当前总行数（一个文件承载多项时重复出现，属正常）。
> "菜单"列取值：`观测` / `配置` / `存储` 为侧边栏分组；`语音流程` / `全局配置` 为两个配置一级菜单的 section 归属。

### 3.1 一级导航页（`AdminNav.all`，12 项；web 可见 11 项）

| # | id | 菜单 | 分组 | 端 | 渲染入口（文件:行） | 文件行数 | 备注 |
|---|---|---|---|---|---|---|---|
| 1 | `overview` | 概览 | 观测 | 全端 | `renderOpsOverview` · OpsPages.kt:72 | 493 | 与配置 section `overview`（能力总览）**不是同一页**，勿混 |
| 2 | `testbench` | 测试台 | 观测 | **仅 macOS** | `renderTestbenchSection`（expect）· AdminPlatform.kt:52 → actual 在 macosArm64Main/AdminPlatform.kt | 87 / 478 | 见 §3.5 |
| 3 | `devices` | 设备 | 观测 | 全端 | `renderOpsDevices` · OpsPages.kt:128 | 493 | 「个性化配置」入口跳 `config` + 设备 overlay |
| 4 | `echomuse` | EchoMuse | 观测 | 全端 | `renderEchoMuse` · EchoMusePage.kt:28 | 82 | 六 tab 见 §3.4 |
| 5 | `sessions` | 会话 | 观测 | 全端 | `renderOpsSessions` · OpsPages.kt:239 | 493 | |
| 6 | `conversations` | 对话记录 | 观测 | 全端 | `renderOpsConversations` · OpsPages.kt:292 | 493 | 含 `conversationTurn` 轮次渲染 |
| 7 | `usage` | 用量 | 观测 | 全端 | `renderOpsUsage` · OpsPages.kt:394 | 493 | |
| 8 | `metrics` | 延迟 | 观测 | 全端 | `renderOpsMetrics` · OpsPages.kt:457 | 493 | |
| 9 | `console` | 控制台 | 观测 | 全端 | `renderConsole` · ConsolePage.kt:19 | 99 | 深色终端块，见 §5 保留项 |
| 10 | `config` | **语音流程** | 配置 | 全端 | `pipelineWorkflowHeader` · PipelineWorkflowView.kt:18 + `renderConfigLayout` · ConfigTabs.kt:13 | 145 / 123 | 轨道 + 配置/测试双 Tab + 左菜单；10 个 section 见 §3.2 |
| 11 | `settings` | **全局配置** | 配置 | 全端 | `renderGlobalConfigLayout` · ConfigWorkspaceViews.kt:116 | 203 | 无轨道无 Tab；7 个 section 见 §3.3 |
| 12 | `files` | 文件 | 存储 | 全端 | `renderFilesSection` · FilesPage.kt:26 | 237 | 上传 / 新建目录 / 删除 / 面包屑 |

**页面入口 `@Page` 共 3 个**

| 页面 | 注解 | 文件 | 行数 | 迁移处置 |
|---|---|---|---|---|
| 主壳 | `@Page("admin")` | AdminPage.kt | 511 | **原地改写为 ComposeContainer**，页面 id 不变 |
| 单页兜底 | `@Page("config")` | ConfigPage.kt | 146 | 保留页面 id，壳改为 GearUI（见 Q2） |
| Compose 试点 | `@Page("admin-compose")` | compose/AdminComposeShell.kt | 237 | **迁移完成后删除** |

### 3.2 「语音流程」菜单 section（10 项，`PipelineFlow.primaryForSection != null`）

| # | section id | 菜单文案 | 渲染函数 | 文件:行 | 文件行数 | 私有子块 |
|---|---|---|---|---|---|---|
| 1 | `vad` | VAD · 语音分段 | `vadConfigCard` | ConfigStageCards.kt:57 | 393 | |
| 2 | `asr` | ASR · 语音识别 | `asrConfigCard` | ConfigStageCards.kt:18 | 393 | |
| 3 | `nlu` | NLU · 意图与技能 | `nluConfigCard` | ConfigNluCard.kt:24 | 269 | `nluBuildModelSection` / `nluIntentList` / `nluIntentBlock` / `nluSlotList` / `nluSlotBlock`（5 个） |
| 4 | `llm` | LLM · 对话生成 | `llmConfigCard` | ConfigStageCards.kt:319 | 393 | |
| 5 | `tts` | TTS · 语音合成 | `ttsConfigCard` | ConfigStageCards.kt:83 | 393 | |
| 6 | `command` | 结束与指令 | `commandConfigCard` | ConfigCommandCard.kt:20 | 96 | |
| 7 | `wakeup` | 唤醒与问候 | `wakeupConfigCard` | ConfigWakeupCard.kt:19 | 74 | |
| 8 | `soul` | 人格 | `soulConfigCard` | ConfigSoulMemoryCards.kt:31 | 476 | |
| 9 | `memory` | 记忆 | `memoryConfigCard` | ConfigSoulMemoryCards.kt:226 | 476 | |
| 10 | `audio` | 音频与播放 | `audioConfigCard` + `playbackConfigCard` | ConfigInfrastructureCards.kt:52 + ConfigPlaybackCard.kt:9 | 195 / 43 | |

**工作区横切（服务 10 个 section，必须一起迁移）**

| 组件 | 渲染入口 | 文件:行 | 文件行数 |
|---|---|---|---|
| 流程轨道 | `pipelineWorkflowHeader`（含 `workflowStage` / `workflowConnection`） | PipelineWorkflowView.kt:18/71/111 | 145 |
| 配置/测试 Tab | `renderConfigLayout` + `nodeWorkspaceTabs` | ConfigTabs.kt:13/62 | 123 |
| 工作区左菜单 | `workspaceMenu` + `workspaceMenuItem` | ConfigWorkspaceViews.kt:36/79 | 203 |
| 节点运行状态 | `workspaceNodeStatus` | ConfigWorkspaceViews.kt:180 | 203 |
| 节点测试面板 | `nodeTestPanel` + `nodeTestAction` | NodeTestPanel.kt:17/57 | 96 |
| section 分发 + 横幅 | `renderConfigSection` + `sectionReadinessBanner` + `editTargetBanner` + `advancedToggleRow` | ConfigCards.kt:74/39/183/148 | 263 |

### 3.3 「全局配置」菜单 section（7 项，`ConfigSections.global`）

| # | section id | 菜单文案 | 渲染函数 | 文件:行 | 文件行数 | 备注 |
|---|---|---|---|---|---|---|
| 1 | `overview` | 能力总览 | `overviewCard` + `engineHealthCard` | ConfigOverviewCard.kt:26 + EngineHealthCard.kt:107 | 142 / 274 | 健康横幅 `healthBanner`（EngineHealthCard.kt:33） |
| 2 | `presets` | 预设 | `presetCard` | ConfigCards.kt:211 | 263 | 设备 overlay 模式下只显示禁用说明 |
| 3 | `aiui` | AIUI 全链路模式 | `aiuiConfigCard` | ConfigInfrastructureCards.kt:126 | 195 | capabilityId = `fullchain` |
| 4 | `server` | 服务 | `serverConfigCard` | ConfigInfrastructureCards.kt:14 | 195 | |
| 5 | `echomuse` | EchoMuse 接入 | `echomuseConfigCard` | ConfigEchoMuseCard.kt:15 | 45 | 与侧边栏 `echomuse` 页不是同一处 |
| 6 | `mcp` | MCP 工具 | `mcpToolsCard` | McpToolsCard.kt:30 | 181 | 含 `deviceBlock` / `toolBlock` |
| 7 | `ota` | 固件与 OTA | `otaCard` | OtaCard.kt:29 | 164 | 含 `otaEntryBlock`；上传/删除确认 |

> 校验口径：`ConfigSections.all` 共 **17** 项 = 语音流程 10 + 全局配置 7，且 `UiContractsTest.everySectionBelongsToExactlyOneMenu` 已锁死"每项唯一归属"。**迁移后此不变量必须保持**。

### 3.4 EchoMuse 页内部（1 审批页 + 6 tab）

| # | 子视图 | 渲染入口 | 文件:行 | 文件行数 |
|---|---|---|---|---|
| 0 | 未批准 · 审批单页 | `echoMuseApprovalPage` | EchoMuseDetailTabs.kt:130 | 169 |
| 1 | 状态 tab | `echoMuseStatusTab`（含 `echoMuseDeviceHead` / `echoMuseDeviceFacts` / `echoMuseResources` / `emStatBar` / `emStatTile` / `echoMuseVolume` / `echoMuseBluetoothSummary` / `echoMuseDiagnostics`） | EchoMuseStatusTab.kt:74 | 543 |
| 2 | 活动 tab | `echoMuseActivityTab` | EchoMuseActivityTab.kt:28 | 86 |
| 3 | 配置 tab | `echoMuseConfigTab` + `echoMuseConnectivity` + `echoMuseConfigSubmit` | EchoMuseConfigTab.kt:35/66/178 | 209 |
| 4 | 配置 · 8 个 stage | `stagePlayback` / `stageWakeWord` / `stageMicrophones` / `stageRing` / `stageAdvanced` / `stageBluetooth` / `stageSendspin` / `stageMusic` | EchoMuseConfigStages.kt:89/154/298/342/377/418/437/463 | 470 |
| 5 | 控制台 tab | `echoMuseConsoleTab` + `terminalFrame` / `terminalTitleBar` / `terminalKey` / `terminalOutput` / `terminalPrompt` / `terminalStatusLine` | EchoMuseConsoleTab.kt:58/78/106/140/171/211/245 | 270 |
| 6 | 更新 tab | `echoMuseUpdatesTab` + `echoMuseVersions` / `echoMuseAssets` / `echoMuseMaintenance` | EchoMuseUpdatesTab.kt:38/57/90/169 | 218 |
| 7 | 日志 tab | `echoMuseLogsTab` | EchoMuseLogsTab.kt:24 | 89 |
| 附 | 设备卡外壳 | `echoMuseDeviceCard` + `echoMuseCardHeader` / `echoMuseStatusMeta` / `echoMuseDeleteButton` / `echoMuseDeleteConfirm` | EchoMuseDeviceCard.kt:33/57/66/95/107 | 139 |
| 附 | BLE 面板 | `echoMuseBlePanel` + `bleSettings` | EchoMuseBleCard.kt:13/132 | 148 |
| 附 | 音乐/播放 | `echoMuseMusicStage` | EchoMuseMusicPanel.kt:24 | 113 |
| 附 | tab 容器 | `echoMuseDetailTabs` + `echoMuseTabFeedback` | EchoMuseDetailTabs.kt:59/106 | 169 |

### 3.5 macOS 专属 UI（必须一起迁，否则 macOS 端编译不过）

| 项 | 文件 | 行数 | 说明 |
|---|---|---|---|
| 测试台三段区 | macosArm64Main/AdminPlatform.kt（`renderTestbenchSection` actual） | 478 | 含 `macServerSwitchEntry`（侧栏"切换服务器"内联表单） |
| 测试台页面 | macosArm64Main/TestBenchPages.kt | 648 | **已超 600 行**，迁移时须拆分 |
| NLU 测试页 | macosArm64Main/TestBenchNluPage.kt | 139 | |
| 平台声明 | AdminPlatform.kt（commonMain，expect） | 87 | expect 签名可保留，实现换 GearUI |
| 字体 actual | AdminFonts.kt（4 个源集各 7~18 行） | 18 | 若弃用自研字号体系则一并退役 |

### 3.6 自研组件层（`ui/`，18 文件 3,783 行 —— 全部退役）

| 文件 | 行数 | 对外符号（迁移覆盖面） |
|---|---|---|
| Theme.kt | 214 | `AdminToken`(59 常量) / `AdminColors`(40) / `AdminShape`(4) / `AdminType`(7) / `AdminSpace`(8) |
| Button.kt | 184 | `ButtonVariant` / `appButton` / `primaryButton` / `secondaryButton` / `textButton` / `outlineButton` |
| Form.kt | 432 | `fieldBox` / `labelWithHelp` / `fieldLabel` / `labeledField` / `labeledIntField` / `labeledDecimalField` / `labeledTextArea` / `actionRow` / `switchRow` / `terminalInput` / `INT_INPUT_FILTER` / `DECIMAL_INPUT_FILTER` |
| Card.kt | 194 | `sectionTitle` / `textNote`×2 / `cardSurface` / `splitCard` / `groupedCard` / `dialogSurface` / `insetPanel` / `skeletonCard` |
| Layout.kt | 419 | `dividerH` / `dividerV` / `hGap` / `vGap` / `splitRow` / `splitColumn` / `SplitScope` / `menuPaneRow` / `cardRow` / `twoPane` / `pageScroller` |
| Widgets.kt | 382 | `dropdownField` / `waveformPlayer` / `dot` / `statusBadge` / `pluginStatusBadge` / `largeTitleBar` |
| IconVectors.kt | 342 | `IconLayer` / `IconOp` / `SvgPathParser`（26 个图标的几何数据） |
| Tabs.kt | 232 | `TabUiState` / `TabPage` / `tabBar` / `tabbedPanel` |
| Choice.kt | 198 | `ChoiceOption` / `buttonGroup` / `segmentedChoice` / `overrideChoiceRow` / `checkBoxRow` / `choiceMenu` |
| Rows.kt | 213 | `infoRow`×2 / `metricRow` / `captionLine`×2 / `detailLine`×2 / `groupLabel` / `outputBlock` |
| Icons.kt | 157 | `AdminIconVectors` / `IconToken.vectors()` / `appIcon` |
| Menu.kt | 154 | `menuListItem` / `MenuItemStyle`(MAIN/WORKSPACE/CARD) / `MenuItemSpec` |
| Nav.kt | 133 | `NavDescriptor` / `AdminNav`（**导航表要保留**，见下）/ `connectionLabel` / `sidebarItem` |
| Header.kt | 130 | `headerRow`×2 / `HeaderTone` / `headerTitle` |
| FieldHelp.kt | 140 | `FieldHelpState` / `FieldHelpHost` / `infoBadge` / `fieldHelpDialog` |
| Contracts.kt | 99 | `Availability` / `ActionState` / `ActionOutcome` / `canInvokeAction` / `CheckState` / `OverrideChoice` / `IconToken` / `NodeRequestTicket` / `NodeRequestGate` |
| ActivityIndicator.kt | 60 | `activityIndicator` |
| LedRing.kt | 100 | `ledRing`（Dot 2 12 段光环） |

**两个必须保留到新包里的符号（否则构建工具链直接断）**

| 符号 | 当前路径 | 谁在依赖 | 处置 |
|---|---|---|---|
| `IconToken`（26 个枚举） | `ui/Contracts.kt:49` | `shared/build.gradle.kts` 的 `verifyAdminIcons` 任务（硬编码读该文件）；`adminCoreTests` 的 `kotlin.include("com/xiaozhi/admin/ui/Contracts.kt")` | 迁到 `com/xiaozhi/admin/contracts/UiContracts.kt`，**同步改两处路径** |
| `IconToken → AdminIconVectors` 映射（1:1） | `ui/Icons.kt` | 同上，`verifyAdminIcons` 硬编码读该文件并要求含 `Canvas(` | 迁到新路径并保持"1:1 + Canvas 渲染 + 无 bitmap 回退"三条断言 |

> 这是本迁移**最容易漏的隐性耦合**：只删 `ui/` 不改这两个文件，`:apps:h5App:publishWeb` 与 `:adminCoreTests:jvmTest` 会直接失败。

---

## 4. 组件映射规范表

### 4.1 自研组件 → GearUI 对应物

| # | 自研组件（`ui/`） | GearUI 组件 | 迁移要点 |
|---|---|---|---|
| 1 | `AdminColors` / `AdminToken`(59) | **删除** | 改用 GearUI 主题语义色；状态色走组件 `type`/`color` 语义（见 §5.3） |
| 2 | `AdminType`(7 档 + `scale`) | **删除** | 改用 `Text` 的字号档；视口缩放见 Q1 |
| 3 | `AdminShape`(4) | **删除** | 用主题 shape（button/card 圆角由组件给） |
| 4 | `AdminSpace`(8) | 保留为**布局常量**（改名 `GearUiSpace`） | 纯数值间距，非视觉 token；保留可避免布局尺寸漂移 |
| 5 | `appButton` / `primaryButton` / `secondaryButton` / `textButton` / `outlineButton` | **`Button`** | `ButtonVariant.PRIMARY→Button(type=primary)`；`SECONDARY/OUTLINE→default`；`TEXT→text`；`danger()→danger 色`；`loading/result` 态用 `Button` 的 loading + `Toast` 反馈（不再自绘"✓已保存"文字切换，见 Q4） |
| 6 | `largeTitleBar` | **`NavBar`** | `title→NavBar.title`；`leading`（汉堡）→左侧 `Icon(Menu)`；`trailing`→右侧 actions |
| 7 | `sidebarItem` / `menuListItem` / `MenuItemStyle` | **`List` + `Cell`**（+`CellGroup` 分组标题） | 选中态、图标、右侧 dirty 点 → `Cell` 的 `selected` / `leading icon` / `trailing slot`；三档尺寸（44/36/min44）在迁移时**合并为 GearUI 统一行高**（接受项，见 §5.2-A5） |
| 8 | `groupLabel` | `CellGroup` 的 group header | |
| 9 | `tabBar` / `tabbedPanel` / `TabUiState` / `TabPage` | **`Tabs`**（长列表用 `Segmented`） | EchoMuse 六 tab 有"44pt 最小宽 × 6 = 264"算术约束，须验证 GearUI `Tabs` 是否等宽可滚动（Q6） |
| 10 | `cardSurface` / `groupedCard` / `splitCard` / `insetPanel` | **`Card`** + `CellGroup` | `groupedCard(title)` → `CellGroup(title)` + 内嵌 `Card` |
| 11 | `dialogSurface` / `fieldHelpDialog` / `infoBadge` | **`Dialog`**（确认类）/ **`Popover`**（字段帮助弹层） | ⓘ 帮助弹层需"点外部关闭 + 不阻塞滚动"，`Popover` 优先，窄屏回落 `Popup` |
| 12 | `labeledField` / `labeledIntField` / `labeledDecimalField` | **`Form` + `Input` / `NumberField`** | 数字过滤（`INT_INPUT_FILTER`/`DECIMAL_INPUT_FILTER`）是**业务校验**，必须保留为新组件的输入过滤器 |
| 13 | `labeledTextArea` | **`Form` + `Textarea`** | 提示词/人格文本 |
| 14 | `fieldBox` / `fieldLabel` / `labelWithHelp` | `Form` 的 field 包装 + `Input` 的 label/help props | 不再自绘边框/内边距 |
| 15 | `switchRow` | **`Switch`**（配 `Cell` 行） | |
| 16 | `checkBoxRow` / `CheckState`(3 态) | **`Checkbox`** | `Indeterminate` 必须存在，否则"混合选择"语义丢失（Q5） |
| 17 | `overrideChoiceRow` / `OverrideChoice`(Inherit/ForceOn/ForceOff) | **`Radio`**（3 项） | `null=继承` 的语义不可退化为 unchecked（`UiContractsTest` 已锁） |
| 18 | `segmentedChoice` / `buttonGroup` / `ChoiceOption` | **`Segmented`** | 配置/测试的 Tab、性别切换等 |
| 19 | `choiceMenu` / `dropdownField` | **`Select`**（长列表用 `Picker` / `ListBox`） | 音色/模型列表是 `ObservableList`（随性别增删），需确认 GearUI 选项源可响应式刷新 |
| 20 | `statusBadge` / `pluginStatusBadge` | **`Tag`** | 四态语义 → Tag 的 success/danger/warning/neutral；"正常不打标"规则保留 |
| 21 | `dot` | **`Badge`(dot)** | dirty 橙点、菜单警告点 |
| 22 | `skeletonCard` / `opsSkeleton` | **`Skeleton`** | |
| 23 | `activityIndicator` | **`Loading`** | 当前用 ComposeView 内嵌，迁移后原生 `Loading` |
| 24 | `ToastHost` / `ToastState` | **`Toast`** | 3 秒自动消失、层级最高、绝对定位不占高度 —— 语义保留 |
| 25 | `Empty` 相关文案块（各页 textNote 空态） | **`Empty`** | 空态统一，文案不动 |
| 26 | `outputBlock` / `terminalInput` / `terminalOutput` | **`Code`**（+ 自定义深色容器） | 终端配色**必须保留深色**（§5.3-保留项），`Code` 只做排版 |
| 27 | `pageScroller` / `splitRow` / `splitColumn` / `split` / `SplitScope` / `menuPaneRow` / `cardRow` / `twoPane` | Compose **`Row` / `Column` / `Box` + `LazyColumn`/`verticalScroll`**（无 GearUI 对应物，属布局原语） | `menuPaneRow`（定宽菜单 + 1px 竖线 + 面板）→ `Row{ Box(width) ; Divider(vertical) ; Box(weight) }`；响应式断点逻辑（768 / 900 / 700）**逐条沿用** |
| 28 | `hGap` / `vGap` | `Spacer(size)` | |
| 29 | `dividerH` / `dividerV` | **`Divider`** | |
| 30 | `appIcon` / `IconToken` / `IconVectors` / `AdminIconVectors` | **`Icon`**（→ 见 §4.2 图标策略） | 26 个 token 一个不能少 |
| 31 | `infoRow` / `metricRow` / `captionLine` / `detailLine` / `headerRow` / `headerTitle` / `sectionTitle` | **`Cell` + `Text`**（表格型用 **`Table`**） | 运维列表（会话/对话/用量/延迟）改 `Table` 后列对齐更规整，属接受项 |
| 32 | `waveformPlayer` | **无对应物**：`Slider`（进度）+ 自绘波形（Compose `Canvas`） | 语音试听/录音回放，波形是功能可读性的一部分 |
| 33 | `ledRing` | **无对应物**：自绘 `Canvas` | Dot 2 光环 12 段 + 中央点，短动画 |
| 34 | `NodeRequestGate` / `Availability` / `ActionState` / `ActionOutcome` / `canInvokeAction` | **纯契约，原样保留**（换包路径） | 防重复请求与旧结果计时器，**不得因 UI 迁移而弱化** |

### 4.2 图标策略（26 个 `IconToken` 的覆盖度）

`IconToken`：`Save, Delete, Menu, ChevronDown, ChevronUp, ChevronRight, Play, Stop, Refresh, Check, Close, Settings, Microphone, Waveform, Text, Intent, Model, Speaker, Info, Wifi, Bluetooth, Power, Copy, Funnel, Folder, Document`

| 判定 | 处理 |
|---|---|
| GearUI `Icon` 已覆盖 | 直接用 `Icon(name)`，一对一 |
| GearUI 未覆盖 | 保留 `AdminIconVectors` 的矢量几何，作为 `Icon` 的 **slot/自定义 path 内容**渲染（`Canvas` 契约不变） |
| **禁止** | 换成 emoji、字符图标（`▾`/`✓`）、或"看起来差不多"的近似图标 —— `verifyAdminIcons` 任务就是为锁这条而存在的 |

### 4.3 无对应物 → GearUI 组合实现（必须给组合方案，不留"自绘一整套"）

| 需求 | GearUI 组合 |
|---|---|
| 流程轨道（VAD→ASR→NLU→LLM→TTS + 连接点分支） | **`Steps`**（横向）+ 分支节点用 `Cell` 挂在 stage 之间；节点异常态 → `Steps` 的 error 态或 `Tag` |
| 概览磁贴（设备数/在线数/延迟等） | **`Grid`** + **`Card`** |
| 运维列表（会话/对话/用量/延迟） | **`List`** / **`Table`**；行详情 **`Collapse`**；轮次 **`Timeline`** |
| 资源占用（EchoMuse 内存/存储条） | **`Progress`** / **`Meter`** |
| 文件页目录树 + 面包屑 | **`List`+`Cell`** + `Steps`/`Breadcrumb` 风格行；删除 → **`SwipeCell`** + `Dialog` 二次确认 |
| OTA 固件上传 / 文件上传 | **`Upload`**（宿主适配见 R6）+ **`Progress`** |
| 文件/图片预览 | **`Image`** + **`ImageViewer`** |
| 删除类不可逆操作 | **`Dialog`**（二次确认），语义同现状行内 `confirmKey`：不可点外部关闭、默认焦点在"取消" |
| 加载失败横幅（`loadError`） | **`Alert`** / **`NoticeBar`**（`type=error`），文案保留"怎么修"式提示 |
| 节点测试结果 | **`Result`** |
| 长页回到顶部 | **`BackTop`** |
| 下拉刷新 / 加载更多 | **`PullRefresh`** / **`LoadMore`** |
| 滚动边界阴影 | **`ScrollShadow`** |
| 设备身份（EchoMuse 设备行） | **`Avatar`** + **`User`** |
| 窄屏抽屉 | **`Drawer`**（含遮罩/关闭），替代自研 `overlay + absolutePosition` 浮层 |
| 快捷键/命令提示 | **`Kbd`**（可选，P2） |

### 4.4 明确"本期不用"的 GearUI 组件（避免过度设计）

`Calendar` / `DatePicker` / `ColorPicker` / `Cascader` / `Rate` / `Swiper` / `IndexBar` / `Tour` / `Watermark` / `InputOTP` / `AgreementCheckbox` / `InputGroup` / `Skeleton`(已有) / `Tooltip`(用 Popover) / `ActionSheet` / `ContextMenu` / `Notification` / `Snackbar` / `BottomSheet` / `Toolbar`

> 理由：现有管理台无对应交互，引入即增面积与回归面。若实现中发现确需（如 `ActionSheet` 替代某处菜单），须回到本表登记后再用。

---

## 5. 视觉差异与接受项

### 5.1 总体变化（一次性发生，不可逆）

| 维度 | 现状（自研暗色） | GearUI 默认 | 结论 |
|---|---|---|---|
| 明暗 | 暗色：窗口/侧栏 `#1C1C1E`，卡片/输入 `#2C2C2E` | iOS 浅色：系统分组灰底 + 白卡 | **接受**（D2） |
| 品牌色 | `#07C160` 主按钮 / `#4CD964` 选中与链接 / 焦点环 `#33 07C160` | 主题 primary（非品牌绿） | **接受**：品牌识别消失 |
| 文字 | `#F5F5F7` 主 / `#AEAEB2` 次 / `#8E8E93` 弱 | 主题 on-surface 系列 | **接受** |
| 分隔线 | `#38383A` / `#48484A` | 主题 outline | **接受** |
| 圆角 | 8 / 10 / 12 / pill | 主题 shape | **接受** |
| 字号 | `AdminType` 7 档 × 视口缩放 1.0~1.5 | 主题固定档 | **待决**（Q1） |
| 侧栏选中 | 亮灰胶囊 + 3px 绿竖条 + 绿字 | `Cell` selected（主题色） | **接受**，但"当前项必须一眼可辨"是 P0 |
| 卡片观感 | 自绘阴影 `#40000000` | 主题 elevation | **接受** |
| 行高 | 菜单 3 档（44/36/min44） | GearUI 统一行高 | **接受**（视觉略有变化，可读性不降） |

### 5.2 明确接受的视觉差异（逐条签字，不再申诉）

| # | 差异 | 为什么接受 |
|---|---|---|
| A1 | 暗色 → 浅色，夜间长时间使用体验变化 | D2 已决策；GearUI 默认主题即产品观感 |
| A2 | 品牌绿从主按钮/选中态/链接上消失 | 同上 |
| A3 | 自研 7 档字号与视口缩放退场，字阶由主题决定 | 避免"主题 + 自研 token"两套字号打架 |
| A4 | 侧栏/菜单的 3px 选中竖条消失 | `Cell` 选中态替代，信息未丢 |
| A5 | 三档菜单行高合并为一档 | 减少 3 套 spec 维护成本 |
| A6 | 自绘卡片阴影/边框改为主题 elevation | 视觉更统一 |
| A7 | 运维列表由自绘行变 `Table`/`List` | 列对齐更规整 |
| A8 | 按钮的"✓ 已保存 / ✕ 保存失败"三态文案切换改为 Toast 反馈 | 避免自绘按钮态与 GearUI 组件冲突；**但保存成功/失败必须有明确反馈**（P0） |

### 5.3 必须保留的视觉/语义（P0，不接受任何"主题化"改写）

| # | 保留项 | 现状证据 | 保留要求 |
|---|---|---|---|
| K1 | **26 个图标语汇** | `ui/Contracts.kt` IconToken + `ui/Icons.kt` 映射 + `verifyAdminIcons` 任务 | 一个不少、几何不变、Canvas 渲染、无 bitmap 回退 |
| K2 | **终端深色块** | `AdminToken.TERM_BG #1C1F18` / `TERM_FG #C8D4B0` / `TERM_DIM #6E7A5E` / `TERM_ACCENT #9ABA80` | 控制台页与 EchoMuse 控制台 tab **保持深色**（"这是另一个世界"的视觉隐喻 + 长读舒适度）。浅色化会让 ANSI 日志不可读 |
| K3 | **状态四色语义** | `statusBadge`：`connected`→绿、`recording`/`error`→红、`busy`→黄、`idle`→灰 | 四态颜色语义不变；"正常不打标"（`active` 不渲染徽标）规则不变 |
| K4 | **危险操作红** | 删除设备/固件/文件/预设、`danger` 按钮系列 | 不可逆操作必须红色按钮 + 二次确认弹层 |
| K5 | **降级/异常黄红** | `degraded`→黄、`failed`→红（就绪度横幅 + 工作区菜单警告点） | 保留 |
| K6 | **dirty 未保存提示** | 侧栏 `config`/`settings` 项的橙点（`warning`，`showDot`） | 保留（`Badge` dot） |
| K7 | **LED 环 12 段色环 + 中央点** | `ui/LedRing.kt`（绿监听/蓝说话/黄思考/红静音/橙离线） | 自绘保留，颜色语义不变 |
| K8 | **音频波形** | `waveformPlayer` | 自绘保留（波形是录音/回放的可读性载体） |
| K9 | **响应式断点** | `NARROW_BREAKPOINT=768`、`twoColumn≥900`、`workspaceIsCompact<700`、侧栏 240 | 数值与行为不变（抽屉/常驻、双列/单列、紧凑菜单） |
| K10 | **状态不丢的重建口径** | 切 section / 切 tab / 窗口跨断点不重建标题栏与当前 section | 迁移后同样成立（Compose 重组须避免身份丢失） |

> 更正一条上游措辞：代码中**不存在**"红涨绿跌"式涨跌配色（全仓 `涨/跌/trend` 匹配 0 处）。实际的状态色语义即 K3 的四态 + K4 危险红 + K5 降级黄。因此无该类冲突需要保留。

### 5.4 迁移期视觉验收方式

- 四端各截 6 张图：概览 / 设备 / 语音流程（含轨道 + 双 Tab + 左菜单）/ 全局配置 / EchoMuse（状态 + 控制台）/ 文件。
- 逐张对照 §5.2（应变化）与 §5.3（不应变化）两表勾选。
- 「不应变化」项发现任一缺失 → 视为缺陷，不进验收。

---

## 6. 需求池（P0 / P1 / P2）

### P0（Must have —— 缺任一项不算迁移完成）

| # | 需求 | 度量 |
|---|---|---|
| P0-1 | gearui-kit **源码引入**并补齐 `android / iosArm64 / iosSimulatorArm64 / js / macosArm64` 五个目标，四端可编译 | 五目标编译通过；macOS 无"缺 macosArm64"解析失败 |
| P0-2 | Kuikly 升级 `2.29.2-2.1.21 → 2.30.0-2.1.21`，与宿主渲染层版本同步 | `gradle.properties` + 根 buildscript 双写一致；四端运行时不再报版本不匹配 |
| P0-3 | 状态观察机制改造：411 个 `observable` + 35 个 `observableList` 迁到 Compose 快照 | 页面交互全部可响应（开关即时回显、加载态即时消失）；**字段/默认值/方法签名零改动** |
| P0-4 | **禁止**第二套提交路径：删除 `compose/AdminConfigDraft` 与逐段草稿 | `AdminConfigDraft` 不存在；`POST /api/config` 提交点唯一 |
| P0-5 | 12 个一级导航页 + 17 个配置 section + EchoMuse 7 视图 + 文件页 + 测试台**全部**迁移完成 | §3 清单逐行核对，无"迁移占位/回到旧版"残留 |
| P0-6 | `com/xiaozhi/admin/ui/` 全目录退役；`IconToken` 与图标映射迁至新包 | 目录不存在；`verifyAdminIcons` 与 `adminCoreTests` 路径同步更新并通过 |
| P0-7 | §5.3 十条保留项全部达成 | 截图逐项勾选 |
| P0-8 | 现有测试全绿 | `UiContractsTest` 7 个用例（含 `everySectionBelongsToExactlyOneMenu`、`webHidesLocalHardwareButKeepsSharedTests`、`pipelineOrderAndCapabilityBindingsAreExplicit`） |
| P0-9 | macOS Catalyst 链接通过 | `linkDebugFrameworkMacosArm64` + `patchFrameworkForCatalyst` + macosApp 链接全绿 |
| P0-10 | 无 600 行超限文件 | `com/xiaozhi/admin/**` 全部 ≤ 600（含现状已超限的 `ConfigContextState.kt` 607 与 `TestBenchPages.kt` 648） |

### P1（Should have）

| # | 需求 |
|---|---|
| P1-1 | `AdminSpace` 以 `GearUiSpace` 名义保留，间距数值等价（防布局漂移） |
| P1-2 | 运维列表改用 `Table`，列头对齐 + 数值右对齐 |
| P1-3 | 长页（配置 / 文件）接入 `BackTop`、刷新接入 `PullRefresh`/`LoadMore` |
| P1-4 | 删除类操作统一 `Dialog` 二次确认（替代行内 `confirmKey`），确认弹层不可点外部关闭 |
| P1-5 | `Drawer` 替换自研抽屉浮层，含焦点陷阱与 Esc 关闭（web/桌面） |
| P1-6 | 迁移后产物体积与首屏时间数据入档（web bundle / macOS framework / Android APK） |

### P2（Nice to have）

| # | 需求 |
|---|---|
| P2-1 | GearUI 暗色主题适配开关（用户可切回暗色）—— 本期不做，仅记录扩展点 |
| P2-2 | `Kbd` 快捷键提示（控制台/终端） |
| P2-3 | 概览磁贴可拖拽排序 |
| P2-4 | 图标集改为 GearUI 原生图标名（若覆盖度满足且 `verifyAdminIcons` 可放宽） |

---

## 7. 验收标准（可执行、可验证）

> 每条都有明确命令或可判定状态。全部通过 = 迁移完成；任一失败 = 未完成。

| # | 验收项 | 验证方式 | 通过判据 |
|---|---|---|---|
| E1 | 四端编译 | `./gradlew :shared:compileKotlinMetadata` + `:shared:assembleDebug`（Android） | BUILD SUCCESSFUL，0 warning-as-error |
| E2 | web 产物 | `./gradlew :apps:h5App:publishWeb`（内含 `:shared:packLocalJSBundleRelease` + `verifyAdminIcons`） | 成功；`apps/h5App/web/` 下有 `nativevue2.js` 与新 `nativevue2.zip` |
| E3 | iOS 链接 | `./gradlew :shared:linkDebugFrameworkIosArm64 :shared:linkDebugFrameworkIosSimulatorArm64` | 成功；framework 可被 `apps/iosApp` 链接 |
| E4 | macOS 链接 + Catalyst | `./gradlew :shared:linkDebugFrameworkMacosArm64`（自动触发 `patchFrameworkForCatalyst`）→ `apps/macosApp` Xcode 构建 | 链接成功，无 `building for 'macCatalyst', but linking … built for 'macOS'` |
| E5 | 契约测试 | `./gradlew :adminCoreTests:jvmTest` | 全绿（含 `UiContractsTest` 7 例 + `FilesModelTest` / `PipelineFlowTest` / `TimeFormatTest` 等）；0 failures |
| E6 | 图标护栏 | `./gradlew :shared:verifyAdminIcons` | `Verified 26 embedded vector icons`；且仍校验 1:1 映射 + `Canvas(` + 无 bitmap |
| E7 | `ui/` 清零 | `ls client/shared/src/commonMain/kotlin/com/xiaozhi/admin/ui/` | 目录不存在（或空目录已删） |
| E8 | 旧包引用清零 | `grep -r "com.xiaozhi.admin.ui" client/` | 0 命中（`verifyAdminIcons`/`adminCoreTests` 已指向新包） |
| E9 | 传统 DSL 清零 | `grep -rE "ViewContainer<\|vif\(\|vfor\(\|vbind\(\|core\.views\.View" client/shared/src/commonMain/kotlin/com/xiaozhi/admin/` | 0 命中 |
| E10 | 自研 token 清零 | `grep -rE "AdminColors\|AdminType\|AdminToken\|AdminShape" client/shared/src/` | 0 命中（`GearUiSpace` 除外） |
| E11 | 试点代码清除 | `ls client/shared/src/commonMain/kotlin/com/xiaozhi/admin/compose/` 与 `@Page("admin-compose")` | 目录不存在；页面 id 不再注册 |
| E12 | 页面注册 | 检查 KSP 生成的 `KuiklyCoreEntry`（各目标） | 含 `admin`（与保留的 `config`）；不含 `admin-compose` |
| E13 | 导航不变量 | `UiContractsTest` + 手工核对 §3.1 | 12 项导航（web 11 项，`testbench` 仅 macOS）；17 section 归属不变 |
| E14 | 文件行数 | `find client/shared/src/commonMain/kotlin/com/xiaozhi/admin -name '*.kt' -exec wc -l {} +` | 全部 ≤ 600 |
| E15 | 视觉保留项 | 四端 6 类页面截图 + §5.3 勾选表 | K1~K10 全通过 |
| E16 | 行为等价抽查（人工，四端各一轮） | ①保存配置 → 成功 Toast；②触发 409 → 「重新加载」可用；③设备个性化 → 编辑对象横幅 + 「返回全局/恢复全局配置」；④切换 section/跨断点 → 当前 section 不丢；⑤删除固件 → 二次确认 → 列表刷新 | 全部符合 |
| E17 | 产物体积 | 记录 web `nativevue2.js`（原始 + gzip）、macOS `shared.framework`、Android AAR 体积，与迁移前对比 | 数据入档；gzip 增幅 > 50% 需上报主理人复核（不设硬门槛，但不得静默） |

---

## 8. 风险与待确认问题

### 8.1 风险（按影响排序）

| # | 风险 | 影响 | 应对 |
|---|---|---|---|
| R1 | **状态层观察机制不兼容**：Kuikly `observable` 不被 Compose 快照追踪（试点已实证） | 页面"点了不刷新"，是最可能被低估的工作量（411 + 35 处） | P0-3 单独立项；先做一个 section 端到端验证状态机制，再批量铺开 |
| R2 | **GearUI 无 macosArm64 制品** | macOS 端无法解析依赖 | D1 已定：源码引入 + 自补目标（同 `sharedCompose`），并补 `THIRD_PARTY_NOTICES` 与 Apache-2.0 声明 |
| R3 | **两个硬编码路径耦合**：`verifyAdminIcons` 读 `ui/Contracts.kt` + `ui/Icons.kt`；`adminCoreTests` `include("com/xiaozhi/admin/ui/Contracts.kt")` | 删 `ui/` 后 web 打包与单测**直接失败** | P0-6 前先改这两处路径；作为迁移第 0 步 |
| R4 | **macOS Catalyst 补丁面对更大归档** | 静态 framework 归档成员数因 gearui-kit 大增，`patch_framework_macabi.py` 的 `ar x/cr/ranlib` 耗时与失败面上升 | 迁移后在 macOS 上实测补丁耗时与幂等性；必要时在脚本里加"成员数/耗时"日志 |
| R5 | **Kuikly 2.30.0 连带升级** | 渲染层（iOS/macOS 的 `OpenKuiklyIOSRender`、web 壳 `h5App.js`）版本必须同步，否则运行时崩溃 | 升级前确认四端渲染层制品；升级与 UI 迁移分成两个提交单元 |
| R6 | **`Upload` 的宿主适配** | OTA 固件上传、文件页上传依赖现有 expect/actual 文件选择器（macOS/iOS/js/android 各一份） | 先做上传链路单点验证再铺开；`Upload` 只做 UI，选择器复用现有 actual |
| R7 | **82 组件不覆盖的 7 类能力**（LED 环 / 波形 / 终端深色块 / 图标集 / 三档菜单 / 定宽左右分栏 / 抽屉浮层） | 不能"全部换成 GearUI"，必然保留少量自绘 | §4.1 逐条给了方案；自绘范围限定为 Canvas 绘制 + 布局原语，不重建组件库 |
| R8 | **一次性全量 + 600 行限制** | 迁移期大量文件同时改动，review 与回归成本高；`TestBenchPages.kt`(648) 与 `ConfigContextState.kt`(607) 已超限 | 实现按 §9 单元分批提交（每单元可独立编译）；超限文件随该单元一起拆分 |
| R9 | **暗→浅的认知成本** | 用户/自己上手需要重新认界面 | §5.4 四端截图验收 + 保留 K1~K10 语义锚点（图标/状态色/终端） |
| R10 | **Compose 在 web(H5) 的性能** | 12 页 + 17 section 全 Compose，长列表（日志/对话/控制台）滚动可能掉帧 | 长列表强制 `LazyColumn`；控制台日志保持"双上限"截断策略；在浏览器实测滚动帧率 |

### 8.2 待确认问题（需主理人/用户裁决，影响规格闭环）

| # | 问题 | 我的建议 | 影响面 |
|---|---|---|---|
| Q1 | `AdminType.scale`（视口宽度 → 1.0~1.5 字号缩放）是否保留？ | **不保留**，接受主题固定字号（与 D2"完全采用默认主题"一致）；若大屏可读性不足，用主题 typography 覆盖而非恢复 scale | 全部页面字号 |
| Q2 | `@Page("config")` 单页兜底是否保留？ | **保留**：非 web/macOS 平台（Android/iOS 直接打开）仍需要无侧栏入口 | `AdminPlatform` / 导航 |
| Q3 | `@Page("admin-compose")` 试点页是否删除？ | **删除**：迁移完成后它是重复物 | 页面注册 / E11/E12 |
| Q4 | 保存结果反馈改成 Toast 后，"✓ 已保存 / ✕ 保存失败"的三态按钮文案是否还要保留在按钮上？ | **按钮只保留 loading 态**，结果走 Toast（避免自绘态）；但 409 冲突仍用顶部「重新加载」按钮 | 标题栏 / 所有保存动作 |
| Q5 | GearUI `Checkbox` 是否支持 `Indeterminate`（三态）？ | 若不支持：用 `Radio`(3 项) 或 `Tag`+`Checkbox` 组合实现，**不允许**退化成二态（`UiContractsTest.mixedSelectionIsNotFalseOrAnInheritedOverride` 会失败） | 配置勾选、设备覆盖 |
| Q6 | GearUI `Tabs` 是否支持"等宽 + 6 项 + 窄窗最小 44pt"？ | 若只支持滚动 tab：EchoMuse 六 tab 允许改为横向可滚动，但**不允许**隐藏任何 tab | EchoMuse 页 |
| Q7 | GearUI `Icon` 是否自带图标集、覆盖 26 个 token 中的几个？ | 需在基座阶段做覆盖度比对表；缺的用 slot 挂自绘矢量 | K1 / `verifyAdminIcons` |
| Q8 | 迁移期间是否允许"旧壳 + 新壳"双入口共存？ | **允许**（便于逐单元验证），但**最后一个提交必须删旧壳**，不进入长期双轨 | 提交节奏 |
| Q9 | `AdminSpace` 是否以 `GearUiSpace` 名义保留？ | **保留**（纯数值，不是视觉 token）；否则 12 页 × 数十处间距需要重新对表 | 布局尺寸 |

---

## 9. 迁移单元分解（供架构师做任务分解的输入，非实施指令）

> 每个单元都应能独立编译通过，避免"全量替换中途全仓不可编译"。顺序即依赖顺序。

| 单元 | 范围 | 内容 | 主要文件 |
|---|---|---|---|
| U0 基座 | 构建 | 源码引入 gearui-kit + 五目标（含 `macosArm64`）+ Kuikly 2.30.0 升级 + 主题装配 + `GearUiSpace` | `settings.gradle.kts` / 新建 `gearuiKit` 或 vendored 模块 / `gradle.properties` / 根 `build.gradle.kts` |
| U1 契约搬迁 | 构建护栏 | `IconToken` + 图标映射迁新包；改 `verifyAdminIcons` 与 `adminCoreTests` include 路径 | `ui/Contracts.kt` → `contracts/`；`ui/Icons.kt`；`shared/build.gradle.kts:155-181`（`:159` = Contracts、`:160` = Icons）；`adminCoreTests/build.gradle.kts:23` |
| U2 状态容器 | 状态层（仅机制） | 411 `observable` + 35 `observableList` → Compose 快照；22 个 `PagerScope` 状态类 | `ConfigFormState` / `ConfigContextState` / `OpsState` / `EchoMuse*State` 等 49 文件中涉及观察机制的 22~50 个 |
| U3 壳层 | UI | `AdminPage` → `ComposeContainer`：`NavBar` + 侧栏 `List/Cell` + `Drawer` + `Toast` + 字段帮助 `Popover` | AdminPage.kt / ToastHost.kt / ui/FieldHelp.kt |
| U4 语音流程工作区 | UI | `Steps` 轨道 + `Tabs` + 左菜单 + 分发器 + 10 个 section 卡 | PipelineWorkflowView / ConfigTabs / ConfigWorkspaceViews / NodeTestPanel / ConfigCards / ConfigStageCards / ConfigNluCard / ConfigCommandCard / ConfigWakeupCard / ConfigSoulMemoryCards / ConfigPlaybackCard（audio 部分） |
| U5 全局配置 | UI | 独立布局 + 7 个 section 卡 | ConfigWorkspaceViews(global) / ConfigOverviewCard / EngineHealthCard / ConfigInfrastructureCards / ConfigEchoMuseCard / McpToolsCard / OtaCard |
| U6 运维面 | UI | 概览 / 设备 / 会话 / 对话 / 用量 / 延迟 / 控制台 | OpsPages.kt（493 行，须拆）/ ConsolePage.kt |
| U7 EchoMuse | UI | 六 tab + 审批页 + 设备卡 + BLE + 音乐 | EchoMusePage / EchoMuseDetailTabs / EchoMuseStatusTab(543,须拆) / EchoMuseActivityTab / EchoMuseConfigTab / EchoMuseConfigStages(470,须拆) / EchoMuseConsoleTab / EchoMuseUpdatesTab / EchoMuseLogsTab / EchoMuseDeviceCard / EchoMuseBleCard / EchoMuseMusicPanel |
| U8 存储面 | UI | 文件页（含上传/面包屑/删除）+ OTA 卡 | FilesPage.kt / OtaCard.kt |
| U9 测试台（mac） | UI | 测试台三段区 + NLU 页 | macosArm64Main/AdminPlatform.kt / TestBenchPages.kt(648,须拆) / TestBenchNluPage.kt |
| U10 清理与验收 | 收尾 | 删 `ui/`、删 `compose/`、删 `admin-compose` 页、跑 E1~E17 | 全仓 |

---

## 10. 附：本文档的核对基线（避免后续争论）

| 事实 | 判定方式 |
|---|---|
| 82 个 GearUI 组件分组与名称 | 上游调研（主理人已采信），本文不重复核实 |
| 无 `macosarm64` 制品 | 上游调研（Maven Central 发布物清单），本文不重复核实 |
| 110 个 .kt / 22,951 行 / `ui/` 18 文件 3,783 行 / `compose/` 7 文件 1,803 行 | `find` + `wc -l`（2026-10-27 实测） |
| 411 个 `observable` + 35 个 `observableList` | `grep -cE "by (scope\.)?observable\("` 全目录求和 |
| 12 导航项 / 17 section / 10+7 划分 / EchoMuse 6 tab + 1 审批页 | `AdminNav.all` / `ConfigSections.all` / `PipelineFlow.primaryForSection` 逐项读取 |
| 无"红涨绿跌"配色 | `grep -E "涨\|跌\|trend"` 全目录 0 命中 |
| 状态四色 / 危险红 / 终端深色 / 26 图标 | `ui/Widgets.kt:259` / `ui/Theme.kt:74-85` / `ui/Contracts.kt:49` / `verifyAdminIcons` |
| `verifyAdminIcons` 与 `adminCoreTests` 硬编码 `ui/` 路径 | `client/shared/build.gradle.kts:155-181`（`:159` `ui/Contracts.kt`、`:160` `ui/Icons.kt`）、`client/adminCoreTests/build.gradle.kts:23` |
| macOS Catalyst 补丁依赖 `macosArm64` framework 归档结构 | `client/scripts/patch_framework_macabi.py` 头部说明 |

---

*本文档为迁移规格基线。实施细节（模块命名、目标声明写法、主题装配方式）由架构师在设计文档中给出；任务分解以 §3 清单与 §9 单元为输入。*
