# 公共管理台维护

这里是跨端管理页面、表单和网络状态；macOS 专用测试台不属于 commonMain。

**单一壳**：web 与 macOS 共用 `@Page("admin")`（[AdminPage](<AdminPage.kt>)，Compose 壳 [AdminShell](<AdminShell.kt>)，一级导航 8 项），平台差异全部收口到 [AdminPlatform.kt](<AdminPlatform.kt>) 的 expect/actual（macosArm64Main 真实现，js/android/ios 空实现）。全仓只注册 `admin` 与 `config`（[ConfigPage](<ConfigPage.kt>)，无侧栏平台/深链的单页兜底）两个页面——KSP 生成的 `KuiklyCoreEntry` 锁定，**⛔ 不新增 `@Page`**（旧 `router` 双壳与 Compose 试点 `admin-compose` 均已删除）。

## 组件与状态（GearUI 迁移后）

- **组件库是 GearUI（vendored 于 [gearuiKit](<../../../../../../gearuiKit/>)，package `com.gearui.*`）+ 自建零件库 [kit/](<kit/>)**：页面一律先用这两处，再考虑新写。GearUI 提供语义组件（`Table`/`Tabs`/`Alert`/`ConfirmDialog`/`EmptyState`/`SkeletonText`/`Button`/`Input`/`Textarea`/`Switch`/`Select`/`Card`/`Cell`/`CellGroup`/`Timeline`/`Collapse`/`LinearProgress`…，主题走 `Theme.colors`/`Theme.typography`）；`kit/` 收敛跨页面的组合零件与口径，新页面先在 kit 里找同型零件，⛔ 不复制零件实现。⛔ 颜色/字号只取 GearUI 语义 token（`Theme.colors.foreground/mutedForeground/destructive/warning/primary/…`），**完全采用 GearUI 默认浅色主题**（D2 决策：`themeMode = Light`、`theme = null`，组合根不覆写色板）；`AdminColors/AdminType/AdminToken/AdminShape` 已随 `ui/` 退役（T12 步骤 6 删尽前仅剩 `icons/LegacyAppIcon.kt` 一处过渡引用），⛔ 新代码零引用（验收口径：`grep -rlE "AdminColors|AdminType|AdminToken|AdminShape"`）。**唯一色值例外**：深色终端的四个色（bg/fg/dim/accent）收敛在 [kit/GearTerminal.kt](<kit/GearTerminal.kt>) 的 `GearTerminalPalette`（K2 保留项），页面文件里零色值。
- **kit/ 零件索引**：[GearFields](<kit/GearFields.kt>)（`labeledField/labeledIntField/labeledDecimalField/labeledTextArea/switchRow/actionRow` + `INT_INPUT_FILTER/DECIMAL_INPUT_FILTER`）、[GearSelect](<kit/GearSelect.kt>)（`labeledSelect`）、[GearBadges](<kit/GearBadges.kt>)（`statusBadge/pluginStatusBadge`，**只在偏离时打标**——正常/在线不渲染任何徽标）、[GearLayout](<kit/GearLayout.kt>)（`pageScroller/splitRow/splitColumn/menuPaneRow/cardRow/twoPane/dividerH/hGap/vGap`）、[GearUiSpace](<kit/GearUiSpace.kt>)（间距常量）、[AdminScope](<kit/AdminScope.kt>)（`GearUiBreakpoints`：NARROW 768 / TWO_COLUMN 900 / WORKSPACE_COMPACT 700 / SIDEBAR_WIDTH 240，**一字不改**；`LocalAdminPager`/`LocalAdminViewport` CompositionLocal）、[GearFieldHelp](<kit/GearFieldHelp.kt>)（`LocalFieldHelp` 下发 ⓘ 帮助弹层）、[GearTerminal](<kit/GearTerminal.kt>)（深色终端四件：`GearTerminal/GearTerminalText/GearTerminalInput/GearTerminalKey`）、[GearWaveform](<kit/GearWaveform.kt>) / [GearLedRing](<kit/GearLedRing.kt>)。
- **状态一律 Compose 快照**：`state/StateVar.kt` 的 `stateVar`/`stateListOf`（读写进快照系统，组合期读到即随写入重组）。计数由护栏 `:shared:verifyComposeStateMigration` 锁定（当前 455/48、legacy=0；删除代码后用 codemod `--check` 实测值回填并在常量注释写明原因，⛔ 禁止为凑数字保留死代码）。⛔ 禁止再用 `com.tencent.kuikly.core.reactive` 的 `observable`/`observableList`——旧原语只在护栏白名单 `LegacyVfor.kt`（T12 随 `ui/` 删除）里允许出现。纯展示态（展开集合、开关、输入草稿的反查）放 `remember`，不进状态层。
- **契约层在 [contracts/](<contracts/>)**（零 Kuikly 依赖）：[UiContracts](<contracts/UiContracts.kt>)（`Availability/CheckState/IconToken` 等纯 UI 契约）、[AdminNav](<contracts/AdminNav.kt>)（一级导航常量表，8 项；`init` 断言 id 唯一 / order 升序 / 同组相邻；2026-10-29 范围削减后 `sessions/conversations/usage/metrics` 四项已移除）、[FieldHelpState](<contracts/FieldHelpState.kt>)（帮助状态实现 + `FieldHelpHost` 接口，页面 `val fieldHelp = FieldHelpState()` 持有，经 `LocalFieldHelp` 下发）。
- **导航与分区**：一级导航由 [AdminNav](<contracts/AdminNav.kt>) 驱动（web 端经 `visible(macos)` 隐藏 `macOnly` 的测试台）；配置分区由 [ConfigSections](<ConfigSections.kt>) 常量表驱动，id 稳定、order 确定，新增分区 = 表加一行 + [ConfigSectionHost](<ConfigCards.kt>) 的 `when` 加一条分支，section 实现落**各自文件**。
- **图标一律 `GearIcon(IconToken.X)`**（[icons/AdminIcons.kt](<icons/AdminIcons.kt>)，内嵌 SVG pathData 经 Kuikly `Canvas` 即时绘制，解析器见 [IconVectors.kt](<icons/IconVectors.kt>)）：⛔ 不拿文本字符当图标（粗细/基线/居中随字体漂移），也不在调用处手绘。`IconToken`↔几何一一对应由 `:shared:verifyAdminIcons` 锁定（26 个），新增图标改 [IconVectors.kt](<icons/IconVectors.kt>) 的 pathData，禁止回退资源文件/PNG/UIImage/位图。
- **Compose 运行时事实**（实测，勿踩）：Kuikly Compose **没有** `Modifier.verticalScroll`，页面滚动用 [kit](<kit/GearLayout.kt>) `pageScroller`（GearUI `ScrollView` 基座）；`fillMaxHeight` 在根约束下会退化成 wrap content（需要显式定高或 `weight`）；`Modifier.weight(1f)` 必须在**有 Column/RowScope 的调用处**传（独立 `@Composable` 里写 `.weight(1f)` 会 unresolved）；`Pager` 经 `LocalAdminPagerOrThrow()` 取，不要在 onClick 闭包里现取。
- **提交路径唯一**：一次 `POST /api/config`（部分更新）+ 设备 overlay 差量。⛔ 禁止第二套配置草稿/第二份提交实现——Compose 试点的 `AdminConfigDraft`「每段一份草稿各自提交」模式已随 `compose/` 删除，任何"分段独立保存"的需求都走 `ConfigFormState` 的部分更新语义，不恢复旧模式。

