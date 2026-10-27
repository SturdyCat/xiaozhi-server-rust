# 公共管理台维护

这里是跨端管理页面、表单和网络状态；macOS 专用测试台不属于 commonMain。

**单一壳**：web 与 macOS 共用 `@Page("admin")`（[AdminPage](<AdminPage.kt>)），平台差异全部收口到 [AdminPlatform.kt](<AdminPlatform.kt>) 的 expect/actual（macosArm64Main 真实现，js/android/ios 空实现）。原 macOS 专属 `AdminShell` @Page("router") 双壳已废除。

## UI 与布局

- **组件库在 [ui/](<ui/>)（package `com.xiaozhi.admin.ui`）**：一律先在这里找，再考虑新写。全部基于 Kuikly 官方组件（`View`/`Text`/`Scroller`/`Tabs`/`PageList`/`AlertDialog`/`Canvas`）与官方 flex 属性组合，不自造交互控件、不手写样式常量。分工：[Theme](<ui/Theme.kt>) token（`AdminColors/AdminSpace/AdminType/AdminShape`）、[Layout](<ui/Layout.kt>) 布局原语（`splitRow/splitColumn/menuPaneRow/cardRow/twoPane/dividerH/dividerV/pageScroller/hGap/vGap`）、[Card](<ui/Card.kt>) 卡片（`cardSurface/splitCard/groupedCard/skeletonCard/dialogSurface/insetPanel` + `sectionTitle/textNote`）、[Header](<ui/Header.kt>) 头部行（`headerRow/headerTitle/HeaderTone`）、[Rows](<ui/Rows.kt>) 语义行（`infoRow/metricRow/captionLine/detailLine/groupLabel/outputBlock`）、[Menu](<ui/Menu.kt>) 菜单项（`menuListItem/MenuItemStyle`）、[Button](<ui/Button.kt>)、[Form](<ui/Form.kt>)（`fieldBox/labelWithHelp/labeledField/labeledTextArea/labeledIntField/labeledDecimalField/switchRow/actionRow`）、[Choice](<ui/Choice.kt>)、[Widgets](<ui/Widgets.kt>)（`dropdownField/waveformPlayer/statusBadge/dot/largeTitleBar`）、[Tabs](<ui/Tabs.kt>)（`tabBar` 面板内标签栏 / `tabbedPanel` 整页翻页）、[Nav](<ui/Nav.kt>)、[Icons](<ui/Icons.kt>)/[IconVectors](<ui/IconVectors.kt>)、[ActivityIndicator](<ui/ActivityIndicator.kt>)、[FieldHelp](<ui/FieldHelp.kt>)（`infoBadge/fieldHelpDialog/FieldHelpState`）、[Contracts](<ui/Contracts.kt>)（`Availability/ActionState/IconToken/CheckState` 等纯 UI 契约）。admin 包内的页面/状态文件用 `import com.xiaozhi.admin.ui.*` 取用。⛔ 不在页面里手写 `View { flexDirectionRow(); … View { flex(1f) } … }` 这类版式，也不复制 token 字面量。
- 页面/配置分区由[Nav](<ui/Nav.kt>)和[ConfigSections](<ConfigSections.kt>)常量表驱动，id 稳定、order 确定。新增分区同时补渲染分支。
- Tabs/PageList 显式提供 pageItemWidth/Height；滚动事件回传 Tabs.scrollParams，点击 tab 调 scrollToPageIndex。不自画指示器。配置/测试工作区无横向 PageList，以相同两态驱动官方 Tabs.scrollParams 与选中内容。
- 紧凑断点取 `adminContentWidth`：AdminPage 常驻侧栏时扣 240+1，其余页取整窗，不能直接拿整窗判工作区宽。H5 官方 Scroller 的 `showScrollerIndicator(false)` 对应 `isList list-no-scrollbar`；宿主 [index](<../../../../../../../apps/h5App/web/index.html>)须保留该 class 的 scoped CSS，不能用 overflow:hidden 或 DOM 轮询伪修滚动条裁切。
- 嵌套 DSL 先 `val ctx=this` 再显式访问 Pager；复用渲染为文件级 ViewContainer 扩展，ViewBuilder 显式传接收者。observable 在 attr/vif 闭包内读取才被跟踪。
- 可点击目标（侧边栏项/标签页/按钮/下拉与选项行/健康跳转行）一律 `attr { cursor(CursorType.POINTER) }`；core 的 cursor prop 仅 macOS 原生直读，web 端靠 h5App 的 `WebCursorPropHandler` 外部处理器落 `style.cursor`，别在业务里散写 CSS。
- **布局分区统一走 [Layout](<ui/Layout.kt>) 的左右/上下原语**：`splitRow`（左 │ 1px 竖线 │ 右）与 `splitColumn`（上 │ 1px 横线 │ 下），分区用 `fixed`（固定宽/高，如左菜单、侧边栏）/`grow`（弹性）/`auto`（内容自适应）声明尺寸，相邻分区自动插分隔线，可任意嵌套——主壳「侧栏│内容」、配置工作区「Tabs│左菜单│面板」、EchoMuse 设备卡「头部│六 tab」都是同一出处；`menuPaneRow`/`cardRow` 仅是常用组合的语义别名。⛔ 不自画 1px 分隔线、不手写 `flexDirectionRow` + 固定宽列。**左右竖线内部是绝对定位**（`dividerVAt`，left = 前面常驻定宽分区累计宽度、top/bottom=0 撑满行高）：Kuikly 的交叉轴 stretch 只在父容器交叉轴尺寸已定时生效，卡片这类「高度由内容决定」的行里 stretch 线高恒为 0（源码处仍是 `TODO: Correctly set the height of items with undefined (auto) crossAxis dimension`），这正是「卡内看不到分割线」的根因——⛔ 不要改回 `alignSelfStretch`。
- **上下布局的上一段一律用 [headerRow](<ui/Header.kt>)**（左 title + 可选状态/徽标 │ 右操作）：`headerRow("设备 A", meta = { 状态点 }, actions = { 删除按钮 })`，标题档位用 `HeaderTone`（SECTION 卡片标题 / **PRIMARY 条目名** / BODY 摘要 / CAPTION 工具条 / MICRO 栏名），落在定高工具条里传 `bottomGap = 0f`，**列表条目的标题行传 `tone = HeaderTone.PRIMARY` + `topGap`**，可跳转条目直接传 `onClick`（自带悬停指针）。已有实现：EchoMuse 设备卡头、运维页「摘要 + 刷新」与设备/会话条目、OTA 条目、MCP 设备块、能力行、健康卡偏离行、工作流标题行。⛔ 不再手写「标题 + `View { flex(1f) }` 弹性占位 + 按钮」。含面包屑/多按钮可换行的复杂工具条（如文件页面包屑头）保持自身实现。
- **间隔一律用 `hGap(w)` / `vGap(h)`**（[Layout](<ui/Layout.kt>)）：Kuikly 的 flex 样式没有 `gap` 属性（官方 `FlexStyle` 只有 direction/justify/align/flex/wrap + margin），间距只能靠占位或 margin 表达——占位这一种收敛在组件库里。⛔ 不写裸 `View { attr { width(AdminSpace.sm) } }` 当间距。
- **图标一律 `appIcon(IconToken.X)`**（[Icons](<ui/Icons.kt>) 的官方 Canvas 矢量）：⛔ 不拿文本字符当图标（`▾ ▴ ▶ ‖ ✓ ✕ ›` 的粗细、基线、居中随字体漂移，历史上有 5 处），也不手绘线条图标（原汉堡按钮的三条 2px 横线已换 `IconToken.Menu`）。`IconToken` 缺图标就往 [IconVectors](<ui/IconVectors.kt>) 里加矢量，而不是在调用处画。
- **语义行不重复实现**：条目名下面那行 caption/textSecondary 用 `captionLine`，次级补充（合计/输入输出）用 `detailLine`，键值对用 `infoRow`，指标数字用 `metricRow`，分组小标题用 `groupLabel`，诊断日志用 `outputBlock`，卡片内子块用 `insetPanel`。⛔ 不再手写 `Text { attr { fontSize(caption); color(textSecondary); … } }` 这一类"标准行"。
- **菜单项一律用 [menuListItem](<ui/Menu.kt>)**（`MenuItemStyle` 三档：MAIN 主壳侧栏 / WORKSPACE 配置工作区 / CARD 卡片内菜单），选中态、图标着色、警示点（经 `trailing` 传入）、禁用态只有一处实现；`sidebarItem` 是 MAIN 档的语义别名。⛔ 不新增第四份菜单项 View。
- **卡片外观一律复用 [cardSurface](<ui/Card.kt>)**（cardBg/radiusLg/1px divider 边框/cardPadding·紧凑 sm/gutter 下外边距的唯一出处），其内容槽就是 `SplitScope`——卡片天然是上下布局容器，典型形态是「**上下包裹左右**」：`splitCard(header = {…}) { menuPaneRow(…) }`（配置工作区式；EchoMuse 设备卡是「header │ tabBar + `vif` 内容」的上下包裹，**不再**用定宽左菜单——`dividerVAt` 的锚点依赖定宽列，`splitColumn` 的 stretch 线没有该约束），固定标题用 `groupedCard(title)`（内部委托 `splitCard` + `sectionTitle`），骨架用 `skeletonCard`，弹层/浮层用 `dialogSurface(width = …)`（无下外边距、宽度按窗口算，`bordered=false` 为下拉选择等轻量弹层）。⛔ 禁止在调用方重抄 `backgroundColor(cardBg) + borderRadius(radiusLg) + border(1f …) + padding(cardPadding)` 这套壳。**卡内左右分栏不铺 `sidebarBg`**：它等于页面底色，铺进 `cardBg` 卡片里会暗一档、像嵌了块异物——卡内「菜单 │ 内容」的区分只交给那条 1px 竖线（`menuPaneRow` 不传 `menuBackground`）。web 端文本划选复制依赖 h5App 入口 `KuiklyProcessor.preventDefaultSelect = false`（渲染器默认在桌面浏览器把 Scroller 内 selectstart 全部拦截，整页只剩 Scroller 外的标题可选）；drag 拦截保持 true 防 HTML5 原生拖拽卡死 PageList，不要回退。
- **Compose 只能整页，不能局部嵌入**：[compose/](<compose/>) 的页面继承 `com.tencent.kuikly.compose.ComposeContainer`，而它**本身就是 `Pager` 子类**——`setContent` 是该 Pager 的唯一根内容。框架没有「在传统 DSL 布局里嵌一块 Compose」的公开 API（`ComposeSceneMediator` 虽为 public，但要自行接管触摸/布局/帧驱动，属未公开契约）。因此 [Form](<ui/Form.kt>)/[Choice](<ui/Choice.kt>) 的磁贴类控件（`switchRow`/`checkBoxRow`/`segmentedChoice` 等，散在 15 个 card 文件约 58 处调用）**不能**逐个换成 SettingsXxx；Compose 化的最小单位是一整页，且**整页换壳后拿不到旧壳的侧栏/工作流轨道**，必须在 Compose 里重建。另两条实测差异：Kuikly Compose 2.29.2 **没有** `Modifier.verticalScroll`（滚动用 `LazyColumn`）；`fillMaxHeight` 在根约束下会退化成 wrap content（用 `pageData.pageViewHeight` 显式定高）。
- **Compose 迁移进度（2026-10 起，全量整页迁移）**：目标壳是 [AdminComposeShellPage](<compose/AdminComposeShell.kt>)（`@Page("admin-compose")`，侧栏复用 [AdminNav](<ui/Nav.kt>) 常量表，尚未接入旧壳入口，用 `?page_name=admin-compose` 直达）。已迁：「全局配置 → AIUI 全链路模式」（[AdminSettingsSection](<compose/AdminSettingsSection.kt>)，接真实 `GET/POST /api/config`，只提交 aiui 段五个字段）。其余 section 一律渲染「前往旧版」占位，**不做半成品表单**。迁移顺序：全局配置其余段 → 概览/观测类 → 设备 → EchoMuse → 语音流程 → 文件/控制台；全部迁完后把 `@Page("admin")` 换成 Compose 壳并删除 DSL 实现。
- **Compose 侧必须走主题桥**：颜色/字号一律来自 [AdminComposeTheme](<compose/AdminComposeTheme.kt>)（`AdminC` 由 [AdminToken](<ui/Theme.kt>) 的 ARGB 常量派生，`AdminType` 供字号），磁贴显式传 `colors = adminListItemColors()` / `shape = adminTileShape()`——⛔ 不在 Compose 文件里写 hex 字面量或默认 Material 配色，否则暗色页面里会冒出浅色紫调卡片。
- **迁移段的开关/单选/多选一律走 [AdminSettingsControls](<compose/AdminSettingsControls.kt>)**：`AdminSwitchTile`（旧 `switchRow`）、`AdminCheckTile`（旧 `checkBoxRow`）、`AdminRadioTile`（单选行）、`AdminSegmentedTile`（旧 `segmentedChoice`）、`AdminDropdownTile`（旧 `dropdownField`，上游无下拉，用 MenuLink + 内联展开面板补齐）、`AdminTextTile`（旧 `labeledField`/`labeledIntField`，BasicTextField 内嵌）、`AdminInfoTile`（旧 `infoRow`）。⛔ 页面里不直接调上游 `SettingsXxx` 再手传主题参数，也不退回旧 DSL 控件——两套混用会让同一页出现两种行高与圆角。
- **段 = 一个 `AdminConfigDraft` 子类**（[AdminConfigDraft](<compose/AdminConfigDraft.kt>)）：`fill(config)` 读 `GET /api/config`、`collect()` 交部分更新、`rows()` 声明界面。读写在基类，409 `expected_revision`、加载/保存态统一，不许各段重写一遍。
- **行模型**（[SettingRow](<compose/AdminSettingsRow.kt>)）：`rows()` 返回一串 `SettingRow`，页面在 `LazyColumn` 里**一条字段一个 item** 渲染。⛔ 不要把整段表单塞进单个 item——item 高度远超视口时 Kuikly Compose 会出现绘制重叠（实测「音频与播放」首版顶部字段与组标题叠在一起）。行间距自己补 8dp（对齐上游 `SettingsGroup` 的 `spacedBy(8.dp)`）。
- **material3 缺失组件是自建补丁，不是上游**：Kuikly Compose 2.29.2 的 material3 制品**没有** `ListItem`/`ListItemColors`/`ListItemDefaults`/`RadioButton`/`SegmentedButton` 系列，Compose-Settings 全部磁贴都编译不过。[sharedCompose](../../../../../../../sharedCompose) 的 `com/tencent/kuikly/compose/material3/` 下有补齐实现（含 `LocalListItemHeadlineStyle` 字号注入槽位）；`expressive/` 变体依赖整套 M3 Expressive API，已移除。升级 Kuikly 时先验证这批符号是否被上游补上，补上就删本仓库补丁。
- 官方传统 DSL Button 保留标题、交互与无障碍语义；加载 Canvas 与图标 Canvas 都是不可交互的兄弟覆盖层，不塞任意子节点、不交给图片槽。图标 = [Icons.kt](<ui/Icons.kt>) 内嵌 SVG pathData（viewBox 0 0 24 24）经 Kuikly Canvas 即时绘制（方法对齐 xiaoya-player 的 TvIcons，2026-10 从 assets 资产迁入代码：原三端宿主实现随 macApp 资源缺包全空白而废除）；新增/改图标直接写 pathData（[IconVectors.kt](<ui/IconVectors.kt>) 解析器支持 M/L/H/V/C/S/Q/T/A/Z），IconToken↔几何一一对应由 `:shared:verifyAdminIcons` 护栏锁定，禁止回退资源文件/PNG/UIImage/位图。NMAllResponse.data 非空，不复制恒假的 data==null 判断。

