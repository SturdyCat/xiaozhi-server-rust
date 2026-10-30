package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.cell.Cell
import com.gearui.components.cellgroup.CellGroup
import com.gearui.components.drawer.Drawer
import com.gearui.components.drawer.DrawerPlacement
import com.gearui.components.navbar.NavBar
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.Spacer
import com.tencent.kuikly.compose.foundation.layout.fillMaxHeight
import com.tencent.kuikly.compose.foundation.layout.fillMaxSize
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.size
import com.tencent.kuikly.compose.foundation.layout.width
import com.tencent.kuikly.compose.foundation.layout.widthIn
import com.tencent.kuikly.compose.foundation.shape.RoundedCornerShape
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.AdminNav
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.contracts.NavDescriptor
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.AdminScopeHost
import com.xiaozhi.admin.kit.AdminViewport
import com.xiaozhi.admin.kit.GearFieldHelpDialog
import com.xiaozhi.admin.kit.GearUiBreakpoints
import com.xiaozhi.admin.kit.dividerV

/**
 * 管理台壳层（Compose）——侧栏 + 标题栏 + section 宿主。
 *
 * ## 与旧 DSL 壳的关系
 *
 * 旧壳是 `AdminPage.body()` 里的一整棵 `View { }`（512 行）。Compose 无法嵌进 DSL 布局，
 * 所以 T06 的做法是"整页换壳"：`AdminPage` 改成 `ComposeContainer`，本文件接管壳的布局与
 * 导航，**12 个 section 先渲染"迁移中"占位**，真实内容由 T07–T11 逐个填进 [AdminSectionHost]
 * 的分发分支里。
 *
 * ## 布局与断点（数值一字不改，K9）
 *
 * ```
 * 宽屏（≥768）                      窄屏（<768）
 * ┌──侧栏 240──┬─── 内容区 ───┐      ┌──── 内容区（标题栏带汉堡）────┐
 * │ 分组标题    │ NavBar       │      │ NavBar  [☰] 标题      [保存]  │
 * │ 导航项      │ section      │      │ section                       │
 * └────────────┴──────────────┘      └───────────[抽屉 240]──────────┘
 * ```
 *
 * 断点与侧栏宽度直接取 [GearUiBreakpoints]（768 / 900 / 700 / 240），与旧壳的
 * `NARROW_BREAKPOINT` / `ADMIN_SIDEBAR_WIDTH` 同值。
 *
 * ## 为什么用 `onSizeChanged` 而不是 `pagerData.pageViewWidth`
 *
 * 旧实现读 `pagerData.pageViewWidth`——一次性取值，窗口 resize **不触发**重排；
 * Compose 侧把视口收进 [AdminViewport] 状态并经 [LocalAdminViewport] 下发，
 * 窗口拖动跨断点会实时重排（比旧实现强，且是方案 D4 的要求）。
 *
 * ## 选中态（P0）
 *
 * GearUI 的 `Cell` 没有 `selected` 参数（它的选中语义属于 `Checkbox`/`Radio`）。
 * 侧栏用两个显式信号表达当前项：**品牌绿文字 + 左侧 3dp 品牌绿竖条**，
 * 与旧 DSL 侧栏（`menuListItem` MAIN 档）的视觉语言一致。
 */
@Composable
fun AdminShell(page: AdminPage) {
    // 视口与三个 CompositionLocal 由 AdminScopeHost 统一提供（与 ConfigPage 同一份实现）
    AdminScopeHost(pager = page, fieldHelp = page.fieldHelp) { viewport ->
        Row(modifier = Modifier.fillMaxSize()) {
            if (!viewport.isNarrow) {
                AdminSidebar(page)
                dividerV()
            }
            Column(modifier = Modifier.weight(1f).fillMaxHeight()) {
                AdminTopBar(page, narrow = viewport.isNarrow)
                AdminSectionHost(page, viewport)
            }
        }

        // 窄屏抽屉：GearUI Drawer（Overlay 架构，遮罩 + 位移动画 + 点遮罩关闭）。
        // 只在窄屏挂载——宽屏时侧栏常驻，抽屉不该存在（否则焦点会跑进隐藏面板）。
        if (viewport.isNarrow) {
            Drawer(
                visible = page.drawerOpen,
                onDismiss = { page.drawerOpen = false },
                placement = DrawerPlacement.LEFT,
                width = GearUiBreakpoints.SIDEBAR_WIDTH.dp,
                content = { AdminSidebar(page) },
            )
        }

        // 字段帮助弹层（ⓘ 的唯一实例）：每页根部挂一次，走 Popup.Host（不占布局）
        GearFieldHelpDialog()
    }
}