## 表单与网络契约

- **异步回调线程契约**：Kuikly 的 Kotlin 侧只允许在 context 线程碰快照状态/`setTimeout`。mac 端管理台实时通道 [AdminRealtime](<AdminRealtime.kt>) 的 `PlatformWsSocket` actual 经 XiaoZhiModule 原生桥（`XiaoZhiModule+AdminWs.m`）实现——模块回调 block 由框架包裹 `performOnContextQueue`，任意原生线程 invoke 都会落回 context 线程；**禁止**在平台 actual 里自建 NSURLSession/GCD 直调共享层（2026-10 断线重连 `assertContextQueue` 卡死事故根因）。服务器设置（`macServerSwitchEntry`）同理：发现事件走原生桥回调，不在 Kotlin 侧起线程。
- [ConfigFormState](<ConfigFormState.kt>)保存草稿，[fill](<ConfigFormFill.kt>)映射读取；密钥只读 presence、写入 putSecret，空草稿不发送。新增密钥与服务端 SECRET_PATHS 同批改。
- 整型字段用 labeledIntField、浮点字段用 labeledDecimalField（[kit/GearFields.kt](<kit/GearFields.kt>)，输入侧实时剥离非法字符）：裸 labeledField 留给文本/密钥——整数字段混入脏输入会让 `toIntOrNull() ?: 默认值` 把错误静默存成默认值。**读服务端回包时**，数字/布尔字段一律走 [intText / doubleText / boolText](<ConfigFormFill.kt>)：缺键保留当前值、当前值非数字才兜底默认值，输出转回表单文本（原 `optInt("k", 字段.toIntOrNull() ?: 默认值).toString()` 写法已全部收敛，共 57 处）；⛔ 不再在 fill 里手写这套转换。
- **字段文案口径**（2026-10 可读性重做，细则见[管理台界面设计 §5.10](<../../../../../../../../docs/designs/admin-ui-design.md>)）：标签只用中文见名知意（CPU / AI / Token / AppID 这类通用术语保留英文），配置 key 不上界面；复杂字段把解释写进 `help`（名称旁 ⓘ → 帮助弹层，组件层实现见 [kit/GearFieldHelp.kt](<kit/GearFieldHelp.kt>)），见名知意的字段不传 `help`、不挂 icon；常驻 caption 只留真正的风险与跨字段矛盾。控制动作（获取模型列表 / 测试凭据 / 探测音色目录 / 重建索引…）走 `labeledField` 的 `trailing` 参数放在名称同一行右端，不另起 actionRow。⛔ 帮助状态是页面持有的快照状态（[contracts/FieldHelpState.kt](<contracts/FieldHelpState.kt>) 实现 `FieldHelpHost`），不要写成全局单例——页面级唯一实例经 `LocalFieldHelp` 下发，徽标与弹层读到的是同一份。
- BLE 的危险目标 handle 是严格校验例外：使用 `labeledField` 保留原始输入并由 [EchoMuseBleCodec](<EchoMuseBleCodec.kt>) 的纯函数拒绝非法十进制/范围；不得把 -1 或 1.2 清洗成别的真实外设 handle，也不允许解析服务树数字时截断小数。
- hot/字段提示由[FormSchemaState](<FormSchemaState.kt>)消费服务端 schema，不手写另一套档位。schema 不是自动布局，engine/preset 可选项仍需两端同步。
- 409 保留草稿，显式"重新加载"，不静默覆盖；紧凑标题栏冲突/设备操作另起右对齐行，不挤压标题。preset 应用复用服务端配置管线；overlay 编辑只提交设备增量，不能混成全局整份配置。健康修复/全局设置的明确配置跳转用 `openConfigSection` 回到 Configuration 参数页，普通顶部节点点击才保留 tab；跳转后按 section 归属切菜单（有流程节点 → `config`，否则 → `settings`），不要用 `selectedSection` 硬编码。
- NetworkModule 只支持 GET/POST，使用服务端 POST 别名；curl 的 PUT/DELETE 能用不等于页面可用。
- 人格预览 effective 全段 fill，客户端不复制预设合并。能力 enabled 与 phase 分开，异常/降级须显示原因与修复指引。
- 设备/会话/最近对话注明进程内口径，online 非心跳，不呈现为完整历史。

