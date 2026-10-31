package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow

/** 原生扩展只画官方页面没有接通的内容；模型文件直接复用 FilesPage，不在这里再套 Tabs。 */
@Composable
internal fun EchoMuseExtensionContent(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    tab: String,
) {
    val pager = LocalAdminPagerOrThrow()
    LaunchedEffect(row.deviceId, row.approved, tab) {
        if (row.approved) {
            state.detail.enterTab(if (tab == "assets") EM_TAB_UPDATES else tab, row, pager)
            if (tab == "ble") state.ble.select(row)
        }
    }
    DisposableEffect(row.deviceId) { onDispose { state.detail.leave() } }
    Column(modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.sm)) {
        when (tab) {
            "music" -> EchoMuseMusicStage(row, state, toast)
            "ble" -> EchoMuseBlePanel(row, state, showSettings = false)
            "assets" -> EmAssets(state, toast)
            "wifi" -> EchoMuseWifiConfirmation(row, state, toast)
            "diagnostics" -> EchoMuseRawDiagnostics(state, toast)
        }
        if (state.detail.error.isNotEmpty()) EmWarn(state.detail.error, error = true)
        if (state.detail.result.isNotEmpty()) EmNote(state.detail.result)
    }
}

@Composable
private fun EchoMuseWifiConfirmation(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState) {
    val pager = LocalAdminPagerOrThrow()
    val detail = state.detail
    EmNote("扫描与切网在「配置 → 网络连接」操作。这里只补充新网络确认；超时未确认设备会回滚旧网。")
    Button(text = "读取切网结果", onClick = { detail.readRuntime(pager) },
        type = ButtonType.OUTLINE, disabled = !row.online || !detail.idle)
    val result = detail.runtime?.optJSONObject("wifi_result")
    if (detail.runtimeCurrent && result?.opt("ok") == true) {
        EmInfoRow("新网络", result.text("ssid").ifEmpty { EM_DASH })
        Button(text = "确认保留新网络", type = ButtonType.OUTLINE,
            disabled = !row.online || state.actionBusy,
            onClick = {
                state.sendWifiCommit(pager, row.deviceId) { ok, message ->
                    toast.show(message, if (ok) "ok" else "error")
                }
            })
        EmNote("响应只代表确认指令已排队，实际网络以设备上报为准。")
    } else EmNote("尚未读取到当前会话的新网络成功结果，不能确认。")
}

/** 官方状态页是结构化投影；这里保留设备 system/info 原文与诊断复制，不重复状态磁贴/重启/日志。 */
@Composable
private fun EchoMuseRawDiagnostics(state: EchoMuseState, toast: ToastState) {
    val pager = LocalAdminPagerOrThrow()
    val detail = state.detail
    actionRow {
        Button(text = "读取原始系统信息", onClick = { detail.readInfo(pager) }, type = ButtonType.OUTLINE,
            loading = detail.busy == "system/info", disabled = !detail.canSystem || !detail.idle)
        Button(text = "复制诊断内容", type = ButtonType.OUTLINE, disabled = !detail.infoLoaded,
            onClick = {
                if (copyToClipboard(detail.diagnosticText)) toast.show("诊断内容已复制", "ok")
                else toast.show("复制失败：当前平台剪贴板不可用", "error")
            })
    }
    if (detail.infoHistorical) EmWarn("以下内容来自历史会话，请重新读取当前设备。")
    if (detail.infoLoaded) {
        EmGroupLabel("设备原始输出（只读）")
        EmNote(detail.infoOutput)
    } else EmNote("尚未读取设备系统信息；此操作只读，不提供任意 shell 或固件刷写。")
    EmNote("诊断内容可能包含设备与网络信息，仅向可信人员分享。")
}
