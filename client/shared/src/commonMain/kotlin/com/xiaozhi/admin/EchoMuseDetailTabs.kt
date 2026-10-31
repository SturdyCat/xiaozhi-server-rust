package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.dialog.Dialog
import com.gearui.components.dialog.DialogAction
import com.gearui.components.dialog.DialogActionRole
import com.gearui.components.dialog.DialogContent
import com.gearui.components.tabs.Tab
import com.gearui.components.tabs.Tabs
import com.gearui.components.tabs.TabsSize
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledField

/**
 * 设备卡内容区：**未批准 → 审批（改名 sheet）**；已批准 → 六 tab 容器 + 分发。
 *
 * ```
 * ┌──────────────────────────────────────────────────────────┐
 * │ 状态 │ 活动 │ 配置 │ 控制台 │ 更新 │ 日志                    │ ← GearUI Tabs(scrollable)
 * ├──────────────────────────────────────────────────────────┤
 * │ 当前 tab 内容（Compose 的 when(tab) —— 组合期条件渲染）      │
 * ├──────────────────────────────────────────────────────────┤
 * │ 操作进行中 / 错误 / 结果（跨 tab 共用同一处反馈）             │
 * └──────────────────────────────────────────────────────────┘
 * ```
 *
 * ## 六个 tab 一个都不能藏（本任务的硬约束之一）
 *
 * `Tabs(scrollable = true)`：窗口再窄也只做**横向滚动**，绝不折叠成「更多 ▾」，
 * 也不按宽度隐藏后几个——官方六 tab 是信息架构的一部分，藏起来等于丢功能。
 * 旧 DSL 因为 `Tabs` 不提供横向滚动，只能靠算术把六个 tab 均分进卡片宽度
 * （`EM_TAB_MIN_W = 44` 的下界即来自那里）；GearUI 的 `Tabs` 自带 `GearLazyRow`
 * 滚动容器，**那套宽度算术与 `indicatorInset` 补偿一起删掉**，不用再赌"算得准"。
 *
 * ## 为什么这里可以 `when (tab)`（旧 DSL 里是明令禁止的）
 *
 * 旧实现的铁律是「tab 分发一律 `vif`，不许构建期 `when(tab)`」——那是 Kuikly DSL 的限制：
 * `when` 在构建期求值一次，observable 变了不会重建子树。Compose 里 `state.detail.tab` 是
 * 快照状态，`when` 就是**条件组合**：值一变即重组换子树，语义正好是我们要的。
 *
 * ## 未批准：审批改成「改名 sheet」
 *
 * 旧实现把审批铺成六 tab 之外的一整页表单（label 输入框常驻）。重写后：
 * 事实三行（设备号 / IP / 首次接入）留在卡内——那是运维核对"这块板是不是我等的那块"
 * 的唯一依据，不能藏进弹层；**命名与批准**收进 `Dialog.Host` 的 sheet，点「批准并命名…」
 * 才出现。理由：批准是不可逆的权限动作，需要一次明确的模态确认；平铺在卡片上时
 * 「批准并加入」按钮与下面的「删除记录」只隔几十像素，误点代价太高。
 */
@Composable
internal fun EchoMuseDetailTabs(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    onOpenModelFiles: () -> Unit = {},
) {
    if (!row.approved) {
        EmApprovalPanel(row = row, state = state, toast = toast)
        return
    }

    val pager = LocalAdminPagerOrThrow()
    val tab = state.detail.tab
    Tabs(
        items = EM_DETAIL_TABS.map { Tab(id = it, label = it) },
        selectedId = tab.takeIf { it in EM_DETAIL_TABS },
        onSelect = { label -> state.enterTab(row, label, pager) },
        modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.md),
        scrollable = true,
        showDivider = false,
        size = TabsSize.SMALL,
    )
    Column(modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.md)) {
        when (tab) {
            EM_TAB_STATUS -> EchoMuseStatusTab(row = row, state = state, toast = toast)
            EM_TAB_ACTIVITY -> EchoMuseActivityTab(state = state)
            EM_TAB_CONFIG -> EchoMuseConfigTab(
                row = row,
                state = state,
                toast = toast,
                onOpenModelFiles = onOpenModelFiles,
            )
            // 控制台会话由 EchoMuseDetailState.enterTab 打开（进 tab 开会话、离 tab 关会话）
            EM_TAB_CONSOLE -> EchoMuseConsoleTab(row = row, state = state)
            EM_TAB_UPDATES -> EchoMuseUpdatesTab(row = row, state = state, toast = toast)
            EM_TAB_LOGS -> EchoMuseLogsTab(state = state, toast = toast)
            else -> EmNote("未知 tab「$tab」：请刷新页面（tab 名与 EM_DETAIL_TABS 不一致）。")
        }
        EmTabFeedback(state = state)
    }
}