本级说明 · [客户端构建](<../../../../../../../AGENTS.md>) · [管理台界面设计与验收记录](<../../../../../../../../docs/designs/admin-ui-design.md>)


---

## 管理台页面与操作

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 5. 管理台（Web + macOS）

管理台是 **Kuikly 多端工程**（`client/`），逻辑在 `client/shared/commonMain` 跨端复用；web 构建产物由 server 在 `/` 静态托管。

默认落地页是 **语音流程**（`@Page("admin")`，侧边栏 id `config`）：该内容区顶部为[横向工作流](<PipelineWorkflowView.kt>)（仅配置区渲染，不跨运维页面），VAD → ASR → NLU → LLM → TTS 为模块，头尾及衔接共六个可点击圆点；独立配置页复用同一组件。轨道宽按 `adminContentWidth` 自适应（桌面 ≥1280 无横向滚动，不设最大上限），窄屏横向滚动，不能退回纵向节点轨道/下拉。点击模块/圆点进入「配置 / 测试」两个标签页（GearUI `Tabs`），各自固定左菜单、右内容；测试左菜单选择单个动作。无配置搜索框，分区只能经工作流或侧边栏到达。工作区小于 700 宽保留 92 宽左菜单，操作行靠右并可换行。布局见 [ConfigTabs](<ConfigTabs.kt>)及[工作区菜单](<ConfigWorkspaceViews.kt>)，位置投影见 [PipelineWorkflow](<PipelineWorkflow.kt>)，功能仍由 [PipelineFlow](<PipelineFlow.kt>)声明。只持有一个 `ConfigSelection(sectionId,nodeId?,workflowPositionId?)`，top/left 选择均由它派生；tab/status/action 是局部展示维度。空圆点只说明暂无扩展，不创建伪配置或测试。**全局配置是侧边栏独立一级菜单**（id `settings`，[GlobalConfigLayout](<ConfigWorkspaceViews.kt>)）：只放没有流程节点的 section（能力总览 / 预设 / AIUI 全链路 / 服务 / EchoMuse 接入 / MCP / OTA），无工作流轨道、无配置/测试 Tab，与「语音流程」共用同一份表单与同一个「保存配置」动作（`ConfigSections.global` + `ConfigFormState.openWorkflowPage/openGlobalPage` 各自记住停留位置）。带流程节点的 section（含人格 / 记忆 / 音频入口与输出）只在「语音流程」里改，**不要**把同一 section 同时挂进两套导航。全局设置、草稿、保存冲突和独立请求凭据保持原契约，不得恢复第二份 active 状态：

