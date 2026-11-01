package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.ScrollView
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.*

/** Official DeviceConfigForm's seven stages, using only the Rust settings contract. */
@Composable
internal fun EchoMuseNativeConfigTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    modifier: Modifier = Modifier,
) {
    val pager = LocalAdminPagerOrThrow()
    val d = state.drafts
    var error by remember(row.deviceId) { mutableStateOf("") }
    var result by remember(row.deviceId) { mutableStateOf("") }
    var reloadRequested by remember(row.deviceId) { mutableStateOf(false) }
    LaunchedEffect(reloadRequested, state.loading, state.loadError) {
        if (reloadRequested && !state.loading) {
            reloadRequested = false
            if (state.loadError.isNotEmpty()) error = state.loadError
            else {
                val latest = state.devices.firstOrNull { it.deviceId == row.deviceId }
                if (latest != null) {
                    state.fillConfigDrafts(latest)
                    error = ""
                    result = "已从服务器重新加载，未保存输入已丢弃"
                } else error = "设备不存在，请关闭后刷新列表"
            }
        }
    }
    // enterTab owns initial fill. List refreshes never overwrite this device's dirty draft.
    Column(modifier = modifier.fillMaxSize()) {
        ScrollView(modifier = Modifier.weight(1f).fillMaxWidth().padding(16.dp)) {
        if (!row.approved) {
            EmWarn("请先批准设备，再修改配置。")
        } else {
            EmNote("只保存修改过的设备设置；空白数值不提交。保存或排队不代表设备已经应用。")
            if (!row.online) EmNote("设备离线：仍可保存，设备下次接入时推送。")
            EmNativePlayback(d, !state.actionBusy)
            EmNativeWake(row, d, !state.actionBusy)
            EmNativeMicrophones(d, !state.actionBusy)
            EmNativeConfigStage("04", "灯环", "音量提示弧与设备本地私听动画。") {
                switchRow("远程调音量时显示灯环", d.remoteVolumeArc, { d.remoteVolumeArc = it }, enabled = !state.actionBusy)
                labeledTextArea("私听动画", d.listeningAnimDraft, { d.listeningAnimDraft = it },
                    help = "设备原生动画 JSON 对象或数组；留空不修改。仅支持设备本地动画，不提供控制器场景、颜色和电平动画参数。",
                    placeholder = "留空不修改", enabled = !state.actionBusy)
                EmNote("官方控制器灯环场景、颜色与电平动画尚无服务端接口，不提供无效开关。")
            }
            EmNativeAdvanced(d, !state.actionBusy)
            EmNativeConfigStage("06", "蓝牙", "设备 BLE 广播扫描与连接开关；通用读写位于「GATT 工具」。") {
                EmWarn("启用代理可能永久切换设备蓝牙芯片工作模式，影响原系统蓝牙音箱配对。")
                switchRow("蓝牙广播扫描", d.bleProxyEnabled, { d.bleProxyEnabled = it }, enabled = !state.actionBusy)
                switchRow("允许蓝牙连接", d.bleProxyConnections, { d.bleProxyConnections = it },
                    enabled = !state.actionBusy && d.bleProxyEnabled,
                    description = "还需服务器开启蓝牙代理、设备声明连接能力；保存不代表连接可用。")
            }
            EmNativeConfigStage("07", "Sendspin", "Music Assistant 直接连接设备；本服务器仅保存并推送设置，不代理播放或配对。") {
                switchRow("Sendspin 播放器", d.sendspinEnabled, { d.sendspinEnabled = it }, enabled = !state.actionBusy)
                switchRow("允许未配对播放", d.sendspinUnpaired, { d.sendspinUnpaired = it },
                    enabled = !state.actionBusy && d.sendspinEnabled,
                    description = "允许 Music Assistant 认可的服务器，安全性较低。")
                labeledField("播放器名称", d.sendspinName, { d.sendspinName = it }, enabled = !state.actionBusy)
                EmNote("配对令牌、分组状态等官方控制器功能暂未提供，不声明已配对或已播放。")
            }
            EchoMuseNativeWifi(row, state, toast)
        }
        }
        if (row.approved) Column(modifier = Modifier.fillMaxWidth().padding(16.dp)) {
            if (error.isNotEmpty()) EmWarn(error, error = true)
            if (result.isNotEmpty()) EmNote(result)
            actionRow {
                Button(text = "重新加载", type = ButtonType.OUTLINE, disabled = state.actionBusy,
                    onClick = {
                        reloadRequested = true
                        state.refresh(pager)
                    })
                Button(text = "保存配置", loading = state.actionBusy, disabled = state.actionBusy,
                    onClick = {
                        error = ""
                        result = ""
                        val delta = d.buildConfigBody { error = it }
                        if (delta != null) {
                            if (!delta.keys().hasNext()) result = "没有需要保存的修改"
                            else state.saveConfig(pager, row.deviceId) { ok, msg ->
                                if (ok) {
                                    result = msg
                                    state.refresh(pager)
                                } else error = msg // Including 409: preserve every draft until explicit reload.
                                toast.show(msg, if (ok) "ok" else "error")
                            }
                        }
                    })
            }
        }
    }
}

@Composable
internal fun EmNativeConfigStage(number: String, title: String, description: String, content: @Composable ColumnScope.() -> Unit) {
    Card(modifier = Modifier.fillMaxWidth().padding(top = 12.dp)) {
        Text("$number  $title", style = Theme.typography.titleSmall, color = Theme.colors.foreground)
        EmNote(description)
        content()
    }
}

/** Actual modal width, not global viewport, decides whether a stage has two columns. */
@Composable
internal fun EmNativeConfigColumns(left: @Composable () -> Unit, right: @Composable () -> Unit) {
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        if (maxWidth.value >= GearUiBreakpoints.WORKSPACE_COMPACT) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(24.dp)) {
                Column(Modifier.weight(1.4f)) { left() }
                Column(Modifier.weight(1f)) { right() }
            }
        } else Column(Modifier.fillMaxWidth()) { left(); right() }
    }
}

@Composable
internal fun EmNativeConfigAdvanced(title: String = "高级设置", content: @Composable () -> Unit) {
    var open by remember { mutableStateOf(false) }
    Button(text = if (open) "收起$title" else "展开$title", type = ButtonType.TEXT,
        onClick = { open = !open })
    if (open) content()
}