/**
 * 跨 tab 的操作反馈：`busy` / `error` / `result` 在六 tab 之外渲染，
 * 切到哪个 tab 都能看到上一次操作的结果（旧「设备管理」面板同样的处理）。
 */
@Composable
private fun EmTabFeedback(state: EchoMuseState) {
    val detail = state.detail
    if (detail.busy.isNotEmpty()) {
        EmNote("操作进行中（${detail.busy}），请等待设备响应…")
    }
    if (detail.error.isNotEmpty()) {
        EmWarn(detail.error, error = true)
    }
    if (detail.result.isNotEmpty()) {
        EmNote(detail.result)
    }
}

/**
 * 未批准设备的审批区：Action-required 横幅 + 四行事实 + 「批准并命名…」。
 *
 * 未批准设备**没有任何远程管理权限**（服务端 `system_gate` 一律 403），所以这里不画六个 tab
 * ——画了只会让人点出一片"操作被拒绝"。给的是"核对 → 命名 → 批准"这条唯一通路。
 */
@Composable
private fun EmApprovalPanel(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState) {
    val pager = LocalAdminPagerOrThrow()
    var sheetOpen by remember { mutableStateOf(false) }

    Alert(
        title = "这台设备还没批准",
        description = "批准前不能做任何远程管理（状态、配置、日志、控制台全部不可用）。" +
            "核对下面的设备号与 IP 是你要接入的那块板，再批准。",
        status = AlertStatus.WARNING,
        modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.md),
    )
    EmInfoRow("设备号（Serial）", row.deviceId, topGap = LocalAdminSpacing.current.sm)
    EmInfoRow("IP", row.ip.ifEmpty { EM_DASH })
    EmInfoRow("首次接入", formatEpochSeconds(row.firstSeen).ifEmpty { EM_DASH })
    EmInfoRow("最近接入", formatEpochSeconds(row.lastSeen).ifEmpty { EM_DASH })
    actionRow {
        Button(
            text = "批准并命名…",
            onClick = { sheetOpen = true },
            type = ButtonType.FILL,
            disabled = state.actionBusy,
        )
    }
    EmNote("批准后会出现在列表里并可远程管理；名称留空则一直显示设备号。")

    Dialog.Host(visible = sheetOpen, onDismiss = { sheetOpen = false }) {
        DialogContent(
            title = "批准并命名这台设备",
            message = "命名只影响管理台的显示（服务端 label 字段），不会改设备本身的名字。留空则显示设备号。",
            content = {
                Column(modifier = Modifier.fillMaxWidth()) {
                    labeledField(
                        label = "设备名称（label，可留空）",
                        value = state.approveDraft,
                        onValueChange = { state.approveDraft = it },
                        placeholder = "如 客厅音箱",
                        enabled = !state.actionBusy,
                    )
                }
            },
            actions = listOf(
                DialogAction(
                    text = "批准并加入",
                    role = DialogActionRole.PRIMARY,
                    enabled = !state.actionBusy,
                    onClick = {
                        state.approve(pager, row.deviceId) { ok, msg ->
                            toast.show(msg, if (ok) "ok" else "error")
                            if (ok) {
                                sheetOpen = false
                                state.refresh(pager)
                            }
                        }
                    },
                ),
                DialogAction(
                    text = "取消",
                    role = DialogActionRole.CANCEL,
                    onClick = { sheetOpen = false },
                ),
            ),
        )
    }
}