| Section（一级导航，8 项） | 数据源 | 回答什么问题 |
|---|---|---|
| 概览 | `GET /api/stats` + `/api/plugins` | 设备在线数、活会话、11 个能力健康（degraded/failed 可点击跳配置） |
| 测试台 | —（仅 macOS 客户端） | 依赖原生麦克风/音频链路，`NavDescriptor.macOnly`：web 不显示该菜单；端到端语音测试用 macOS 客户端 |
| 设备 | `GET /api/devices` | 连过的设备、协商参数、上次会话为什么断；消息注入、个性化配置入口 |
| EchoMuse | `GET /api/echomuse/devices` + 管理台 WS `/api/admin/ws` | 批准制设备登记（在线/离线、版本、base_os）；设备卡 = 标题行（状态点+删除）+ **六 tab**（状态 / 活动 / 配置 / 控制台 / 更新 / 日志，未批准只有单页审批）；状态 tab 逐行设备事实（缺测 `—` + 副标）、资源三块、音量下发、蓝牙摘要、系统诊断折叠；配置 tab = 网络连接三块 + Stage 01–08 + 「下发配置」（部分更新：留空不发该键）；更新 tab 只读版本 + OWW 资产五态 + 维护（重启二次确认/复制诊断/重装 VAD），**零 OTA 文案**；时间戳本地格式化；页面停留期间经管理台 WS 事件推送实时刷新（断线自动重连），无轮询 |
| 控制台 | `GET /api/logs`（WS 日志流） | 实时服务器日志：级别筛选、暂停/复制/清空；深色终端（K2 保留项） |
| 语音流程 | `/api/config` 等 | 五个核心阶段 + 连接点分支（唤醒 / 指令 / 人格 / 记忆 / 音频入口与输出）的节点参数与节点测试 |
| 全局配置 | `/api/config` 等 | 没有流程节点的部署级参数：能力总览、预设、AIUI 全链路、服务、EchoMuse 接入、MCP、OTA；与语音流程共用一份表单、一次 `POST /api/config` |
| 文件 | `GET /api/files` + POST mkdir/upload/delete | 服务端存储目录浏览与管理：新建文件夹、上传模型/文件（`oww-models/` 即 EchoMuse 自定义唤醒词模型的存放处）、删除（二次确认，受保护文件拒绝） |

> ⚠️ 2026-10-29 范围削减（docs/plans/gearui-kit-scope-reduction.md §二）：`sessions / conversations / usage / metrics` 四个观测页已从产品范围移除，导航、页面、state 与服务端请求一并删除；数据口径仍由 `GET /api/stats` 的摘要（概览页磁贴）承载。
> ⚠️ 设备/对话/延迟是**进程内存态**：重启清零、`online` 由会话生命周期派生、对话只留最近若干轮。长期留存请开启 `[memory]`。
> 单页模式：`?page_name=config`（支持 `&edit_device=<id>` 直接进入某设备个性化编辑）。

#### HTTP API 一览（管理动作）

| 端点 | 语义 |
|---|---|
| `GET /api/health` / `GET /api/health/engines` | 存活探针 / 11 capability 健康自检（纯只读投影，幂等无副作用） |
| `GET / POST(=PUT) /api/config` | 配置读写（部分更新：缺失保持原值、显式 `null` 清除、密钥空串保留、`expected_revision` 乐观并发 409 拦过期写入） |
| `GET/PUT/DELETE /api/devices/{id}/overlay` | 每设备白名单增量（POST = PUT、`POST .../overlay/delete` = DELETE 别名，供 Kuikly 客户端使用） |
| `GET/POST /api/config/presets`、`POST .../{name}/apply`、`DELETE/POST .../{name}/delete` | 预设：列出 / 快照当前 / 应用 / 删除 |
| `GET /api/test/capabilities`、`POST /api/test/run` | 保存态 SQLite 快照的隔离节点/串联测试，带 revision、输入边界、阶段与真实观测口径；Web/macOS 复用公共面板，不声明硬件播放成功 |
| `POST /api/test/{llm,tts,vad}` | 旧接口保留，复用隔离服务与管理鉴权；请求级覆盖不落盘，不发布生产引擎 |
| `POST /api/test/llm/models` | 拉取上游模型列表（服务端代理 OpenAI 兼容 `GET {base}/models`；请求可带 api_base/api_key 仅本次覆盖，密钥不下发客户端） |
| `GET /api/memory/status`、`POST /api/memory/{recall,maintain,clear,rag/reindex}` | 记忆状态 / 手动试召回 / 立即维护 / 清空 / RAG 索引重建 |
| `GET /api/soul/presets`、`POST /api/soul/preview` | 人格预设清单 / 最终提示词预览 |
| `POST /api/devices/{id}/inject` | 设备消息注入（文本作为用户话语走完整管线） |
| `GET /api/echomuse/devices`、`GET /api/admin/ws`、`POST .../devices/{id}/{approve,command,settings,delete}` | EchoMuse 列表/批准/WiFi 指令/唤醒词部分更新/删除；GET token 编码进查询串，POST 走 Bearer；变更通知走管理台 WS（`/wait` 长挂已删除）；旧 reboot command 不再使用 |
| `GET .../devices/{id}/runtime`、`POST .../volume` | 手动运行状态快照；缺测显示未知、离线显示历史；音量仅 approved + emos + online + speaker，level 整数 0～127，202 仅排队、刷新 runtime 核对实际上报 |
| `GET .../devices/{id}/system/{info,logs}`、`POST .../system/reboot` | 仅 approved + emos + online；固定日志源 server（设备 `/tmp/server.log`）/messages/messages_last/last_kmsg_prev/supervisor/net，重启必须 confirm:true；响应仅确认发送，不能断言重启完成；不提供任意 shell/OTA |
| `GET .../devices/{id}/ble/{adverts,state}`、`POST .../ble/gatt` | 独立通用 BLE 面板：广播快照/槽位/连接/发现/读/确认写/手动事件；GET 带 session_id 查询、GATT 带 session_id 元数据，三类成功回包严格核对 device_id/session_id；都走 Bearer，客户端超时45s |
| `GET /api/logs` | 控制台 WS 日志流（级别过滤、暂停/续传由客户端状态机控制） |
| `GET /api/metrics`、`GET /api/usage` | 延迟大盘 / 令牌用量（内存态；2026-10-29 起客户端无独立页面，摘要进概览） |
| `GET /api/files?path=`、`POST /api/files/{mkdir,upload,delete}` | 服务端存储目录浏览与管理：列目录（服务端目录优先排序）/ 建目录（409=同名文件占用）/ 原始字节上传（409 需显式覆盖、413 超限）/ 删除（目录递归；data.db 等受保护文件 403）；GET token 走查询串、POST 走 Bearer，错误统一 `{"error":"..."}` |