## 表单与网络契约

- **异步回调线程契约**：Kuikly 的 Kotlin 侧只允许在 context 线程碰 observable/`setTimeout`。mac 端管理台实时通道 [AdminRealtime](<AdminRealtime.kt>) 的 `PlatformWsSocket` actual 经 XiaoZhiModule 原生桥（`XiaoZhiModule+AdminWs.m`）实现——模块回调 block 由框架包裹 `performOnContextQueue`，任意原生线程 invoke 都会落回 context 线程；**禁止**在平台 actual 里自建 NSURLSession/GCD 直调共享层（2026-10 断线重连 `assertContextQueue` 卡死事故根因）。服务器设置（`macServerSwitchEntry`）同理：发现事件走原生桥回调，不在 Kotlin 侧起线程。
- [ConfigFormState](<ConfigFormState.kt>)保存草稿，[fill](<ConfigFormFill.kt>)映射读取；密钥只读 presence、写入 putSecret，空草稿不发送。新增密钥与服务端 SECRET_PATHS 同批改。
- 整型字段用 labeledIntField、浮点字段用 labeledDecimalField（[Form](<ui/Form.kt>)，输入侧实时剥离非法字符）：裸 labeledField 留给文本/密钥——整数字段混入脏输入会让 `toIntOrNull() ?: 默认值` 把错误静默存成默认值。**读服务端回包时**，数字/布尔字段一律走 [intText / doubleText / boolText](<ConfigFormFill.kt>)：缺键保留当前值、当前值非数字才兜底默认值，输出转回表单文本（原 `optInt("k", 字段.toIntOrNull() ?: 默认值).toString()` 写法已全部收敛，共 57 处）；⛔ 不再在 fill 里手写这套转换。
- **字段文案口径**（2026-10 可读性重做，细则见[管理台界面设计 §5.10](<../../../../../../../../docs/designs/admin-ui-design.md>)）：标签只用中文见名知意（CPU / AI / Token / AppID 这类通用术语保留英文），配置 key 不上界面；复杂字段把解释写进 `help`（名称旁 ⓘ → 官方 AlertDialog 弹层，[FieldHelp](<ui/FieldHelp.kt>)），见名知意的字段不传 `help`、不挂 icon；常驻 caption 只留真正的风险与跨字段矛盾。控制动作（获取模型列表 / 测试凭据 / 探测音色目录 / 重建索引…）用 `fieldLabel(..., trailing = {...})` 放在名称同一行右端，不另起 actionRow。⛔ 帮助状态别写回全局单例 observable：脱离 PagerScope 的 observable 不驱动重建；状态由页面经 [FieldHelpHost](<FieldHelp.kt>) 持有，徽标与弹层用 `getPager()` 回溯同一份。
- BLE 的危险目标 handle 是严格校验例外：使用 `labeledField` 保留原始输入并由 [EchoMuseBleCodec](<EchoMuseBleCodec.kt>) 的纯函数拒绝非法十进制/范围；不得把 -1 或 1.2 清洗成别的真实外设 handle，也不允许解析服务树数字时截断小数。
- hot/字段提示由[FormSchemaState](<FormSchemaState.kt>)消费服务端 schema，不手写另一套档位。schema 不是自动布局，engine/preset 可选项仍需两端同步。
- 409 保留草稿，显式“重新加载”，不静默覆盖；紧凑标题栏冲突/设备操作另起右对齐行，不挤压标题。preset 应用复用服务端配置管线；overlay 编辑只提交设备增量，不能混成全局整份配置。健康修复/全局设置的明确配置跳转用 `openConfigSection` 回到 Configuration 参数页，普通顶部节点点击才保留 tab；跳转后按 section 归属切菜单（有流程节点 → `config`，否则 → `settings`），不要用 `selectedSection` 硬编码。
- NetworkModule 只支持 GET/POST，使用服务端 POST 别名；curl 的 PUT/DELETE 能用不等于页面可用。
- 人格预览 effective 全段 fill，客户端不复制预设合并。能力 enabled 与 phase 分开，异常/降级须显示原因与修复指引。
- 设备/会话/最近对话/延迟注明进程内口径，online 非心跳，不呈现为完整历史。

