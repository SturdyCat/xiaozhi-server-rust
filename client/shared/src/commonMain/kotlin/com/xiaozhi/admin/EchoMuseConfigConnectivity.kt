package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.alert.Alert
import com.gearui.components.alert.AlertStatus
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledField
import com.xiaozhi.admin.kit.splitRow

/**
 * 配置 tab 的**网络连接**三块（对齐官方 ConnectivityTab，原「WiFi 配置」面板六块内容的归位处）：
 *
 * ```
 * ┌──────────────────────┬──────────────────────┐
 * │ 当前连接              │ 可见网络              │ ← splitRow 两列等宽
 * │ 网络/信号/频率/速率    │ 扫描清单（当前那条标 ← 当前）│
 * └──────────────────────┴──────────────────────┘
 * 切换网络：SSID / PSK + 下发 + 扫描 + 读取扫描结果
 * 待确认横幅（已排队但设备还没回报 wifi_result）
 * 设备回报成功 → 「确认新网络（wifi_commit）」= 二次确认，不确认则回滚窗口后自动回旧网
 * ```
 *
 * ## 202 的口径在这里出现两次（横幅与底部说明）
 *
 * **只代表指令已排队**：切网结果一律以设备上报的 `wifi_result` 为准，页面不假装知道结果。
 * 重复两次是刻意的——一次在"等待"时（用户刚点完下发），一次在"提交"前（用户决定固化），
 * 两个时机都是最容易误读的时刻。
 *
 * 「当前连接」的四行全部来自 `runtime.stats`（设备上报的标量白名单），**不猜**：
 * 没上报就是 `—`，不显示 0 dBm 或 0 Mbps。
 */
@Composable
internal fun EchoMuseConnectivity(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
) {
    val pager = LocalAdminPagerOrThrow()
    EmGroupLabel("网络连接（官方 connectivity）", topGap = GearUiSpace.sm)
    splitRow(spacing = GearUiSpace.md) {
        grow {
            EmNote("当前连接")
            EmInfoRow("网络", state.wifiCurrentSsid.ifEmpty { EM_DASH })
            EmInfoRow("信号", emRssi(state.wifiStat("wifiRssi")))
            EmInfoRow("频率", emMhz(state.wifiStat("wifiFreqMhz")))
            EmInfoRow("速率", emMbps(state.wifiStat("linkSpeedMbps")))
        }
        grow {
            EmNote("可见网络（设备扫描上报，非实时）")
            if (state.wifiScanRows.isEmpty()) {
                EmNote(
                    if (state.wifiRuntimeBusy) "正在读取扫描结果…"
                    else "尚未读取：点下方「扫描周边网络」后在设备上排队执行，再点「读取扫描结果」",
                )
            } else {
                EmNote(state.wifiScanRows.joinToString("\n"))
            }
        }
    }

    EmGroupLabel("切换网络")
    labeledField("WiFi 名称（SSID）", state.wifiSsid, { state.wifiSsid = it }, placeholder = "路由器名称")
    labeledField("WiFi 密码（PSK）", state.wifiPsk, { state.wifiPsk = it }, placeholder = "密码")
    actionRow {
        Button(
            text = if (state.actionBusy) "下发中…" else "下发 WiFi 配置",
            onClick = {
                state.sendWifi(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
            },
            type = ButtonType.FILL,
            loading = state.actionBusy,
            disabled = !(row.online && state.wifiSsid.isNotBlank() && !state.actionBusy),
            iconWidget = { GearIcon(token = IconToken.Wifi, size = 16.dp) },
        )
        Button(
            text = "扫描周边网络",
            onClick = {
                state.sendWifiScan(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
            },
            type = ButtonType.OUTLINE,
            loading = state.actionBusy || state.wifiRuntimeBusy,
            disabled = !row.online,
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
        Button(
            text = "读取扫描结果",
            onClick = { state.readWifiRuntime(pager, row.deviceId) },
            type = ButtonType.OUTLINE,
            loading = state.wifiRuntimeBusy,
            disabled = !row.online || state.wifiRuntimeBusy,
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
    }
    // 待确认横幅：已下发但设备还没回报「新网可用」——此时既不能说成功也不能说失败
    if (state.wifiPending) {
        Alert(
            title = "切换指令已排队，尚未确认成功",
            description = "设备重连新网成功后会在这里出现「确认新网络」；" +
                "回滚窗口结束仍未确认，设备会自己回到原来的网络（切错网不失联的保险）。",
            status = AlertStatus.WARNING,
            modifier = Modifier.fillMaxWidth().padding(top = GearUiSpace.sm),
        )
    }
    // 二次确认：只有设备回报 wifi_result.ok == true 才出现
    if ((state.wifiRuntime?.optJSONObject("wifi_result")?.opt("ok") as? Boolean) == true) {
        state.wifiResultText?.let { EmNote(it) }
        actionRow {
            Button(
                text = "确认新网络（wifi_commit）",
                onClick = {
                    state.sendWifiCommit(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
                },
                type = ButtonType.FILL,
                loading = state.actionBusy,
                disabled = !row.online,
                iconWidget = { GearIcon(token = IconToken.Check, size = 16.dp) },
            )
        }
        EmNote("确认后设备固定使用新网；不确认则回滚窗口结束后自动回到旧网。")
    }
    if (state.wifiRuntimeError.isNotEmpty()) {
        EmWarn(state.wifiRuntimeError, error = true)
    }
    if (!row.online) {
        EmNote("设备离线：WiFi 指令需要设备在线才能接收，扫描/切网都做不了（保存不了，只能等它上线）。")
    }
}

/**
 * 「下发配置」——配置 tab 的**唯一**写动作（Stage 01–07 的所有控件都只是改草稿）。
 *
 * 文案必须写「仅代表已排队」：202 = 设备在线已排队下发，200 = 只保存到服务端、设备下次
 * 接入自动推送——两种都不等于"已生效"，实际生效只能在设备上核对。
 */
@Composable
internal fun EchoMuseConfigSubmit(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
) {
    val pager = LocalAdminPagerOrThrow()
    EmGroupLabel("下发配置")
    actionRow {
        Button(
            text = if (state.actionBusy) "下发中…" else "下发配置",
            onClick = {
                state.saveConfig(pager, row.deviceId) { ok, msg -> toast.show(msg, if (ok) "ok" else "error") }
            },
            type = ButtonType.FILL,
            loading = state.actionBusy,
            iconWidget = { GearIcon(token = IconToken.Save, size = 16.dp) },
        )
        Button(
            text = "按设备现值重新填充",
            onClick = { state.fillConfigDrafts(row) },
            type = ButtonType.OUTLINE,
            disabled = state.actionBusy,
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
    }
    EmNote(
        "「下发配置」仅代表已排队：设备在线时接口回 202（已排队下发），离线时只保存到服务端、" +
            "设备下次接入自动推送。两种都不等于已生效——请按听感/灯环在设备上核对。" +
            "上面留空的数值键不会下发（保留设备当前值）。",
    )
}

/** `wifi_runtime.stats[key]` 的数值（未上报 = null，绝不填 0 或估算）。 */
private fun EchoMuseState.wifiStat(key: String): Double? =
    emNumber(wifiRuntime?.optJSONObject("stats"), key)