EchoMuse 实现按职责分为 [列表与配置状态](<EchoMuseState.kt>)（列表/审批/WiFi 配网闭环 + 管理台 WS 订阅 [AdminRealtime](<AdminRealtime.kt>)、[enterTab](<EchoMuseState.kt>) 是六 tab 唯一入口）、[设备行模型](<EchoMuseDeviceRow.kt>)（纯数据 + 展示文本）、[音乐面状态](<EchoMuseMusicState.kt>)（DLNA 扫描/浏览/播放/停止，独立写动作锁）、[详情卡分区状态](<EchoMuseDetailState.kt>)（六 tab、进 tab 懒加载、runtime/系统信息/音量/OWW 资产/控制台会话/单操作闸门）、[配置草稿](<EchoMuseConfigDrafts.kt>)（Stage 01–07 的 48 个字段 + `buildConfigBody` 部分更新语义）、[日志状态](<EchoMuseLogsState.kt>)（固定来源白名单 + 读取 + 复制）、[控制台状态机](<EchoMuseConsoleState.kt>)（WS 通道 + 六态）、[控制台协议字面量与用户文案](<EchoMuseConsoleProtocol.kt>)（`ConsoleWire` / `ConsoleCopy`，跨端契约，⛔ 改名即协议破坏）、[控制台文本管线](<EchoMuseConsoleText.kt>)（纯函数：ANSI 剥离/`\r` 覆盖/双上限，零 Kuikly 依赖，由 `:adminCoreTests` 直驱）、[tab 共用小原语](<EchoMuseParts.kt>)（`EmInfoRow/EmNote/EmWarn/EmStatTile` + `EM_DASH`/`emNum`，六 tab 内部的"标签│值"行与缺测占位）、[设备卡（只有标题行）](<EchoMuseDeviceCard.kt>)、[六 tab 外壳与审批单页](<EchoMuseDetailTabs.kt>)、[状态 tab](<EchoMuseStatusTab.kt>)、[活动 tab](<EchoMuseActivityTab.kt>)、[配置 tab（网络连接三块 + 下发条）](<EchoMuseConfigTab.kt>)、[配置 Stage 01–08](<EchoMuseConfigStages.kt>)、[更新 tab（只读版本 + 资产五态 + 维护）](<EchoMuseUpdatesTab.kt>)、[控制台 tab（深色终端）](<EchoMuseConsoleTab.kt>)、[等宽字体族名（`expect` + 4 个 actual）](<AdminFonts.kt>)、[日志 tab](<EchoMuseLogsTab.kt>)、[音乐 Stage 08 内容](<EchoMuseMusicPanel.kt>)、[页面壳体](<EchoMusePage.kt>)、[鉴权与编码](<EchoMuseAccess.kt>)：
- 六 tab 是唯一视图入口：GearUI `Tabs(scrollable = true)` 横向滚动（窗口再窄也不折叠成「更多 ▾」），未批准设备只渲染审批单页（Action-required 横幅 + 设备号/IP/首次接入 + 「批准并加入」），批准后才出六 tab。
- 状态 tab 里**每个可能取不到的字段都以"缺测占位"呈现**，缺测一律 `—`（不填 0、不估算）；`Latency` 恒 `—`+「未测量」（官方由控制器 ping，本服务端不测）、`Flash 磨损` 恒 `—`+「未采集」（固件不上报）；物理静音与输出音量是**两行**（三态，不与 0 混）；`stats` 整块缺失时只说「等待设备上报…」；「系统诊断（原始输出）」默认收起。
- 配置 tab 的 Stage 01–08 严格只画 `admin.rs::SETTINGS_KEYS` 内的键，Stage 头必须写清「为什么比官方少」（缺白名单键 / 缺采集 / 官方那部分是控制器侧渲染）；Stage 07 渲染 `sendspinEnabled`/`sendspinName`/`sendspinUnpaired`（40 键含它，2026-10 起）；Stage 08 是「音乐（本服务端扩展，官方 dashboard 无此分组）」。
- 进入管理区不自动读设备；加载/失败/操作结果就地呈现，日志与系统输出可滚动及复制诊断内容。断线标历史；列表/详情/runtime 的 `session_id` 是当前 control 的观测身份，变化时清旧请求代际、busy、重启确认并标旧诊断为历史；runtime 仅设备号与会话都匹配时视为当前快照，双空兼容旧服务端、不将空当通配符。
- `configureAccess(token, base)` 在加载前传入；切 base 或 token 清管理目标和输出，列表/长挂/写操作通过代际丢弃旧连接回包。
- 鉴权语义（产品决策：**局域网默认可管理**）：`server.expected_token` 为空（默认）= 管理面免 token 可用（系统操作/管理接口直接放行，不再要求双 token 非空）；配置了 token 即收紧为必须鉴权——web 用 `?token=URL编码后的管理令牌` 进入，不附裸查询参数，不在诊断内容中复制 token；macOS 服务器设置提供密码型管理令牌输入，只保留在当前 Pager 会话，不落 SP/配置。设备侧看 `echomuse.token`：空 = 局域网信任设备（shell/BLE 通道不要求设备令牌），非空 = 设备必须带 `X-Em-Token`。请求凭据由 [AdminAccess](<AdminAccess.kt>)统一读取，不得复用配置草稿 expectedToken；受保护真实服务仍需独立联调。
- 唤醒设置以固件类型发送：`owwOnDevice=off|shadow|on`、`wakeSoundLevel=quiet|medium|loud`，旧 bool/number 只在读取时迁移（旧数字统一 medium）；阈值滑杆按官方量程 0.1～0.975、步进 0.025、Precise↔Eager 反向（常量与映射在 [EchoMuseWakeWord](<EchoMuseWakeWord.kt>)），轨外历史值读数保真、把手钉端点、拖动才落轨，保存仍拒非正数（服务端 `(0,1]`）；shadow 为官方诊断档、仅当前已处于该档时显示为选项，保存/重填后按当前值重建选项；提示音档位仅开关打开时显示；只发送本表单字段，服务端按部分更新合并，保留 BLE 等其他设置。唤醒词模型显示与选择规则在 [EchoMuseWakeWord](<EchoMuseWakeWord.kt>)（锚定上游默认 `hey_jarvis_v0.1`、官方四款与 display_name 规则）：存储为空即设备默认 hey jarvis 并明示，下拉列官方四款 + 服务端 `oww-models/` 已上传自定义模型 + 已存自定义值（[buildOwwOptions] 纯函数去重），保存恒发非空生效模型名（设备 config 对空 `owwModel` 不生效，恢复默认须显式发默认模型名）。卡片唤醒面板提供「管理模型文件」入口跳「文件」页 `oww-models/` 目录；**上传 ≠ 安装**——设备只认本地已装模型，未安装的模型选了也听不到唤醒词。
- 设备控制台（`GET .../devices/{id}/console`，WS 长驻）**不进 [AdminRealtime](<AdminRealtime.kt>) 共享组**：它是单设备单会话的独占租约（服务端与一次性操作共用一把槽），共享连接会把「另一个标签页已占住」变成静默复用；也**只手工重连**（自动退避 = 悄悄开一条新 root shell）。六态 `connecting/ready/offline/busy/closed/unsupported` 与全部用户文案在 [EchoMuseConsoleState](<EchoMuseConsoleState.kt>) 的 `ConsoleCopy`（唯一出处，与设计 §8.3／服务端 `console_frames.rs` 逐字一致）；`PLATFORM_WS_SUPPORTED=false`（Android）直接 `unsupported`、不建连不重试。帧协议字面量（`type`/`error.code`/`closed.reason`/`notice.code`/`ready.mode`）集中在同文件 `ConsoleWire`，任何一侧改名都是协议破坏。链路是 **pty 终端**：服务端发 `shell_open{pty:true}`（不再是不带 pty 的 pipe），设备回拨后若先发 `shell_meta{pty}` 则原样转给浏览器（控制帧不进终端画面）；一次性操作仍走 pipe，两个 `SHELL_OPEN` 常量⛔ 不许合并（pty 回显会污染 md5 校验与哨兵匹配）。
- 控制台输出**只在前端一处净化**：服务端只做 UTF-8 边界对齐，ANSI CSI/OSC（含跨 chunk 被切开的半截）、`\r` 就地覆盖、`\t`→4 空格、非法字节→`�` 全部在 [EchoMuseConsoleText](<EchoMuseConsoleText.kt>)（零 Kuikly 依赖，由 `:adminCoreTests` 的 `EchoMuseConsoleTextTest` 直驱）；行/字符双上限 2000 行 / 20 万字符、单行 4000 字截断，刷新 200 ms 合并。
- 控制台界面是**终端**（对齐官方 `dashboard.jsx` 的 `Shell`），实现 = [kit/GearTerminal.kt](<kit/GearTerminal.kt>) 四件：深色块 `GearTerminal` + 等宽正文 `GearTerminalText` + 无壳输入行 `GearTerminalInput` + 文字动作键 `GearTerminalKey`（中断/清屏/重连）。配色唯一真源是 `GearTerminalPalette`（K2 保留项），页面文件零色值——**终端里不要用 GearUI `Input`/普通浅色控件**，会画出浅色字段框破坏终端观感。⛔ 不再渲染行模式横幅、状态表格、首次进入说明、「关于行模式」折叠区与四按钮动作行——那些把终端退化成了「能打字的日志查看器」。
- 提示符与回显归**设备**：`shell_open{pty:true}` 之后由设备 pty 上的 mksh 自己打印提示符、回显与行编辑，前端不伪造提示符、不做行编辑。老固件忽略 `pty` 仍开 pipe（无提示符），写 stdin / 读输出不受影响。
- `执行`/`中断`/`清屏`/`重连` 是可用性兜底：软键盘的回车提交在部分端不可靠，没有可见动作键 web 端就没有提交入口。`LINE_MAX` 是 4096 **字节**（中文 3 字节/字），超长在终端内提示并置灰「执行」。
- 控制台文案唯一出处仍是 `ConsoleCopy`（`internal`，视图复用同一条 `insecure` 警告）：行模式专属的 `ROOT_BANNER`/`FIRST_ENTER`/`ABOUT_*`/`IN_USE_HINT` 已随界面一并删除，⛔ 不要再加回来——终端里常驻大段说明会挤掉输出区。⛔ UI 文案里不出现 markdown `**`。
- `closed{reason, detail}` → 文案的分派**只在 [ConsoleHint.closed](<EchoMuseConsoleProtocol.kt>) 一处**（纯函数、零 Kuikly 依赖，由 `EchoMuseConsoleClosedTest` 直驱）：⛔ 不要在状态机里再写一份 `when(reason)`。`reason=internal` **必须**拼上 `detail`——`console.rs` 的 `DETAIL_DIAL_TIMEOUT`「设备未在 15 秒内回拨 shell 通道」/`DETAIL_DIAL_CLOSED`「设备回拨通道已关闭」是这条失败唯一的信息源，丢掉就等于把所有内部错误都说成「服务端已退出或网络中断」，排查方向直接被带偏；反过来 `error{code:"internal"}` 的 `error` 字段服务端已带 `COPY_INTERNAL` 前缀，**不许**再拼 `INTERNAL_PREFIX`（否则双前缀）。`replaced`（槽被非预期接管）与 `superseded`（设备重连）是两件事，文案不许合并。
- 等宽字体是 `expect val ADMIN_MONO_FONT`（`js`/`android`=`monospace`、`ios`/`macos`=`Menlo`），**不打包字体文件**；族名未命中时平台自动回退，所以 UI ⛔ 不依赖字符宽度对齐（列对齐用固定宽度约束，不用空格）。新增该 `expect` 必须同时补齐 4 个 actual，否则少 actual 的那一端编译失败。
- 更新 tab 的资产五态来自 `GET .../oww/state`（只读，**读不到设备不是缺失** → `unknown`），进 tab 拉一次并 30 s 轮询、离开立刻停；它不占单操作闸门（闸门只在 [EchoMuseDetailState.run](<EchoMuseDetailState.kt>) 一处）。