本级说明 · [客户端构建](<../../../../../../../AGENTS.md>) · [管理台界面设计与验收记录](<../../../../../../../../docs/designs/admin-ui-design.md>)


---

## 管理台页面与操作

> 本文为当前功能使用说明；实现细节以邻近源码、配置样例与测试为准。

### 5. 管理台（Web + macOS）

管理台是 **Kuikly 多端工程**（`client/`），逻辑在 `client/shared/commonMain` 跨端复用；web 构建产物由 server 在 `/` 静态托管。

默认落地页是 **语音流程**（`@Page("admin")`，侧边栏 id `config`）：该内容区顶部为[横向工作流](<PipelineWorkflowView.kt>)（仅配置区渲染，不跨运维页面），VAD → ASR → NLU → LLM → TTS 为模块，头尾及衔接共六个可点击圆点；独立配置页复用同一组件。轨道宽按 `adminContentWidth` 自适应（桌面 ≥1280 无横向滚动，不设最大上限），窄屏横向滚动，不能退回纵向节点轨道/下拉。点击模块/圆点进入「配置 / 测试」两个官方 Tabs（官方 `indicatorInTabItem` 文字下滑块样式），各自固定左菜单、右内容；测试左菜单选择单个动作。无配置搜索框，分区只能经工作流或侧边栏到达。工作区小于 700 宽保留 92 宽左菜单，操作行靠右并可换行。布局见 [ConfigTabs](<ConfigTabs.kt>)及[工作区菜单](<ConfigWorkspaceViews.kt>)，位置投影见 [PipelineWorkflow](<PipelineWorkflow.kt>)，功能仍由 [PipelineFlow](<PipelineFlow.kt>)声明。只持有一个 `ConfigSelection(sectionId,nodeId?,workflowPositionId?)`，top/left 选择均由它派生；tab/status/action 是局部展示维度。空圆点只说明暂无扩展，不创建伪配置或测试。**全局配置是侧边栏独立一级菜单**（id `settings`，[renderGlobalConfigLayout](<ConfigWorkspaceViews.kt>)）：只放没有流程节点的 section（能力总览 / 预设 / AIUI 全链路 / 服务 / EchoMuse 接入 / MCP / OTA），无工作流轨道、无配置/测试 Tab，与「语音流程」共用同一份表单与同一个「保存配置」动作（`ConfigSections.global` + `ConfigFormState.openWorkflowPage/openGlobalPage` 各自记住停留位置）。带流程节点的 section（含人格 / 记忆 / 音频入口与输出）只在「语音流程」里改，**不要**把同一 section 同时挂进两套导航。全局设置、草稿、保存冲突和独立请求凭据保持原契约，不得恢复第二份 active 状态：

