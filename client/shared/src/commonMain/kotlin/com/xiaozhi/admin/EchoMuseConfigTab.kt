package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager

/**
 * 配置 tab：**网络连接（官方 connectivity，三块）** + Stage 01–08 + 底部「下发配置」。
 *
 * 三块网络连接（对齐官方 1477-1556 的 ConnectivityTab，也是原「WiFi 配置」面板的六块内容
 * 逐块归位处）：
 *
 * ```
 * ┌──────────────────────┬──────────────────────┐
 * │ 当前连接              │ 可见网络              │ ← splitRow 两列都 flex(1f)（§6.6：
 * │ 网络 / 信号 / 频率 / 速率 │ 扫描清单（当前那条标 ← 当前）│   定宽列才会踩 dividerVAt 锚点）
 * └──────────────────────┴──────────────────────┘
 * 切换网络：SSID / PSK + 下发 + 扫描 + 读取扫描结果
 * 待确认横幅（已排队但设备还没回报 wifi_result）
 * 设备回报成功 → 「确认新网络（wifi_commit）」= 二次确认，不确认则回滚窗口后自动回旧网
 * ```
 *
 * 202 的口径在这里出现两次（横幅与底部说明）：**只代表指令已排队**，切网结果一律以设备
 * 上报的 `wifi_result` 为准，页面不假装知道结果。
 *
 * Stage 01–08 的内容全在 [EchoMuseConfigStages]（按设计 §6.3 分派，只画白名单内的键）。
 */

/** `wifi_runtime.stats[key]` 的数值（未上报 = null，绝不填 0 或估算）。 */
private fun EchoMuseState.wifiStat(key: String): Double? =
    (wifiRuntime?.optJSONObject("stats")?.opt(key) as? Number)?.toDouble()

internal fun ViewContainer<*, *>.echoMuseConfigTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
    onOpenModelFiles: () -> Unit = {},
) {
    echoMuseConnectivity(row, state, toast, ctx)

    stagePlayback(state)
    stageWakeWord(state, ctx, onOpenModelFiles)
    stageMicrophones(state)
    stageRing(state)
    stageAdvanced(state)
    stageBluetooth(row, state, ctx)
    stageSendspin(state)
    stageMusic(row, state, toast, ctx)

    echoMuseConfigSubmit(row, state, toast, ctx)
}

// ============================================================
// 网络连接（官方 connectivity）
// ============================================================

/**
 * 网络连接三块：当前连接 / 可见网络 / 切换网络。
 *
 * 「当前连接」的四行全部来自 `runtime.stats`（设备上报的标量白名单），**不猜**：
 * 没上报就是 `—`，不显示 0 dBm 或 0 Mbps。
 */
