package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.avatar.Avatar
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonTheme
import com.gearui.components.button.ButtonType
import com.gearui.components.dialog.ConfirmDialog
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.statusBadge

/**
 * 一台 EchoMuse 设备卡片：**只有 header + 内容槽**，内容全在 [EchoMuseDetailTabs]。
 *
 * ```
 * ┌────────────────────────────────────────────────┐
 * │ (头像●) 标题                     [删除记录]      │ ← header（在线不打标；离线/待批准才打标）
 * │        版本 · 板型 · 系统                        │
 * ├────────────────────────────────────────────────┤
 * │ 未批准 → 审批（改名 sheet）                       │ ← EchoMuseDetailTabs.kt
 * │ 已批准 → 六 tab 容器                             │
 * └────────────────────────────────────────────────┘
 * ```
 *
 * ## 删除：红 + 二次确认（K4）
 *
 * 删除是**不可逆**动作（清掉服务端登记，含批准状态与唤醒词设置），所以：
 * - 按钮是 `ButtonTheme.DANGER` + `OUTLINE`（红字红框，不做实心红——页面上不该有一块
 *   随时可误点的大红色）；
 * - 点一次**不会**删除，只把 `state.confirmDeleteId` 置为本设备；
 * - 真正的删除走 `ConfirmDialog(destructive = true)`，且 `dismissOnOutside` 保持默认
 *   `false`：点遮罩不关，必须显式选「确认删除」或「取消」——两步都不可跳过。
 *
 * ## 身份：Avatar + 文案
 *
 * 设备身份用 [Avatar]（`online` 参数自带在线小圆点，替代旧 DSL 手绘的绿点/灰点），
 * 标题行沿用 `row.title`（label 优先，未命名回退设备号），副标题是版本 · 板型 · 系统。
 * 「正常不打标」：在线不渲染徽标，只有离线 / 待批准才占视觉（[statusBadge] 四态语义）。
 */
@Composable
internal fun EchoMuseDeviceCard(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    onOpenModelFiles: () -> Unit = {},
) {
    val pager = LocalAdminPagerOrThrow()
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(bottom = LocalAdminSpacing.current.gutter),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(LocalAdminSpacing.current.cardPadding),
        ) {
            EmDeviceHeader(row = row, onDelete = { state.confirmDeleteId = row.deviceId })
            EchoMuseDetailTabs(
                row = row,
                state = state,
                toast = toast,
                onOpenModelFiles = onOpenModelFiles,
            )
        }
    }

    // 删除二次确认：红按钮 + 点遮罩不关（K4）。enabled=false 在请求在飞时同时置灰两行，
    // 避免"删两次"（服务端第二次会回 404，但那是靠错误码兜底，不是 UI 该有的样子）。
    ConfirmDialog(
        visible = state.confirmDeleteId == row.deviceId,
        title = "删除设备记录？",
        message = "将清除服务端对「${row.title}」的登记（含批准状态与唤醒词设置）。" +
            "这不影响设备本身：它下次接入会重新登记为待批准。",
        confirmText = "确认删除",
        cancelText = "取消",
        destructive = true,
        enabled = !state.actionBusy,
        onConfirm = {
            state.delete(pager, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
                if (ok) state.refresh(pager)
            }
        },
        onCancel = { state.confirmDeleteId = "" },
    )
}

/** 卡片 header：头像（带在线点）+ 标题/副标题 + 状态徽标（仅偏离时）+ 删除按钮。 */
@Composable
private fun EmDeviceHeader(row: EchoMuseDeviceRow, onDelete: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.sm),
    ) {
        Avatar(
            fallback = row.title.take(2),
            online = row.online,
            contentDescription = row.title,
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = row.title,
                style = Theme.typography.titleSmall,
                color = Theme.colors.foreground,
            )
            if (row.subtitle.isNotEmpty()) EmNote(row.subtitle)
            // 偏离才打标：在线是正常态，不打标（一屏绿点等于没有信息）
            if (!row.online || !row.approved) {
                Row(
                    modifier = Modifier.padding(top = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.xs),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (!row.online) statusBadge(state = "error", text = "离线")
                    if (!row.approved) statusBadge(state = "busy", text = "待批准")
                }
            }
        }
        Button(
            text = "删除记录",
            onClick = onDelete,
            theme = ButtonTheme.DANGER,
            type = ButtonType.OUTLINE,
            iconWidget = { GearIcon(token = IconToken.Delete, size = 16.dp, tint = Theme.colors.destructive) },
            contentDescription = "删除该设备记录",
        )
    }
}