「文件」实现按职责分为 [纯逻辑](<FilesModel.kt>)（相对路径校验与服务端同规则镜像、清单解析安全兜底、尺寸/`yyyy-MM-dd HH:mm` 本地时间格式化——零 Kuikly 依赖，由普通 JVM 测试模块 `:adminCoreTests` 直接回归）、[目录状态](<FilesState.kt>)、[页面](<FilesPage.kt>) 与 [平台选文件桥](<UploadFilePicker.kt>)：
- 写动作只走服务端 POST 端点（`mkdir`/`upload`/`delete`，即 NetworkModule GET/POST 约束下的别名）；GET 把 token 编进查询串、POST 走 Bearer；`{"error":"..."}` 文案就地透出（409 同名不静默覆盖、403 受保护、413 超限）。
- 删除一律**二次确认**（点「删除」武装 → `ConfirmDialog(destructive = true, dismissOnOutside = false)` 弹层，点遮罩不关闭，必须明确选「取消」或「删除」；换目录/刷新即复位）；目录删除由服务端递归，受保护文件不给删除按钮。
- 上传平台矩阵：**web（隐藏 `<input type=file>` + fetch 流式，独立 `window.__xzAdminUploadFile` 槽位）与 macOS（复用 XiaoZhiModule 通用文档选择器 + `httpRequestBinary`，180s 超时）可用；android/ios 回调"当前平台不支持上传"**。选文件桥与 OTA 固件选包同型但完全独立（槽位/字节缓存分离，互不串包）。
- EchoMuse 设备列表加载成功后顺带静默拉取 `oww-models/` 的 `.onnx` 文件名（`modelFilesRefresh`，失败保持现状），唤醒词下拉据此追加自定义模型。