private fun ViewContainer<*, *>.echoMuseConnectivity(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    groupLabel("网络连接（官方 connectivity）")
    splitRow(spacing = AdminSpace.md) {
        grow {
            fieldLabel("当前连接")
            infoRow({ "网络" }, { state.wifiCurrentSsid.ifEmpty { EM_DASH } })
            infoRow({ "信号" }, { emRssi(state.wifiStat("wifiRssi")) })
            infoRow({ "频率" }, { emMhz(state.wifiStat("wifiFreqMhz")) })
            infoRow({ "速率" }, { emMbps(state.wifiStat("linkSpeedMbps")) })
        }
        grow {
            fieldLabel("可见网络（设备扫描上报，非实时）")
            vif({ state.wifiScanRows.isEmpty() }) {
                detailLine(
                    {
                        if (state.wifiRuntimeBusy) "正在读取扫描结果…"
                        else "尚未读取：点下方「扫描周边网络」后在设备上排队执行，再点「读取扫描结果」"
                    },
                    topGap = 0f,
                )
            }
            vif({ state.wifiScanRows.isNotEmpty() }) {
                detailLine({ state.wifiScanRows.joinToString("\n") }, topGap = 0f)
            }
        }
    }

    groupLabel("切换网络")
    labeledField("WiFi 名称（SSID）", { state.wifiSsid }, { state.wifiSsid = it }, "路由器名称")
    labeledField("WiFi 密码（PSK）", { state.wifiPsk }, { state.wifiPsk = it }, "密码")
    actionRow {
        primaryButton(
            "下发 WiFi 配置",
            enabled = { row.online && state.wifiSsid.isNotBlank() && !state.actionBusy },
            loading = { state.actionBusy },
            loadingText = "下发中…",
            icon = IconToken.Wifi,
        ) {
            state.sendWifi(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
        hGap(AdminSpace.sm)
        secondaryButton(
            "扫描周边网络",
            enabled = { row.online },
            loading = { state.actionBusy || state.wifiRuntimeBusy },
            icon = IconToken.Refresh,
        ) {
            state.sendWifiScan(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
        hGap(AdminSpace.sm)
        secondaryButton(
            "读取扫描结果",
            enabled = { row.online && !state.wifiRuntimeBusy },
            loading = { state.wifiRuntimeBusy },
            icon = IconToken.Refresh,
        ) {
            state.readWifiRuntime(ctx, row.deviceId)
        }
    }
    // 待确认横幅：已下发但设备还没回报「新网可用」——此时既不能说成功也不能说失败
    vif({ state.wifiPending }) {
        textNote(
            "切换指令已排队下发，尚未确认成功：设备重连新网成功后会在这里出现「确认新网络」；" +
                "回滚窗口结束仍未确认，设备会自己回到原来的网络（切错网不失联的保险）。",
            tone = "warn",
            micro = true,
        )
    }
    // 二次确认：只有设备回报 wifi_result.ok == true 才出现
    vif({ (state.wifiRuntime?.optJSONObject("wifi_result")?.opt("ok") as? Boolean) == true }) {
        vif({ state.wifiResultText != null }) { captionLine({ state.wifiResultText ?: "" }) }
        actionRow {
            primaryButton(
                "确认新网络（wifi_commit）",
                enabled = { row.online },
                loading = { state.actionBusy },
                icon = IconToken.Check,
            ) {
                state.sendWifiCommit(ctx, row.deviceId) { ok, msg ->
                    toast.show(msg, if (ok) "ok" else "error")
                }
            }
        }
        textNote(
            "确认后设备固定使用新网；不确认则回滚窗口结束后自动回到旧网。",
            micro = true,
        )
    }
    vif({ !row.online }) {
        textNote("设备离线：WiFi 指令需要设备在线才能接收，扫描/切网都做不了（保存不了，只能等它上线）。", micro = true)
    }
}

// ============================================================
// 底部：下发配置
// ============================================================

/**
 * 「下发配置」是配置 tab 的**唯一**写动作（Stage 01–07 的所有控件都只是改草稿）。
 *
 * 文案必须写「仅代表已排队」：202 = 设备在线已排队下发，200 = 只保存到服务端、设备下次
 * 接入自动推送——两种都不等于"已生效"，实际生效只能在设备上核对。
 */
private fun ViewContainer<*, *>.echoMuseConfigSubmit(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    groupLabel("下发配置")
    actionRow {
        primaryButton(
            "下发配置",
            loading = { state.actionBusy },
            loadingText = "下发中…",
            icon = IconToken.Save,
        ) {
            state.saveConfig(ctx, row.deviceId) { ok, msg ->
                toast.show(msg, if (ok) "ok" else "error")
            }
        }
        hGap(AdminSpace.sm)
        secondaryButton(
            "按设备现值重新填充",
            enabled = { !state.actionBusy },
            icon = IconToken.Refresh,
        ) { state.fillConfigDrafts(row) }
    }
    textNote(
        "「下发配置」仅代表已排队：设备在线时接口回 202（已排队下发），离线时只保存到服务端、" +
            "设备下次接入自动推送。两种都不等于已生效——请按听感/灯环在设备上核对。" +
            "上面留空的数值键不会下发（保留设备当前值）。",
        micro = true,
    )
}
