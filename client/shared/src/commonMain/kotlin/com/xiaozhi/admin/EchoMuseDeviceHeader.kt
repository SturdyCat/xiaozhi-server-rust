package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.dialog.ConfirmDialog
import com.gearui.components.input.Input
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.LocalAdminViewport

/** 12 个 Tab 共享的唯一设备头部；不再让每份官方文档重复渲染头部和管理动作。 */
@Composable
internal fun EchoMuseDeviceHeader(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    onDismiss: () -> Unit,
    onInputBlocked: (Boolean) -> Unit = {},
) {
    val pager = LocalAdminPagerOrThrow()
    val spacing = LocalAdminSpacing.current
    val compact = LocalAdminViewport.current.workspaceCompact
    var renaming by remember(row.deviceId) { mutableStateOf(false) }
    var name by remember(row.deviceId, row.label) { mutableStateOf(row.label) }
    var deleting by remember(row.deviceId) { mutableStateOf(false) }
    // GearUI's LocalInputBlockedByOverlay only covers page content, not an earlier modal.
    SideEffect { onInputBlocked(deleting || renaming) }
    val saveName = {
        if (name.trim().isEmpty()) {
            toast.show("设备名称不能为空", "error")
        } else {
            state.approve(pager, row.deviceId, label = name) { ok, message ->
                toast.show(if (ok) "设备名称已保存" else message, if (ok) "ok" else "error")
                if (ok) {
                    renaming = false
                    state.refresh(pager)
                }
            }
        }
    }
    Column(modifier = Modifier.fillMaxWidth().padding(bottom = spacing.sm),
        verticalArrangement = Arrangement.spacedBy(spacing.xs)) {
        Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
            EchoMuseDevicePicture(row, if (compact) 48.dp else 72.dp)
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(spacing.xs)) {
                if (renaming) {
                    Input(value = name, onValueChange = { name = it }, label = "设备名称",
                        maxLength = 64, enabled = !state.actionBusy, modifier = Modifier.fillMaxWidth())
                } else {
                    Text(text = row.title, style = Theme.typography.titleMedium,
                        color = Theme.colors.foreground, maxLines = 1)
                }
                Text(text = listOf(echoMuseDeviceAddress(row), row.deviceId, row.subtitle)
                    .filter { it.isNotEmpty() }.joinToString(" · "),
                    style = Theme.typography.caption, color = Theme.colors.mutedForeground, maxLines = 2)
            }
            if (!compact) {
                EchoMuseConnectionText(row)
                EchoMuseHeaderActions(row.approved, renaming, state.actionBusy, saveName,
                    { renaming = false; name = row.label },
                    { name = row.label; renaming = true }, { deleting = true }, onDismiss)
            }
        }
        if (compact) {
            Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(modifier = Modifier.weight(1f)) { EchoMuseConnectionText(row) }
                EchoMuseHeaderActions(row.approved, renaming, state.actionBusy, saveName,
                    { renaming = false; name = row.label },
                    { name = row.label; renaming = true }, { deleting = true }, onDismiss)
            }
        }
    }
    ConfirmDialog(visible = deleting, title = "删除设备记录？",
        message = "将删除「${row.title}」的登记记录，不会卸载设备系统或断开在线会话。设备再次登记后会重新出现。",
        confirmText = "删除", destructive = true, dismissOnOutside = false, enabled = !state.actionBusy,
        onConfirm = {
            state.delete(pager, row.deviceId) { ok, message ->
                toast.show(message, if (ok) "ok" else "error")
                if (ok) {
                    deleting = false
                    state.refresh(pager)
                    onDismiss()
                }
            }
        },
        onCancel = { if (!state.actionBusy) deleting = false })
}

@Composable
private fun EchoMuseHeaderActions(
    approved: Boolean, renaming: Boolean, busy: Boolean, save: () -> Unit, cancel: () -> Unit,
    rename: () -> Unit, delete: () -> Unit, close: () -> Unit,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.xs),
        verticalAlignment = Alignment.CenterVertically) {
        if (renaming) {
            EchoMuseActionButton(IconToken.Check, "保存设备名称", save, disabled = busy, loading = busy)
            EchoMuseActionButton(IconToken.Close, "取消改名", cancel, disabled = busy)
        } else {
            if (approved) EchoMuseActionButton(IconToken.Edit, "改名", rename, disabled = busy)
            EchoMuseActionButton(IconToken.Delete, "删除设备", delete, disabled = busy, destructive = true)
        }
        EchoMuseActionButton(IconToken.Close, "关闭设备详情", close)
    }
}