OTA 固件托管（`GET /api/ota/firmware` + 上传/删除，[OtaState](<OtaState.kt>) + [OtaCard.kt](<OtaCard.kt>)，入口挂「全局配置 → OTA」）与 `/api/config` 完全无关：保存配置不动固件、上传/删除固件不动配置。上传链路唯一：`pickFirmwareFile` / `uploadPickedFirmware` expect/actual 平台桥（macOS 原生选择器 + 二进制通道；web `<input type=file>` + fetch）；功能/型号/版本由服务端从包内元数据自动识别，表单字段只是可选覆盖（默认收起）。删除走 `ConfirmDialog(destructive)`；上传中进度条为**不确定态**（状态层无字节级进度且禁加字段，不显示假百分比），结果以 toast + 清单刷新表达，失败用常驻 `Alert(DANGER)` 给完整原因。

BLE 独立分为 [纯校验/codec/确认指纹](<EchoMuseBleCodec.kt>)、[请求与状态](<EchoMuseBleState.kt>)、[面板](<EchoMuseBleCard.kt>)：
- 已批准设备可展开单设备面板；在线、`ble_connect` 和非空当前会话才可请求，GATT 还要求每设备扫描/连接都开启；服务端 `ble_proxy` 与双 token 是额外门控，不能由页面开启。无 session 的旧服务端失败关闭，不复用 emOS runtime 双空兼容策略。
- 广播快照可选择 public/random MAC 或手填；服务树展示 UUID/声明 handle/value_handle/props 与描述符。特征读写必须使用 value_handle，描述符只用其 handle；HTTP200 中 result.ok=false 仍是失败，slots 按 t=slots 判断；read 必须显式字符串 value 且合法 base64，缺测不能假空值。⚠️ 手填 MAC / 切地址类型的函数名是 `applyAddress`/`applyAddressType` 而**不是** `setAddress*`：`address`/`addressType` 是 `var`，Kotlin 会为它们生成同签名的 `setAddress(String)`/`setAddressType(Int)`，同名函数在 JVM/Android 目标直接撞「Platform declaration clash」——`js`/`macos` 不报，Android 编译才红。
- hex 非空、最多512字节、只允许ASCII空白分隔的完整hex字节；转换标准base64。写入二次确认捕获设备、session、外设MAC/地址类型、handle、字节与response。任何草稿变化清确认；外设选择变更丢旧回包但不解除在途busy，编辑不可能撤销已发送操作；bridge设备/base/token/session/在线变化使旧代际失效。独立请求序号仅释放自己的busy。
- response=true成功只代表ATT接受，response=false仅代表设备发送，都不是IoT业务动作完成证明；CCCD UUID 0x2902由用户显式选择描述符handle，普通写0100/0200/0000，不猜handle、不新增subscribe协议。GET state仅手动读取有界notify/disconnected窗口，无后台高频轮询，也不承诺实时或完整历史。
- BLE设置仅POST部分更新bleProxyEnabled/bleProxyConnections，沿用旧settings接口不附session元数据（前端代际仍检查）；结果只说明已保存/在线推送排队，不假定固件已应用。广播、服务树、原始输出均有界滚动。纯函数由普通测试模块 `:bleProtocolTests:jvmTest` 直接引用同一生产 codec 验证，无 Kuikly/原生宿主依赖，且不进入生产 bundle；跨端编译不等于真实外设联调。