| Section | 数据源 | 回答什么问题 |
|---|---|---|
| 概览 | `GET /api/stats` + `/api/plugins` | 设备在线数、活会话、11 个能力健康（degraded/failed 可点击跳配置） |
| 测试台 | —（仅 macOS 客户端） | 依赖原生麦克风/音频链路，`NavDescriptor.macOnly`：web 不显示该菜单；端到端语音测试用 macOS 客户端 |
| 设备 | `GET /api/devices` | 连过的设备、协商参数、上次会话为什么断；消息注入、个性化配置入口 |
| EchoMuse | `GET /api/echomuse/devices` + 管理台 WS `/api/admin/ws` | 批准制设备登记（在线/离线、版本、base_os）；设备卡 = 标题行（状态点+删除）+ **六 tab**（状态 / 活动 / 配置 / 控制台 / 更新 / 日志，未批准只有单页审批）；状态 tab 逐行设备事实（缺测 `—` + 副标）、资源三块、音量下发、蓝牙摘要、系统诊断折叠；配置 tab = 网络连接三块 + Stage 01–08 + 「下发配置」（部分更新：留空不发该键）；更新 tab 只读版本 + OWW 资产五态 + 维护（重启二次确认/复制诊断/重装 VAD），**零 OTA 文案**；时间戳本地格式化；页面停留期间经管理台 WS 事件推送实时刷新（断线自动重连），无轮询 |
| 会话 | `GET /api/sessions` | 此刻活着的连接与阶段（聆听/思考/说话） |
| 对话记录 | `GET /api/conversations` | 最近聊了什么（内存环形，非持久化） |
| 用量 | `GET /api/usage` | 令牌去向、记忆/人格把上下文撑大了多少 |
| 延迟 | `GET /api/metrics` | 整轮与各段首包延迟 p50/p95、错误率（内存态） |
| 语音流程 | `/api/config` 等 | 五个核心阶段 + 连接点分支（唤醒 / 指令 / 人格 / 记忆 / 音频入口与输出）的节点参数与节点测试 |
| 全局配置 | `/api/config` 等 | 没有流程节点的部署级参数：能力总览、预设、AIUI 全链路、服务、EchoMuse 接入、MCP、OTA；与语音流程共用一份表单、一次 `POST /api/config` |
| 文件 | `GET /api/files` + POST mkdir/upload/delete | 服务端存储目录浏览与管理：新建文件夹、上传模型/文件（`oww-models/` 即 EchoMuse 自定义唤醒词模型的存放处）、删除（二次确认，受保护文件拒绝） |

