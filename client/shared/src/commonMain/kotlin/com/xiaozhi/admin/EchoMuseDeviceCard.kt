package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 一台 EchoMuse 设备卡片：**只有 header**，内容全在 [echoMuseDetailTabs]。
 *
 * ```
 * ┌──────────────────────────────────────────────┐
 * │ 标题  ● 在线 [待批准]              （删除 icon）│ ← header（删除走行内二次确认）
 * ├──────────────────────────────────────────────┤
 * │ 未批准 → 审批单页                             │ ← EchoMuseDetailTabs.kt
 * │ 已批准 → 六个 tab（状态/活动/配置/控制台/更新/日志）│
 * └──────────────────────────────────────────────┘
 * ```
 *
 * 本文件曾经是 615 行的"header + 卡内左菜单 + 七个面板"，2026-10（设计 §7-T04）按六 tab
 * 重组：左菜单、定宽菜单列、`echoMuseSliderChrome` 的扣减项全部消失，
 * 面板内容按 §6.2 归位表散到 `EchoMuseStatusTab`/`EchoMuseConfigTab`/`EchoMuseUpdatesTab`/
 * `EchoMuseLogsTab`。留在这里的只有「这台设备叫什么、在不在线、要不要删掉它」。
 *
 * ⚠️ Kuikly 铁律：渲染函数必须在目标容器闭包内**非限定**调用
 * （`ctx.echoMuseDetailTabs(…)` 会把节点挂到 Pager 根容器）。
 */

/** 一台 EchoMuse 设备卡片：header（标题 + 状态 + 删除）+ 六 tab 内容（EchoMusePage 调用）。 */
internal fun ViewContainer<*, *>.echoMuseDeviceCard(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
    onOpenModelFiles: () -> Unit = {},
) {
    splitCard(
        header = {
            echoMuseCardHeader(row, state)
            vif({ state.confirmDeleteId == row.deviceId }) { echoMuseDeleteConfirm(row, state, toast, ctx) }
        },
        body = { echoMuseDetailTabs(row, state, toast, ctx, onOpenModelFiles) },
    )
}

// ============================================================
// header：标题 + 在线状态 + 删除（icon 按钮 + 行内二次确认）
// ============================================================

/**
 * 卡片 header：走统一 [headerRow]（左标题 + 状态徽标 │ 右删除按钮），
 * 不手写 `flexDirectionRow + View { flex(1f) }` 的弹性占位。
 */
private fun ViewContainer<*, *>.echoMuseCardHeader(row: EchoMuseDeviceRow, state: EchoMuseState) {
    headerRow(
        title = row.title,
        meta = { echoMuseStatusMeta(row) },
        actions = { echoMuseDeleteButton(state, row.deviceId) },
    )
}

/** 标题右侧的状态元信息：在线/离线圆点 + 未批准时的「待批准」徽标。 */
private fun ViewContainer<*, *>.echoMuseStatusMeta(row: EchoMuseDeviceRow) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            marginLeft(AdminSpace.sm)
        }
        // 在线状态：绿点「在线」/ 灰点「离线」，文字色随状态
        dot({ if (row.online) AdminColors.accent else AdminColors.textPlaceholder })
        hGap(AdminSpace.xs)
        Text {
            attr {
                fontSize(AdminType.caption)
                color(if (row.online) AdminColors.accentTintText else AdminColors.textTertiary)
                text(if (row.online) "在线" else "离线")
            }
        }
        // 未批准：状态旁保留「待批准」徽标
        if (!row.approved) {
            hGap(AdminSpace.sm)
            statusBadge(state = { "busy" }, text = { "待批准" })
        }
    }
}

/**
 * 删除记录的 icon-only 按钮（[appButton] OUTLINE + danger + [IconToken.Delete]）。
 * 点击只进入行内二次确认（[echoMuseDeleteConfirm]），不直接删除。
 */
private fun ViewContainer<*, *>.echoMuseDeleteButton(state: EchoMuseState, deviceId: String) {
    appButton(
        "删除记录",
        ButtonVariant.OUTLINE,
        danger = { true },
        icon = IconToken.Delete,
        iconOnly = true,
        accessibilityLabel = "删除该设备记录",
    ) { state.confirmDeleteId = deviceId }
}

/** 删除记录的行内二次确认（header 下方）：确认/取消 + 口径说明。 */
private fun ViewContainer<*, *>.echoMuseDeleteConfirm(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    View {
        attr {
            flexDirectionColumn()
            paddingBottom(AdminSpace.sm)
        }
        actionRow {
            primaryButton(
                "确认删除",
                danger = { true },
                loading = { state.actionBusy },
                loadingText = "删除中…",
                icon = IconToken.Delete,
            ) {
                state.delete(ctx, row.deviceId) { ok, msg ->
                    toast.show(msg, if (ok) "ok" else "error")
                    if (ok) state.refresh(ctx)
                }
            }
            hGap(AdminSpace.sm)
            secondaryButton("取消", icon = IconToken.Close) { state.confirmDeleteId = "" }
        }
        textNote(
            "删除只清服务端登记（含批准/唤醒词设置），不影响设备本身；设备下次接入会重新登记。",
            micro = true,
        )
    }
}
