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
| EchoMuse | `GET /api/echomuse/devices` + 管理台 WS `/api/admin/ws` | Kuikly 设备摘要卡 → 唯一管理弹窗 → 一条系统 Tabs；官方六项（状态/活动/配置/控制台/更新/日志）与六个原生扩展混排，功能相同的原生视图直接移除，不设「官方/原生」二级切换 |
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

EchoMuse 第一阶段采用「Kuikly 卡片、弹窗、系统 Tabs + 官方内容嵌入」：
- [设备列表](<EchoMusePage.kt>)只显示[摘要卡](<EchoMuseDeviceCard.kt>)；卡片或「管理」按钮打开唯一[管理弹窗](<EchoMuseOfficialEntry.kt>)。选中设备从实时列表反查，删除、切服务器/令牌、离页或关闭时释放嵌入文档、详情轮询与 shell；设备重登记不恢复旧弹窗。
- 顶部唯一 GearUI `Tabs(scrollable=true)` 混排 12 项：官方的状态/活动/配置/控制台/更新/日志；原生的 DLNA 音乐/GATT 工具/VAD 资产/模型文件/WiFi 确认/原始诊断。**禁止恢复「官方 UI / 原生 UI」二级切换或官方内部重复 Tab 栏**。功能相同的旧原生六 Tab 源码直接移除，仅保留官方内容。
- 工作区复用 GearUI 居中 modal overlay，遮罩外点击不关闭，尺寸随视口变化；页面 Pager/viewport/spacing/fieldHelp 显式穿过全局 overlay 门户。未批准设备由官方审批内容处理，原生扩展禁止越过审批。
- [嵌入视图](<EchoMuseOfficialView.kt>)通过 Kuikly `MakeKuiklyComposeNode` 桥接 Web iframe / macOS `WKWebView`；URL 为 `/em/?embed=1&device=<编码设备号>&tab=<官方tab>&token=<编码令牌>`，不另开浏览器页面。Android/iOS 暂显示明确不支持提示。切出官方内容即销毁文档，防止隐藏控制台占 shell。
- 官方宿主 [host.js](<../../../../../../../apps/h5App/em/static/host.js>) 的 embed 模式只读选中设备，按 `tab` 呈现对应官方内容，不挂舰队列表、全局页头或第二层弹窗；官方 `dashboard.jsx` / `dashboard.js` 保持校验不变。独立 `/em/` 仅保留兼容入口，产品主路径不跳出 Kuikly。
- [原生扩展](<EchoMuseExtensionContent.kt>)不复刻官方状态/配置/控制台/日志/重启/审批/删除。DLNA 复用 [音乐状态](<EchoMuseMusicState.kt>)；GATT 复用 [蓝牙状态](<EchoMuseBleState.kt>)（BLE 扫描/连接开关交官方配置）；VAD 复用 [资产面板](<EchoMuseAssetsPanel.kt>)；模型文件在同一弹窗内复用 [FilesPage](<FilesPage.kt>)，默认目录 `oww-models/`，上传不等于设备安装。
- WiFi 原生项只补 `wifi_commit`，不重复扫描/切网表单。确认必须来自当前设备/会话 runtime 的 `wifi_result.ok=true`，历史/离线快照不允许确认。原始诊断只提供设备 `system/info` 原文及复制，不重复官方状态磁贴或日志。所有 202 只代表排队，不保证生效。
- `configureAccess(token, base)` 在加载前传入，切服务器/令牌使旧请求代际失效。局域网默认免 token，配置 `server.expected_token` 后必须鉴权；凭据从 [AdminAccess](<AdminAccess.kt>)读取，不借用配置草稿、不写日志/诊断。底层状态/API/纯协议契约继续复用，不另造提交链路。

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
