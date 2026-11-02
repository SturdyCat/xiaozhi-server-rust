package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.input.Input
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.LocalAdminSpacing

/** The upstream approval sheet, rendered locally; device approval never needs a WebView. */
@Composable
internal fun EchoMuseApprovalTab(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState,
    modifier: Modifier = Modifier) {
    val pager = LocalAdminPagerOrThrow()
    val space = LocalAdminSpacing.current
    var label by remember(row.deviceId) { mutableStateOf(row.label) }
    var error by remember(row.deviceId) { mutableStateOf("") }
    ScrollView(modifier = modifier) {
        Column(modifier = Modifier.fillMaxWidth().padding(top = space.md),
            verticalArrangement = Arrangement.spacedBy(space.md)) {
            Alert(title = "新设备 · 等待批准", description = "请为设备命名并批准加入。批准前不会开放配置、控制台或其他设备操作。", status = AlertStatus.WARNING)
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.fillMaxWidth().padding(space.md), verticalArrangement = Arrangement.spacedBy(space.sm)) {
                    Text("设备登记", style = Theme.typography.titleMedium, color = Theme.colors.foreground)
                    Text("设备号：${row.deviceId}", color = Theme.colors.mutedForeground)
                    Text("IP 地址：${echoMuseDeviceAddress(row)}", color = Theme.colors.mutedForeground)
                    Text("连接状态：${if (row.online) "在线" else "离线"}", color = Theme.colors.mutedForeground)
                    Input(value = label, onValueChange = { label = it; error = "" }, label = "设备名称",
                        placeholder = "例如：厨房音箱", maxLength = 64, enabled = !state.actionBusy,
                        modifier = Modifier.fillMaxWidth())
                    if (error.isNotEmpty()) Alert(title = error, status = AlertStatus.DANGER)
                    Row(horizontalArrangement = Arrangement.spacedBy(space.sm)) {
                        Button(text = "批准并加入设备", disabled = state.actionBusy || label.trim().isEmpty(),
                            loading = state.actionBusy, type = ButtonType.FILL,
                            onClick = {
                                state.approve(pager, row.deviceId, label) { ok, message ->
                                    if (ok) { toast.show(message, "ok"); state.refresh(pager) }
                                    else error = message
                                }
                            })
                    }
                    Text("只批准你认识并信任的设备；名称以后可在标题行中修改。", style = Theme.typography.caption,
                        color = Theme.colors.mutedForeground)
                }
            }
        }
    }
}