/**
 * 侧栏：分组标题 + 导航项 + 底部版本行。
 *
 * 宽屏常驻列与窄屏抽屉**共用本函数**——同一个组件，视觉才不会漂移（旧壳的同一条结论）。
 * 容器用 `ScrollView`（12 项 + 3 个分组标题在矮窗口里会超出高度）。
 */
@Composable
private fun AdminSidebar(page: AdminPage) {
    Column(
        modifier = Modifier
            .widthIn(max = GearUiBreakpoints.SIDEBAR_WIDTH.dp)
            .width(GearUiBreakpoints.SIDEBAR_WIDTH.dp)
            .fillMaxHeight()
            .background(Theme.colors.surface),
    ) {
        ScrollView(modifier = Modifier.weight(1f).fillMaxWidth()) {
            for ((group, items) in AdminNav.grouped(IS_MACOS)) {
                // CellGroup 负责：分组标题对齐、卡片圆角、行间分隔线（末行无分隔）
                CellGroup(items = items, title = group, key = { it.id }, itemContent = { item ->
                    AdminNavRow(page, item)
                })
            }
            // macOS「服务器设置」（T12 接线）：切换服务器基址 + 局域网自动发现；
            // 开关状态收口在 [AdminPage.serverSwitchOpen]，web/iOS/Android 不渲染。
            if (IS_MACOS) {
                ServerSwitchEntry(page)
            }
        }
        // 底部版本行：不参与导航，纯标注
        Text(
            text = "小智服务端 · 管理台",
            style = Theme.typography.caption,
            color = Theme.colors.mutedForeground,
            modifier = Modifier.padding(
                start = Spacing.lg,
                end = Spacing.lg,
                top = Spacing.sm,
                bottom = Spacing.lg,
            ),
        )
    }
}

/**
 * 单个导航项。
 *
 * `Cell` 的 `leading`/`trailing` 是可选插槽，null 时不占位——所以未选中项的标题
 * 仍然与分组标题左对齐（选中项多出一条 3dp 竖条 + 8dp 间隙，是有意的视觉加重）。
 */
@Composable
private fun AdminNavRow(page: AdminPage, item: NavDescriptor) {
    val selected = page.selectedSection == item.id
    // 两个配置菜单共用同一份草稿：任一菜单都能是"有未保存改动"的入口
    val dirty = (item.id == "config" || item.id == "settings") && page.form.dirty

    val leading: (@Composable () -> Unit)? = if (selected) {
        { NavSelectionBar() }
    } else {
        null
    }
    val trailing: (@Composable () -> Unit)? = if (dirty) {
        { NavDirtyDot() }
    } else {
        null
    }

    Cell(
        title = item.label,
        onClick = { page.selectSection(item.id) },
        titleColor = if (selected) Theme.colors.primary else null,
        leading = leading,
        trailing = trailing,
    )
}

/** 选中竖条：品牌绿 3×18。 */
@Composable
private fun NavSelectionBar() {
    Box(
        modifier = Modifier
            .width(3.dp)
            .height(18.dp)
            .clip(RoundedCornerShape(2.dp))
            .background(Theme.colors.primary),
    )
}

/** 未保存改动的小橙点（旧壳 `dot(warning)` 的等价物；GearUI 无独立 Badge 组件，用 8dp 圆点）。 */
@Composable
private fun NavDirtyDot() {
    Box(
        modifier = Modifier
            .size(8.dp)
            .clip(RoundedCornerShape(4.dp))
            .background(Theme.colors.warning),
    )
}

/**
 * 标题栏（GearUI `NavBar`）：标题 = 当前 section 文案（「语音流程」按编辑目标加设备号）。
 *
 * 窄屏时 leading 换成汉堡按钮（开抽屉）；宽屏不渲染汉堡——侧栏常驻，汉堡没有意义。
 * 右侧「保存配置」只在编辑目标为全局时出现（与旧壳同一条判定：设备 overlay 与全局
 * 是两条保存路径，不能混显）。
 */