> ⚠️ 设备/会话/对话记录/延迟是**进程内存态**：重启清零、`online` 由会话生命周期派生、对话只留最近若干轮。长期留存请开启 `[memory]`。
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
| `GET /api/metrics`、`GET /api/usage` | 延迟大盘 / 令牌用量（内存态） |
| `GET /api/files?path=`、`POST /api/files/{mkdir,upload,delete}` | 服务端存储目录浏览与管理：列目录（服务端目录优先排序）/ 建目录（409=同名文件占用）/ 原始字节上传（409 需显式覆盖、413 超限）/ 删除（目录递归；data.db 等受保护文件 403）；GET token 走查询串、POST 走 Bearer，错误统一 `{"error":"..."}` |

EchoMuse 实现按职责分为 [列表与配置状态](<EchoMuseState.kt>)（列表/审批/WiFi 配网闭环 + 管理台 WS 订阅 [AdminRealtime](<AdminRealtime.kt>)、[enterTab](<EchoMuseState.kt>) 是六 tab 唯一入口）、[设备行模型](<EchoMuseDeviceRow.kt>)（纯数据 + 展示文本）、[音乐面状态](<EchoMuseMusicState.kt>)（DLNA 扫描/浏览/播放/停止，独立写动作锁）、[详情卡分区状态](<EchoMuseDetailState.kt>)（六 tab、进 tab 懒加载、runtime/系统信息/音量/OWW 资产/控制台会话/单操作闸门）、[配置草稿](<EchoMuseConfigDrafts.kt>)（Stage 01–07 的 48 个字段 + `buildConfigBody` 部分更新语义）、[日志状态](<EchoMuseLogsState.kt>)（固定来源白名单 + 读取 + 复制）、[控制台状态机](<EchoMuseConsoleState.kt>)（WS 通道 + 六态）、[控制台协议字面量与用户文案](<EchoMuseConsoleProtocol.kt>)（`ConsoleWire` / `ConsoleCopy`，跨端契约，⛔ 改名即协议破坏）、[控制台文本管线](<EchoMuseConsoleText.kt>)（纯函数：ANSI 剥离/`\r` 覆盖/双上限）、[设备卡（只有标题行）](<EchoMuseDeviceCard.kt>)、[六 tab 外壳与审批单页](<EchoMuseDetailTabs.kt>)、[状态 tab](<EchoMuseStatusTab.kt>)、[活动 tab](<EchoMuseActivityTab.kt>)、[配置 tab（网络连接三块 + 下发条）](<EchoMuseConfigTab.kt>)、[配置 Stage 01–08](<EchoMuseConfigStages.kt>)、[更新 tab（只读版本 + 资产五态 + 维护）](<EchoMuseUpdatesTab.kt>)、[控制台 tab（输出区 / 输入行 / 六态）](<EchoMuseConsoleTab.kt>)、[等宽字体族名（`expect` + 4 个 actual）](<AdminFonts.kt>)、[日志 tab](<EchoMuseLogsTab.kt>)、[音乐 Stage 08 内容](<EchoMuseMusicPanel.kt>)、[页面壳体](<EchoMusePage.kt>)、[鉴权与编码](<EchoMuseAccess.kt>)：
- 六 tab 是唯一视图入口：`tabBar(EM_DETAIL_TABS)` + 六个 `vif({ detail.tab == … })`，**不许**构建期 `when(tab)`；tab 宽按 `echoMuseDetailChrome` + `(内容宽/6).coerceIn(44f, 96f)` 算，`indicatorInset` 必须同步缩 `(tabW/3f).coerceAtMost(30f)`（44 宽的 tab 上 30/30 会算出负宽度）。未批准设备只渲染审批单页（Action-required 横幅 + 设备号/IP/首次接入 + label + 「批准并加入」），批准后才出六 tab。
- 状态 tab 里**每个可能取不到的字段都以 lambda 传给 `infoRow({label},{value})`**，缺测一律 `—`（不填 0、不估算）；`Latency` 恒 `—`+「未测量」（官方由控制器 ping，本服务端不测）、`Flash 磨损` 恒 `—`+「未采集」（固件不上报）；物理静音与输出音量是**两行**（三态，不与 0 混）；`stats` 整块缺失时只说「等待设备上报…」；「系统诊断（原始输出）」默认收起。
- 配置 tab 的 Stage 01–08 严格只画 `admin.rs::SETTINGS_KEYS` 内的键，Stage 头必须写清「为什么比官方少」（缺白名单键 / 缺采集 / 官方那部分是控制器侧渲染）；Stage 07 渲染 `sendspinEnabled`/`sendspinName`/`sendspinUnpaired`（40 键含它，2026-10 起）；Stage 08 是「音乐（本服务端扩展，官方 dashboard 无此分组）」。
- 进入管理区不自动读设备；加载/失败/操作结果就地呈现，日志与系统输出可滚动及复制诊断内容。断线标历史；列表/详情/runtime 的 `session_id` 是当前 control 的观测身份，变化时清旧请求代际、busy、重启确认并标旧诊断为历史；runtime 仅设备号与会话都匹配时视为当前快照，双空兼容旧服务端、不将空当通配符。
- `configureAccess(token, base)` 在加载前传入；切 base 或 token 清管理目标和输出，列表/长挂/写操作通过代际丢弃旧连接回包。
- 鉴权语义（产品决策：**局域网默认可管理**）：`server.expected_token` 为空（默认）= 管理面免 token 可用（系统操作/管理接口直接放行，不再要求双 token 非空）；配置了 token 即收紧为必须鉴权——web 用 `?token=URL编码后的管理令牌` 进入，不附裸查询参数，不在诊断内容中复制 token；macOS 服务器设置提供密码型管理令牌输入，只保留在当前 Pager 会话，不落 SP/配置。设备侧看 `echomuse.token`：空 = 局域网信任设备（shell/BLE 通道不要求设备令牌），非空 = 设备必须带 `X-Em-Token`。请求凭据由 [AdminAccess](<AdminAccess.kt>)统一读取，不得复用配置草稿 expectedToken；受保护真实服务仍需独立联调。
- 唤醒设置以固件类型发送：`owwOnDevice=off|shadow|on`、`wakeSoundLevel=quiet|medium|loud`，旧 bool/number 只在读取时迁移（旧数字统一 medium）；阈值滑杆按官方量程 0.1～0.975、步进 0.025、Precise↔Eager 反向（常量与映射在 [EchoMuseWakeWord](<EchoMuseWakeWord.kt>)），轨外历史值读数保真、把手钉端点、拖动才落轨，保存仍拒非正数（服务端 `(0,1]`）；shadow 为官方诊断档、仅当前已处于该档时显示为选项，保存/重填后按当前值重建选项；提示音档位仅开关打开时显示；只发送本表单字段，服务端按部分更新合并，保留 BLE 等其他设置。唤醒词模型显示与选择规则在 [EchoMuseWakeWord](<EchoMuseWakeWord.kt>)（锚定上游默认 `hey_jarvis_v0.1`、官方四款与 display_name 规则）：存储为空即设备默认 hey jarvis 并明示，下拉列官方四款 + 服务端 `oww-models/` 已上传自定义模型 + 已存自定义值（[buildOwwOptions] 纯函数去重），保存恒发非空生效模型名（设备 config 对空 `owwModel` 不生效，恢复默认须显式发默认模型名）。卡片唤醒面板提供「管理模型文件」入口跳「文件」页 `oww-models/` 目录；**上传 ≠ 安装**——设备只认本地已装模型，未安装的模型选了也听不到唤醒词。
- 灵敏度滑杆必须给 `Slider` **显式宽高**（宽度由 [echoMuseSliderWidth](<EchoMuseConfigStages.kt>) 按内容区宽算出，不能用 `flex(1f)`）：官方 Slider 是组合组件，渲染时读自身 attr 的宽高，`flex` 不参与该判定，读不到就 `throwRuntimeError`（日志「Slider组件需要设置宽度和高度」）并中断所在面板的构建事务——实测表现为该 Stage 整片空白（只剩卡片标题），同页其它 Stage 不受影响。六 tab 布局下 `echoMuseSliderChrome` 的扣减项已去掉旧左菜单的定宽 150 + 1px 分隔线 + `panelPaddingLeft`（合计 167f），否则窄窗会算得过窄。
- 设备控制台（`GET .../devices/{id}/console`，WS 长驻）**不进 [AdminRealtime](<AdminRealtime.kt>) 共享组**：它是单设备单会话的独占租约（服务端与一次性操作共用一把槽），共享连接会把「另一个标签页已占住」变成静默复用；也**只手工重连**（自动退避 = 悄悄开一条新 root shell）。六态 `connecting/ready/offline/busy/closed/unsupported` 与全部用户文案在 [EchoMuseConsoleState](<EchoMuseConsoleState.kt>) 的 `ConsoleCopy`（唯一出处，与设计 §8.3／服务端 `console_frames.rs` 逐字一致）；`PLATFORM_WS_SUPPORTED=false`（Android）直接 `unsupported`、不建连不重试。帧协议字面量（`type`/`error.code`/`closed.reason`/`notice.code`/`ready.mode`）集中在同文件 `ConsoleWire`，任何一侧改名都是协议破坏。链路是 **pty 终端**：服务端发 `shell_open{pty:true}`（不再是不带 pty 的 pipe），设备回拨后若先发 `shell_meta{pty}` 则原样转给浏览器（控制帧不进终端画面）；一次性操作仍走 pipe，两个 `SHELL_OPEN` 常量⛔ 不许合并（pty 回显会污染 md5 校验与哨兵匹配）。
- 控制台输出**只在前端一处净化**：服务端只做 UTF-8 边界对齐，ANSI CSI/OSC（含跨 chunk 被切开的半截）、`\r` 就地覆盖、`\t`→4 空格、非法字节→`�` 全部在 [EchoMuseConsoleText](<EchoMuseConsoleText.kt>)（零 Kuikly 依赖，由 `:adminCoreTests` 的 `EchoMuseConsoleTextTest` 直驱）；行/字符双上限 2000 行 / 20 万字符、单行 4000 字截断，刷新 200 ms 合并。
- 控制台输出区是 **`Scroller` + 单个 `Text`**（`fontFamily(ADMIN_MONO_FONT)` + `AdminType.micro`）：⛔ 不用 `List`/`LazyColumn`（2000 item 的数据同步成本、`\r` 覆盖当前行的语义、还要自己算 item 高度才能 `scrollToPosition` 到底）。自动滚底靠官方 `contentSizeChanged` + `setContentOffset(0, 内容高−视口高)`，**不用 `setTimeout` 轮询**；`scroll` 事件按「贴底容差 12pt」开关 `autoScroll`——上翻即暂停、滚回底部自动恢复（漏了这条，用户看历史时会被一直拽回底部）。`viewHeight` 由 `scroll` 事件回填，`contentSizeChanged` 只给内容高。
- 控制台界面是**终端**（对齐官方 `dashboard.jsx` 的 `Shell`）：整块 `TERM_BG` 深色块（`AdminToken.TERM_*` 是终端配色的唯一真源）+ 等宽正文 + `❯` 提示符 + 无壳输入行（[terminalInput](<ui/Form.kt>)），顶栏只有 `shell — <deviceId>` 与「中断/清屏/重连」文字键。⛔ 不再渲染行模式横幅、状态表格、首次进入说明、「关于行模式」折叠区与四按钮动作行——那些把终端退化成了「能打字的日志查看器」。
- 提示符与回显归**设备**：`shell_open{pty:true}` 之后由设备 pty 上的 mksh 自己打印提示符、回显与行编辑，前端不伪造提示符、不做行编辑。老固件忽略 `pty` 仍开 pipe（无提示符），写 stdin / 读输出不受影响。
- `执行`/`中断`/`清屏`/`重连` 是可用性兜底：H5 的 `Input.inputReturn` 官方标注「实现中」可能不触发，没有可见按钮 web 端就没有提交入口。`LINE_MAX` 是 4096 **字节**（中文 3 字节/字），超长在终端内提示并置灰「执行」。
- 控制台文案唯一出处仍是 `ConsoleCopy`（`internal`，视图复用同一条 `insecure` 警告）：行模式专属的 `ROOT_BANNER`/`FIRST_ENTER`/`ABOUT_*`/`IN_USE_HINT` 已随界面一并删除，⛔ 不要再加回来——终端里常驻大段说明会挤掉输出区。⛔ UI 文案里不出现 markdown `**`。
- `closed{reason, detail}` → 文案的分派**只在 [ConsoleHint.closed](<EchoMuseConsoleProtocol.kt>) 一处**（纯函数、零 Kuikly 依赖，由 `EchoMuseConsoleClosedTest` 直驱）：⛔ 不要在状态机里再写一份 `when(reason)`。`reason=internal` **必须**拼上 `detail`——`console.rs` 的 `DETAIL_DIAL_TIMEOUT`「设备未在 15 秒内回拨 shell 通道」/`DETAIL_DIAL_CLOSED`「设备回拨通道已关闭」是这条失败唯一的信息源，丢掉就等于把所有内部错误都说成「服务端已退出或网络中断」，排查方向直接被带偏；反过来 `error{code:"internal"}` 的 `error` 字段服务端已带 `COPY_INTERNAL` 前缀，**不许**再拼 `INTERNAL_PREFIX`（否则双前缀）。`replaced`（槽被非预期接管）与 `superseded`（设备重连）是两件事，文案不许合并。
- 等宽字体是 `expect val ADMIN_MONO_FONT`（`js`/`android`=`monospace`、`ios`/`macos`=`Menlo`），**不打包字体文件**；族名未命中时平台自动回退，所以 UI ⛔ 不依赖字符宽度对齐（列对齐用固定像素 `View`，不用空格）。新增该 `expect` 必须同时补齐 4 个 actual，否则少 actual 的那一端编译失败。
- 更新 tab 的资产五态来自 `GET .../oww/state`（只读，**读不到设备不是缺失** → `unknown`），进 tab 拉一次并 30 s 轮询、离开立刻停；它不占单操作闸门（闸门只在 [EchoMuseDetailState.run](<EchoMuseDetailState.kt>) 一处）。

