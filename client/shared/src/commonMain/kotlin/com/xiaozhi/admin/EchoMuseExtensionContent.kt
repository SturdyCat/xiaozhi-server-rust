package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.dialog.ConfirmDialog
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow

/** Separate custom workspace. Official WiFi confirmation and asset installation are not repeated. */
@Composable
internal fun EchoMuseExtensionContent(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState, tab: String) {
    val pager = LocalAdminPagerOrThrow()
    LaunchedEffect(row.deviceId, row.approved, tab) {
        if (row.approved && tab == "ble") state.ble.select(row)
    }
    Column(modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.sm)) {
        when (tab) {
            "music" -> EchoMuseMusicStage(row, state, toast)
            "ble" -> EchoMuseBlePanel(row, state, showSettings = false)
            "diagnostics" -> EchoMuseRawDiagnostics(state, toast)
        }
        if (state.detail.error.isNotEmpty()) EmWarn(state.detail.error, error = true)
        if (state.detail.result.isNotEmpty()) EmNote(state.detail.result)
    }
}

/** Raw reports and debug actions live here, never inside official Status / Activity. */
@Composable
private fun EchoMuseRawDiagnostics(state: EchoMuseState, toast: ToastState) {
    val pager = LocalAdminPagerOrThrow()
    val detail = state.detail
    actionRow {
        Button(text = "读取设备报告", type = ButtonType.OUTLINE, disabled = !detail.idle,
            onClick = { detail.readRuntime(pager) })
        Button(text = "读取原始系统信息", type = ButtonType.OUTLINE,
            loading = detail.busy == "system/info", disabled = !detail.canSystem || !detail.idle,
            onClick = { detail.readInfo(pager) })
        Button(text = "复制诊断内容", type = ButtonType.OUTLINE,
            disabled = !detail.infoLoaded && !detail.runtimeLoaded,
            onClick = {
                val text = detail.diagnosticText + "\n\n原始设备报告：\n" + detail.runtime?.toString().orEmpty()
                toast.show(if (copyToClipboard(text)) "诊断内容已复制" else "复制失败：剪贴板不可用")
            })
    }
    if (detail.runtimeLoaded) {
        EmGroupLabel(if (detail.runtimeCurrent) "当前会话最近报告" else "历史设备报告")
        EmOutputBlock(detail.runtime?.toString().orEmpty(), height = 240.dp)
    }
    if (detail.infoLoaded) {
        EmGroupLabel(if (detail.infoHistorical) "历史系统信息" else "原始系统信息")
        EmOutputBlock(detail.infoOutput, height = 240.dp)
    }
    Button(text = "重启设备", type = ButtonType.OUTLINE, disabled = !detail.canSystem || !detail.idle,
        onClick = { detail.confirmReboot = true })
    ConfirmDialog(visible = detail.confirmReboot, title = "确认重启设备？",
        message = "当前连接与语音会中断。请求发送不代表重启已完成。", destructive = true,
        confirmText = "发送重启请求", cancelText = "取消", dismissOnOutside = false,
        enabled = detail.canSystem && detail.idle, onConfirm = { detail.reboot(pager) },
        onCancel = { detail.confirmReboot = false })
    EmNote("自定义诊断包含原始设备与网络信息，请仅向可信人员分享。")
}