@Composable
private fun AdminTopBar(page: AdminPage, narrow: Boolean) {
    val editable = page.selectedSection == "config" || page.selectedSection == "settings"
    val globalTarget = editable && page.form.editDeviceId.isEmpty()

    val leftWidget: (@Composable () -> Unit)? = if (narrow) {
        { HamburgerButton { page.drawerOpen = true } }
    } else {
        null
    }
    val rightWidget: (@Composable () -> Unit)? = if (globalTarget) {
        {
            Box(modifier = Modifier.padding(end = Spacing.lg)) {
                Button(
                    text = if (page.form.saving) "保存中…" else "保存配置",
                    onClick = {
                        page.form.save(page, page.apiBase) { msg, level -> page.showToast(msg, level) }
                    },
                    theme = ButtonTheme.PRIMARY,
                    type = ButtonType.FILL,
                    loading = page.form.saving,
                )
            }
        }
    } else {
        null
    }

    NavBar(
        title = titleOf(page),
        centerTitle = false,
        leftWidget = leftWidget,
        leftWidgetWidth = if (narrow) 44.dp else null,
        rightWidget = rightWidget,
        // 两个按钮（保存 / 重新加载）的槽宽由 Button 自身撑开，这里给足 96dp
        rightWidgetWidth = if (rightWidget != null) 96.dp else null,
    )
}

/** 44×44 触控目标 + 矢量汉堡图标（[IconToken.Menu]，与图标集同一套几何）。 */
@Composable
private fun HamburgerButton(onClick: () -> Unit) {
    Box(modifier = Modifier.size(44.dp), contentAlignment = Alignment.Center) {
        GearIcon(
            token = IconToken.Menu,
            size = 20.dp,
            tint = Theme.colors.foreground,
            onClick = onClick,
        )
    }
}

/** 标题：语音流程按编辑目标加设备号；其余取导航表文案。 */
private fun titleOf(page: AdminPage): String {
    if (page.selectedSection != "config") return AdminNav.label(page.selectedSection)
    return if (page.form.editDeviceId.isEmpty()) "节点管理" else "节点管理 · 设备 ${page.form.editDeviceId}"
}

/**
 * section 宿主（**迁移中的临时形态**）。
 *
 * T06 的验收口径是"12 项都能点、不空白、不崩溃"，所以这里对每个 section 渲染一张
 * 说明卡：告诉使用者当前是迁移占位、该 section 的真实界面在哪个任务落地。T07–T11
 * 会逐个把 `when` 分支换成真正的 Compose 实现（届时删除对应的占位入口）。
 */
