package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.*

/** Actual wifi_scan → wifi_set → current-session wifi_commit workflow, no controller-only endpoint. */
@Composable
internal fun EchoMuseNativeWifi(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState) {
    val pager = LocalAdminPagerOrThrow()
    val detail = state.detail
    val current = detail.target == row.deviceId && detail.runtimeCurrent
    val runtime = detail.runtime.takeIf { detail.target == row.deviceId }
    val wifi = runtime?.optJSONObject("wifi_result")
    var ssid by remember(row.deviceId) { mutableStateOf("") }
    var psk by remember(row.deviceId) { mutableStateOf("") }
    val canSend = row.online && row.approved && !state.actionBusy
    fun command(action: String, payload: com.tencent.kuikly.core.nvi.serialization.json.JSONObject = com.tencent.kuikly.core.nvi.serialization.json.JSONObject()) {
        payload.put("action", action)
        state.command(pager, row.deviceId, payload) { ok, message ->
            if (ok && action == "wifi") psk = ""
            toast.show(if (ok) "指令已排队；请稍后读取设备上报核对结果" else message, if (ok) "ok" else "error")
        }
    }
    EmNativeConfigStage("08", "网络连接", "扫描、切网与确认分开操作；未确认的新网络可能自动回滚。") {
        EmInfoRow("当前网络", runtime?.optJSONObject("stats")?.text("wifiSsid")?.ifEmpty { "未知" } ?: "未知")
        if (!current && runtime != null) EmWarn("以下是历史网络快照，不可用于确认新网络。")
        actionRow {
            Button(text = "扫描周边网络", type = ButtonType.OUTLINE, disabled = !canSend,
                onClick = { command("wifi_scan") })
            Button(text = "读取网络结果", type = ButtonType.OUTLINE, loading = detail.busy == "runtime",
                disabled = !detail.idle, onClick = { detail.readRuntime(pager) })
        }
        val networks = runtime?.optJSONObject("wifi_scan")?.array("networks")
        if (networks != null) for (i in 0 until minOf(networks.length(), 30)) {
            val network = networks.optJSONObject(i) ?: continue
            val name = network.text("ssid")
            Button(text = name.ifEmpty { "隐藏网络" }, type = ButtonType.TEXT, disabled = name.isEmpty(),
                onClick = { ssid = name })
        }
        labeledField("网络名称（SSID）", ssid, { ssid = it }, enabled = canSend)
        labeledField("网络密码", psk, { psk = it }, keyboardType = com.tencent.kuikly.compose.ui.text.input.KeyboardType.Password, enabled = canSend)
        Button(text = "切换到新网络", disabled = !canSend || ssid.trim().isEmpty(), onClick = {
            command("wifi", com.tencent.kuikly.core.nvi.serialization.json.JSONObject().apply {
                put("ssid", ssid.trim()); put("psk", psk)
            })
        })
        if (wifi != null && wifi.length() > 0) {
            EmInfoRow("最近切网结果", when (wifi.opt("ok")) {
                true -> "连接成功：${wifi.text("ssid")}（仍需确认）"
                false -> "连接失败：${wifi.text("error")}"; else -> "未知"
            })
        }
        Button(text = "确认保留新网络", type = ButtonType.OUTLINE,
            disabled = !canSend || !current || wifi?.opt("ok") != true,
            onClick = { state.sendWifiCommit(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") } })
        if (detail.error.isNotEmpty()) EmWarn(detail.error, error = true)
        EmNote("指令排队不是切网成功；请在当前会话读取新网络成功结果后再确认。")
    }
}