测试台（macOS 专属，[TestBenchState](<../../../macosArm64Main/kotlin/com/xiaozhi/admin/TestBenchState.kt>)，页面在 [TestBenchPages.kt](<../../../macosArm64Main/kotlin/com/xiaozhi/admin/TestBenchPages.kt>)）正在重写为 Compose + GearUI：五个 tab（连接 / ASR 识别 / TTS 合成 / LLM 对话 / NLU 意图）的语义与录音→试听→识别解耦、合成→波形回放流程不变；音频播放**模块层直控**（经 `XiaoZhiModule` 原生桥，不依赖渲染器 AudioView）。此前 DSL 实现右栏的配置卡占位（`benchConfigHandoff`）随重写删除，恢复为与配置页同一份 Compose 配置卡。

```bash
## 构建 web 管理页（产物含 index.html 与 nativevue2.js，由 [server].admin_dir 指向）
cd client && ./gradlew :apps:h5App:publishWeb

## 启动 server（admin_dir 默认 ../client/apps/h5App/web，按实际构建输出调整）
cd server && ~/.cargo/bin/cargo run --features sherpa -- --config-db ./data/data.db
## 浏览器访问 http://127.0.0.1:8000/（管理台）
```

---

## 上级入口

[客户端指引](<../../../../../../../AGENTS.md>)

## 本级资料

- [管理台界面设计与验收记录](<../../../../../../../../docs/designs/admin-ui-design.md>)：信息架构、上下文页面、健康与记忆验收的历史记录。
