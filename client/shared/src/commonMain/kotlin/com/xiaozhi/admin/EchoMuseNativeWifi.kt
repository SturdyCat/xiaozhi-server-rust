package com.xiaozhi.admin

import androidx.compose.runtime.*
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.kit.*

/** Official connectivity layout: current connection / visible networks, then change network. */
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
    LaunchedEffect(row.deviceId, row.sessionId) { if (row.approved) detail.readRuntime(pager) }
    fun command(action: String, payload: JSONObject = JSONObject()) {
        payload.put("action", action)
        state.command(pager, row.deviceId, payload) { ok, message ->
            if (ok && action == "wifi") psk = ""
            toast.show(if (ok) "指令已排队；请读取设备结果核对" else message, if (ok) "ok" else "error")
        }
    }
    Column(Modifier.fillMaxWidth()) {
        EmNativeConfigColumns(left = {
            EchoMuseNativePanel("当前连接") {
                EmInfoRow("网络", runtime?.optJSONObject("stats")?.text("wifiSsid")?.ifEmpty { "未知" } ?: "未知")
                EmInfoRow("IP 地址", echoMuseDeviceAddress(row))
                EmInfoRow("信号", emRssi(emNumber(runtime?.optJSONObject("stats"), "wifiRssi")))
                if (!current && runtime != null) EmNote("历史网络快照")
            }
        }, right = {
            EchoMuseNativePanel("可用网络") {
                actionRow {
                    Button(text = "扫描周边网络", type = ButtonType.OUTLINE, disabled = !canSend,
                        onClick = { command("wifi_scan") })
                    EchoMuseActionButton(IconToken.Refresh, "读取网络结果", loading = detail.busy == "runtime",
                        disabled = !detail.idle, onClick = { detail.readRuntime(pager) })
                }
                val networks = runtime?.optJSONObject("wifi_scan")?.array("networks")
                if (networks != null) for (i in 0 until minOf(networks.length(), 30)) {
                    val network = networks.optJSONObject(i) ?: continue
                    val name = network.text("ssid")
                    Button(text = name.ifEmpty { "隐藏网络" }, type = ButtonType.TEXT, disabled = name.isEmpty(),
                        onClick = { ssid = name })
                }
            }
        })
        EchoMuseNativePanel("更改网络") {
            EmNativeConfigColumns(left = {
                labeledField("网络名称（SSID）", ssid, { ssid = it }, enabled = canSend)
            }, right = {
                labeledSecretField("网络密码", psk, { psk = it }, enabled = canSend)
            })
            actionRow {
                Button(text = "切换到新网络", disabled = !canSend || ssid.trim().isEmpty(), onClick = {
                    command("wifi", JSONObject().apply { put("ssid", ssid.trim()); put("psk", psk) })
                })
                if (current && wifi?.opt("ok") == true) Button(text = "确认保留新网络", type = ButtonType.OUTLINE,
                    disabled = !canSend, onClick = {
                        state.sendWifiCommit(pager, row.deviceId) { ok, message -> toast.show(message, if (ok) "ok" else "error") }
                    })
            }
            if (wifi != null && wifi.length() > 0) EmInfoRow("最近结果", when (wifi.opt("ok")) {
                true -> "已连接：${wifi.text("ssid")}（等待确认）"
                false -> "失败：${wifi.text("error")}"
                else -> "未知"
            })
            EmNote("无法连接的新网络将自动回滚；指令排队不代表切换成功。")
            if (detail.error.isNotEmpty()) EmWarn(detail.error, error = true)
        }
    }
}
