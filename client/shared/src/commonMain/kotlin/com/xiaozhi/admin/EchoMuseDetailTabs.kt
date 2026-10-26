package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager

/**
 * 设备卡内容区：**未批准 → 单页审批**；已批准 → `tabBar` 六标签 + `vif` 分发。
 *
 * ```
 * ┌──────────────────────────────────────────────────────────┐
 * │ 状态 │ 活动 │ 配置 │ 控制台 │ 更新 │ 日志                    │ ← tabBar（等宽 6 项）
 * ├──────────────────────────────────────────────────────────┤ ← splitColumn 自动插的分隔线
 * │ 当前 tab 内容（vif 切换，构建期不许 when(tab)）              │
 * └──────────────────────────────────────────────────────────┘
 * │ 操作进行中 / 错误 / 结果（跨 tab 共用同一处反馈）             │
 * └──────────────────────────────────────────────────────────┘
 * ```
 *
 * 为什么从 `menuPaneRow`（左菜单）改成 `splitColumn`（上 tab）：左菜单 + 绝对定位竖线
 * `dividerVAt` 的锚点来自**定宽**菜单列，一旦要响应式收放就得算锚点；`splitColumn` 的
 * 分隔线是 stretch 版，那一整类"锚点算错"的布局风险直接消失（设计 §6.1/§6.6）。
 *
 * ⚠️ 三条 Kuikly 铁律在本文件的落点：
 * 1. tab 分发一律 `vif({ detail.tab == "状态" }) { … }`，**不许**构建期 `when(tab)`；
 * 2. 渲染函数在 `vif` 的 lambda 体内**非限定**调用（`ctx.echoMuseStatusTab(…)` 会把节点
 *    挂到 Pager 根容器，布局逃逸）；
 * 3. 所有按钮的 `enabled`/`loading`/`dynamicText` 都是 lambda（构建期求值不刷新）。
 */

/** 2 汉字 @14pt(28) + 左右留白 16 = 44；再窄就裁字。 */
private const val EM_TAB_MIN_W = 44f

/** 上限 96：再宽六个 tab 会撑爆卡片（等宽前提，见 `tabBar` 文档）。 */
private const val EM_TAB_MAX_W = 96f

/**
 * 详情卡内容区宽度：页面左右留白 + 卡片内边距。
 *
 * 新布局没有卡内左菜单，故比旧 `echoMuseSliderChrome` 正好少 167f
 * （旧左菜单定宽 150 + 1px 分隔线 + `panelPaddingLeft` 16）。
 */
private fun echoMuseDetailChrome(ctx: Pager): Float =
    AdminSpace.xl * 2 + (if (workspaceIsCompact(ctx)) AdminSpace.sm else AdminSpace.cardPadding) * 2

/**
 * 六等分后的 tab 宽；clamp 到 [EM_TAB_MIN_W, EM_TAB_MAX_W]。
 *
 * `44×6 = 264` 是卡片还看得见的最低宽度；低于这个数卡片本身已不可用，所以下界钉死 44。
 * `Tabs` 不提供横向滚动（官方 `tabs.md` 只有 scrollParams/indicatorInTabItem），
 * 所以必须靠算术保证装得下，而不是靠滚动兜底；也**不做**"窄窗隐藏部分 tab"——
 * 官方六 tab 是信息架构的一部分，藏起来等于丢功能。
 */
private fun echoMuseTabWidth(ctx: Pager): Float =
    ((adminContentWidth(ctx) - echoMuseDetailChrome(ctx)) / 6f).coerceIn(EM_TAB_MIN_W, EM_TAB_MAX_W)