「文件」实现按职责分为 [纯逻辑](<FilesModel.kt>)（相对路径校验与服务端同规则镜像、清单解析安全兜底、尺寸/`yyyy-MM-dd HH:mm` 本地时间格式化——零 Kuikly 依赖，由普通 JVM 测试模块 `:adminCoreTests` 直接回归）、[目录状态](<FilesState.kt>)、[页面](<FilesPage.kt>) 与 [平台选文件桥](<UploadFilePicker.kt>)：
- 写动作只走服务端 POST 端点（`mkdir`/`upload`/`delete`，即 NetworkModule GET/POST 约束下的别名）；GET 把 token 编进查询串、POST 走 Bearer；`{"error":"..."}` 文案就地透出（409 同名不静默覆盖、403 受保护、413 超限）。
- 删除一律**行内二次确认**（先点武装「再次点击确认删除」，换目录/刷新即复位）；目录删除由服务端递归，受保护文件不给删除按钮。
- 上传平台矩阵：**web（隐藏 `<input type=file>` + fetch 流式，独立 `window.__xzAdminUploadFile` 槽位）与 macOS（复用 XiaoZhiModule 通用文档选择器 + `httpRequestBinary`，180s 超时）可用；android/ios 回调"当前平台不支持上传"**。选文件桥与 OTA 固件选包同型但完全独立（槽位/字节缓存分离，互不串包）。
- EchoMuse 设备列表加载成功后顺带静默拉取 `oww-models/` 的 `.onnx` 文件名（`modelFilesRefresh`，失败保持现状），唤醒词下拉据此追加自定义模型。