@Composable
private fun AdminSectionHost(page: AdminPage, viewport: AdminViewport) {
    when (page.selectedSection) {
        // 「语音流程」：轨道 + 双 Tab 工作区（T07 落地）
        "config" -> Column(modifier = Modifier.fillMaxSize()) {
            PipelineWorkflowHeader(form = page.form)
            WorkflowWorkspaceLayout(
                form = page.form,
                meta = page.plugins,
                health = page.engineHealth,
                mcp = page.mcpTools,
                ota = page.ota,
                toast = page.toast,
            )
        }
        // 「全局配置」：全局 section 菜单 + 同一套卡片（T08 接手各段内容）
        "settings" -> GlobalConfigLayout(
            form = page.form,
            meta = page.plugins,
            health = page.engineHealth,
            mcp = page.mcpTools,
            ota = page.ota,
            toast = page.toast,
        )
        // 运维面（T09）：概览 / 设备 / 会话 / 对话记录 / 用量 / 延迟
        "overview" -> OpsOverviewPage(
            stats = page.stats,
            meta = page.plugins,
            health = page.engineHealth,
            // 就绪度横幅跳转：菜单归属由 form.onSectionRouted 统一按 section 判定
            onJumpSection = { section -> page.form.openConfigSection(section) },
        )
        "devices" -> OpsDevicesPage(
            devices = page.devices,
            form = page.form,
            // 「个性化配置」入口（P1-5）：以该设备为编辑目标跳「语音流程」
            //（editDeviceId 置位后 load 走「全局基线 + overlay」分支）
            onEditDevice = { id ->
                page.form.editDeviceId = id
                page.form.load(page, page.apiBase)
                page.selectSection("config")
            },
        )
        // 控制台：深色终端（K2 保留项，色值收敛在 kit/GearTerminal）
        // （2026-10-29 范围削减：sessions / conversations / usage / metrics 四页已随
        //  docs/plans/gearui-kit-scope-reduction.md §二 移除，分支一并删除）
        "console" -> ConsolePage(console = page.console, toast = page.toast)
        // 文件（存储面）：面包屑 + 上传 + 新建目录 + 删除二次确认
        "files" -> FilesPage(state = page.files, toast = page.toast)
        // EchoMuse 设备（T10）：顶部「官方设备管理页（/em/）」入口卡 + 原有六 tab 详情卡。
        // 官方页是按 EchoMuse 官方 dashboard 源码 vendor 的独立文档（web-only），
        // 旧六 tab 保留专有能力（DLNA 音乐 / BLE GATT / 唤醒词资产 / WiFi 闭环）。
        "echomuse" -> EchoMusePage(
            state = page.echomuse,
            toast = page.toast,
            isActive = { page.selectedSection == "echomuse" },
            onOpenModelFiles = {
                page.selectSection("files")
                page.files.openPath(page, OWW_MODELS_DIR)
            },
            officialEntry = { EmOfficialEntryCard(page) },
        )
        // 测试台（T12，mac-only）：五 tab 工作区（连接 / ASR 识别 / TTS 合成 / LLM 对话 / NLU 意图），
        // 播放控制下沉 XiaoZhiModule（Audio DSL 视图无法进 Compose 树，见 TestBenchPage.kt）
        "testbench" -> TestbenchSection(page)
        else -> Box(modifier = Modifier.fillMaxSize()) {
            SectionPlaceholder(
                title = AdminNav.label(page.selectedSection),
                detail = sectionDetail(page.selectedSection),
                viewport = viewport,
            )
        }
    }
}

/** 每个 section 的迁移去向（占位文案用；T07–T11 落地后随之删除）。 */
private fun sectionDetail(section: String): String = when (section) {
    "overview" -> "概览（能力总览 / 引擎健康）在 T07 迁移到 GearUI 卡片 + Table。"
    "devices" -> "设备列表与个性化配置入口在 T07 迁移。"
    "console" -> "控制台日志流（/api/logs）在 T09 迁移。"
    "files" -> "文件管理在 T11 迁移。"
    else -> "该 section 尚未迁移。"
}

/**
 * 占位卡：标题 + 一句去向说明 + 一行"数据源仍在线"的提示。
 *
 * 明确写出"这是迁移中"而不是留白或假表单——T06 之后到 T11 之前，管理台是**逐段拼装**的，
 * 用户看到的每一段要么是真的，要么明确说"还没搬"。
 */
@Composable
internal fun SectionPlaceholder(title: String, detail: String, viewport: AdminViewport) {
    ScrollView(modifier = Modifier.fillMaxSize()) {
        Column(modifier = Modifier.fillMaxWidth().padding(Spacing.xl)) {
            Text(
                text = title,
                style = Theme.typography.titleLarge,
                color = Theme.colors.foreground,
            )
            Spacer(modifier = Modifier.height(Spacing.sm))
            Text(
                text = "正在迁移到 GearUI 组件库",
                style = Theme.typography.bodyMedium,
                color = Theme.colors.mutedForeground,
            )
            Spacer(modifier = Modifier.height(Spacing.lg))
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .background(Theme.colors.secondary)
                    .padding(Spacing.lg),
            ) {
                Column {
                    Text(
                        text = detail,
                        style = Theme.typography.bodyMedium,
                        color = Theme.colors.secondaryForeground,
                    )
                    Spacer(modifier = Modifier.height(Spacing.sm))
                    Text(
                        text = "数据加载链路已在运行（本页 pageDidAppear 仍会拉取该 section 的数据源），" +
                            "迁移只替换渲染层，不改变接口与鉴权口径。",
                        style = Theme.typography.caption,
                        color = Theme.colors.mutedForeground,
                    )
                    if (viewport.twoColumn) {
                        Spacer(modifier = Modifier.height(Spacing.sm))
                        Text(
                            text = "当前视口 ≥900dp：迁移完成后该 section 会以双列布局呈现。",
                            style = Theme.typography.caption,
                            color = Theme.colors.mutedForeground,
                        )
                    }
                }
            }
        }
    }
}