internal fun ViewContainer<*, *>.echoMuseDetailTabs(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
    onOpenModelFiles: () -> Unit = {},
) {
    // 未批准只有一页：审批单（官方 Action-required 横幅 + 三行设备事实 + label + 批准）
    vif({ !row.approved }) { echoMuseApprovalPage(row, state, toast, ctx) }
    vif({ row.approved }) {
        splitColumn(trailingGap = AdminSpace.sm) {
            auto {
                tabBar(
                    tabs = EM_DETAIL_TABS,
                    selectedIndex = { EM_DETAIL_TABS.indexOf(state.detail.tab).coerceAtLeast(0) },
                    onSelect = { index ->
                        val label = EM_DETAIL_TABS.getOrNull(index) ?: EM_TAB_STATUS
                        state.enterTab(row, label, ctx)
                    },
                    itemWidth = echoMuseTabWidth(ctx),
                    // 指示条 inset 必须跟着 tab 宽一起缩，否则 44 宽的 tab 上
                    // left=30/right=30 会算出负宽度（§6.6 明写）
                    indicatorInset = (echoMuseTabWidth(ctx) / 3f).coerceAtMost(30f),
                )
            }
            auto {
                vif({ state.detail.tab == EM_TAB_STATUS }) { echoMuseStatusTab(row, state, toast, ctx) }
                vif({ state.detail.tab == EM_TAB_ACTIVITY }) { echoMuseActivityTab(state) }
                vif({ state.detail.tab == EM_TAB_CONFIG }) {
                    echoMuseConfigTab(row, state, toast, ctx, onOpenModelFiles)
                }
                // 控制台：会话本身由 EchoMuseDetailState.enterTab 打开（进 tab 开会话、离 tab 关会话）
                vif({ state.detail.tab == EM_TAB_CONSOLE }) {
                    echoMuseConsoleTab(row, state, toast, ctx)
                }
                vif({ state.detail.tab == EM_TAB_UPDATES }) { echoMuseUpdatesTab(row, state, toast, ctx) }
                vif({ state.detail.tab == EM_TAB_LOGS }) { echoMuseLogsTab(state, toast) }
            }
        }
        echoMuseTabFeedback(state)
    }
}

/**
 * 跨 tab 的操作反馈：`busy` / `error` / `result` 三个 observable 在六 tab 之外渲染，
 * 切到哪个 tab 都能看到上一次操作的结果（原「设备管理」面板同样的处理）。
 */
private fun ViewContainer<*, *>.echoMuseTabFeedback(state: EchoMuseState) {
    val detail = state.detail
    vif({ detail.busy.isNotEmpty() }) {
        textNote(text = { "操作进行中（${detail.busy}），请等待设备响应…" }, micro = true)
    }
    vif({ detail.error.isNotEmpty() }) {
        textNote({ detail.error }, { "error" }, micro = true)
    }
    vif({ detail.result.isNotEmpty() }) {
        textNote(text = { detail.result }, micro = true)
    }
}

// ============================================================
// 未批准：审批单页
// ============================================================

/**
 * 审批单页（官方 2129-2165 的 Action-required 卡）。
 *
 * 未批准设备**只能**看到这一页：批准前它没有任何远程管理权限（服务端 `system_gate`
 * 一律 403），所以给六个 tab 只会让人点出一片"操作被拒绝"。三行事实（设备号/IP/首次接入）
 * 是运维核对"这台到底是不是我等的那块板"的唯一依据，不能省。
 */
private fun ViewContainer<*, *>.echoMuseApprovalPage(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    textNote(
        "这台设备还没批准：批准前不能做任何远程管理（状态、配置、日志、控制台全部不可用）。" +
            "核对下面的设备号与 IP 是你要接入的那块板，再批准。",
        tone = "warn",
        micro = true,
    )
    infoRow({ "设备号（Serial）" }, { row.deviceId }, topGap = AdminSpace.sm)
    infoRow({ "IP" }, { row.ip.ifEmpty { EM_DASH } })
    infoRow({ "首次接入" }, { formatEpochSeconds(row.firstSeen).ifEmpty { EM_DASH } })
    infoRow({ "最近接入" }, { formatEpochSeconds(row.lastSeen).ifEmpty { EM_DASH } })
    labeledField(
        "设备名称（label，可留空）",
        { state.approveDraft },
        { state.approveDraft = it },
        "如 客厅音箱",
    )
    actionRow {
        primaryButton(
            "批准并加入",
            loading = { state.actionBusy },
            loadingText = "批准中…",
            icon = IconToken.Check,
        ) {
            state.approve(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
                if (ok) state.refresh(ctx)
            }
        }
    }
    textNote(
        "批准后设备会出现在列表里并可远程管理；名称留空则一直显示设备号。",
        micro = true,
    )
}