BLE 独立分为 [纯校验/codec/确认指纹](<EchoMuseBleCodec.kt>)、[请求与状态](<EchoMuseBleState.kt>)、[面板](<EchoMuseBleCard.kt>)：
- 已批准设备可展开单设备面板；在线、`ble_connect` 和非空当前会话才可请求，GATT 还要求每设备扫描/连接都开启；服务端 `ble_proxy` 与双 token 是额外门控，不能由页面开启。无 session 的旧服务端失败关闭，不复用 emOS runtime 双空兼容策略。
- 广播快照可选择 public/random MAC 或手填；服务树展示 UUID/声明 handle/value_handle/props 与描述符。特征读写必须使用 value_handle，描述符只用其 handle；HTTP200 中 result.ok=false 仍是失败，slots 按 t=slots 判断；read 必须显式字符串 value 且合法 base64，缺测不能假空值。⚠️ 手填 MAC / 切地址类型的函数名是 `applyAddress`/`applyAddressType` 而**不是** `setAddress*`：`address`/`addressType` 是 `var`，Kotlin 会为它们生成同签名的 `setAddress(String)`/`setAddressType(Int)`，同名函数在 JVM/Android 目标直接撞「Platform declaration clash」——`js`/`macos` 不报，Android 编译才红。
- hex 非空、最多512字节、只允许ASCII空白分隔的完整hex字节；转换标准base64。写入二次确认捕获设备、session、外设MAC/地址类型、handle、字节与response。任何草稿变化清确认；外设选择变更丢旧回包但不解除在途busy，编辑不可能撤销已发送操作；bridge设备/base/token/session/在线变化使旧代际失效。独立请求序号仅释放自己的busy。
- response=true成功只代表ATT接受，response=false仅代表设备发送，都不是IoT业务动作完成证明；CCCD UUID 0x2902由用户显式选择描述符handle，普通写0100/0200/0000，不猜handle、不新增subscribe协议。GET state仅手动读取有界notify/disconnected窗口，无后台高频轮询，也不承诺实时或完整历史。
- BLE设置仅POST部分更新bleProxyEnabled/bleProxyConnections，沿用旧settings接口不附session元数据（前端代际仍检查）；结果只说明已保存/在线推送排队，不假定固件已应用。广播、服务树、原始输出均有界滚动。纯函数由普通测试模块 `:bleProtocolTests:jvmTest` 直接引用同一生产 codec 验证，无 Kuikly/原生宿主依赖，且不进入生产 bundle；跨端编译不等于真实外设联调。

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
