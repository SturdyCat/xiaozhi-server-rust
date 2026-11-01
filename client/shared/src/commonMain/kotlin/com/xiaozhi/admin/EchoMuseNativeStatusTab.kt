package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.dialog.ConfirmDialog
import com.gearui.components.progress.LinearProgress
import com.gearui.components.progress.ProgressStatus
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.BoxWithConstraints
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.GearUiBreakpoints
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledIntField
import com.xiaozhi.admin.kit.twoPane

/** Native projection of the official Device / Resources panels. Binding is owned by the modal. */
@Composable
internal fun EchoMuseNativeStatusTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    modifier: Modifier = Modifier,
) {
    val pager = LocalAdminPagerOrThrow()
    val detail = state.detail
    val bound = detail.target == row.deviceId
    // This tab owns only its confirmation lifetime; no polling, socket or competing request chain.
    DisposableEffect(row.deviceId, row.sessionId, row.online, row.approved) {
        onDispose { detail.confirmReboot = false }
    }
    val runtime = detail.runtime.takeIf { bound }
    val stats = runtime?.optJSONObject("stats")
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.sm)) {
        EchoMuseNativeSnapshotHeader(row, state)
        actionRow {
            Button(text = "读取运行状态", type = ButtonType.OUTLINE,
                loading = detail.busy == "runtime", disabled = !bound || !row.approved || !detail.idle,
                onClick = { detail.readRuntime(pager) })
            Button(text = "复制状态", type = ButtonType.TEXT,
                disabled = !bound || !detail.runtimeLoaded,
                onClick = {
                    toast.show(if (copyToClipboard(row.copyText + "\n\n" + detail.runtimeText)) "状态已复制" else "复制失败：剪贴板不可用")
                })
        }
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val wide = maxWidth.value >= GearUiBreakpoints.WORKSPACE_COMPACT
            twoPane(wide = { wide }, left = {
                EchoMuseNativePanel("设备") {
                    EmInfoRow("固件版本", row.version.ifEmpty { "未知" })
                    EmInfoRow("设备地址", row.ip.ifEmpty { "未知" } + if (!row.online && row.ip.isNotEmpty()) "（最近接入）" else "")
                    EmInfoRow("无线网络", stats?.text("wifiSsid")?.ifEmpty { "未知" } ?: "未知")
                    EmInfoRow("控制连接", if (row.online) "在线" else "离线 · 最近接入 ${formatEpochSeconds(row.lastSeen).ifEmpty { "未知" }}")
                    EmInfoRow("连接时长", if (row.online && row.connectedSecs >= 0) humanDuration(row.connectedSecs * 1000) else "未知")
                    EmInfoRow("输出音量", emNum(emNumber(runtime?.optJSONObject("volume_state"), "level")) + " / 127")
                    EmInfoRow("设备静音", when (runtime?.optJSONObject("mute_state")?.opt("muted")) { true -> "是"; false -> "否"; else -> "未知" })
                    EmInfoRow("管理链路", echoMuseNativeLinkText(runtime?.optJSONObject("link")))
                    EmInfoRow("启动原因", row.bootReason.ifEmpty { "未上报" })
                    EmInfoRow("内核", listOf(row.kernelRelease, row.kernelArch).filter { it.isNotEmpty() }.joinToString(" · ").ifEmpty { "未知" })
                    EmNote("链路认证与无线信号是不同指标；接口未上报设备 TLS 状态，不由页面协议推断。")
                }
            }, right = {
                EchoMuseNativePanel("资源") {
                    val cpu = emNumber(stats, "cpuPct")
                    EchoMuseNativeResource("CPU 使用率", emNum(cpu, unit = "%"), cpu?.div(100))
                    EmNote("在线核心 ${emNum(emNumber(stats, "coresOnline"))} / ${emNum(emNumber(stats, "coresTotal"))}；使用率基于在线核心容量。")
                    val usedRam = emNumber(stats, "memUsedMb")
                    val totalRam = emNumber(stats, "memTotalMb")
                    EchoMuseNativeResource("内存", "${emNum(usedRam)} / ${emNum(totalRam)} MB", emRatio(usedRam, totalRam))
                    val usedStorage = emNumber(stats, "storageUsedMb")
                    val totalStorage = emNumber(stats, "storageTotalMb")
                    EchoMuseNativeResource("存储", "${emGb(usedStorage)} / ${emGb(totalStorage)} GB", emRatio(usedStorage, totalStorage))
                    EmInfoRow("无线信号", emRssi(emNumber(stats, "wifiRssi")), topGap = LocalAdminSpacing.current.sm)
                    EmInfoRow("无线频率", emMhz(emNumber(stats, "wifiFreqMhz")))
                    EmInfoRow("链路速率", emMbps(emNumber(stats, "linkSpeedMbps")))
                    EmInfoRow("CPU 温度", emCelsius(emNumber(stats, "cpuTempC")))
                    EmInfoRow("最高温度", emCelsius(emNumber(stats, "maxTempC")))
                    val coreLimit = emNumber(stats, "thermalCoreLimit")
                    val cores = emNumber(stats, "coresTotal")
                    if (coreLimit != null && cores != null && coreLimit < cores) EmWarn("温控正在限制 CPU 核心：${emNum(coreLimit)} / ${emNum(cores)}")
                    EmNote("往返延迟、分钟丢包趋势、闪存磨损与 HA 卫星状态：当前接口不提供。缺测不按 0 或健康处理。")
                }
            })
        }
        EchoMuseNativePanel("设备操作") {
            labeledIntField(label = "输出音量（0～127）", value = detail.volumeDraft,
                onValueChange = { detail.volumeDraft = it }, enabled = bound && detail.canVolume && detail.idle,
                placeholder = "0 为静音", trailing = {
                    Button(text = "下发音量", type = ButtonType.OUTLINE,
                        loading = detail.busy == "volume", disabled = !bound || !detail.canVolume || !detail.idle,
                        onClick = { detail.setVolume(pager) })
                })
            if (!row.capabilities.contains("speaker")) EmNote("设备未声明扬声器能力，不能下发音量。")
            EmNote("排队响应不代表实际生效；下发后请重新读取运行状态核对。")
            actionRow {
                Button(text = "重启设备", type = ButtonType.OUTLINE,
                    loading = detail.busy == "system/reboot", disabled = !bound || !detail.canSystem || !detail.idle,
                    onClick = { detail.confirmReboot = true })
            }
            if (row.baseOs != "emos") EmNote("系统重启仅支持 emOS 设备。")
            else if (!row.online) EmNote("设备离线：音量与重启不可用。")
        }
        EchoMuseNativeOperationFeedback(state)
    }
    ConfirmDialog(visible = bound && detail.confirmReboot,
        title = "确认重启设备？", message = "将向“${row.title}”发送重启请求，当前连接与语音会中断。接口响应只确认请求发送，不代表重启已完成。",
        confirmText = "发送重启请求", cancelText = "取消", destructive = true,
        enabled = bound && detail.canSystem && detail.idle, dismissOnOutside = false,
        onConfirm = { detail.reboot(pager) }, onCancel = { detail.confirmReboot = false })
}

@Composable
internal fun EchoMuseNativePanel(title: String, content: @Composable () -> Unit) {
    Card(modifier = Modifier.fillMaxWidth().padding(bottom = LocalAdminSpacing.current.sm), borderColor = Theme.colors.border) {
        Text(text = title, style = Theme.typography.titleSmall, color = Theme.colors.foreground,
            modifier = Modifier.padding(bottom = LocalAdminSpacing.current.sm))
        content()
    }
}

@Composable
private fun EchoMuseNativeResource(label: String, value: String, ratio: Double?) {
    EmInfoRow(label, value, topGap = LocalAdminSpacing.current.sm)
    if (ratio != null && ratio.isFinite()) {
        LinearProgress(progress = ratio.toFloat().coerceIn(0f, 1f), modifier = Modifier.fillMaxWidth(),
            status = ProgressStatus.PRIMARY, showLabel = false, animated = false)
    } else EmNote("尚未上报，暂无用量读数")
}

@Composable
internal fun EchoMuseNativeSnapshotHeader(row: EchoMuseDeviceRow, state: EchoMuseState) {
    val detail = state.detail
    val runtime = detail.runtime.takeIf { detail.target == row.deviceId }
    val current = detail.target == row.deviceId && detail.runtimeCurrent && row.online
    Card(modifier = Modifier.fillMaxWidth(), containerColor = Theme.colors.muted, borderColor = Theme.colors.border) {
        Text(text = echoMuseNativeListenText(runtime?.optJSONObject("listen_state")?.text("state").orEmpty()),
            style = Theme.typography.titleMedium, color = if (current) Theme.colors.foreground else Theme.colors.mutedForeground)
        EmNote(if (runtime == null) "尚未读取设备快照" else if (current) "当前控制会话的最近上报 · 非实时监测" else "历史快照 · 不代表当前设备状态")
        val updated = (runtime?.opt("updated_at") as? Number)?.toLong()
        EmNote("上报时间：${updated?.let { formatEpochSeconds(it) }?.ifEmpty { "未知" } ?: "未知"}")
        runtime?.optJSONObject("listen_state")?.text("reason")?.takeIf { it.isNotEmpty() }?.let { EmWarn("聆听状态原因：$it") }
    }
}

@Composable
internal fun EchoMuseNativeOperationFeedback(state: EchoMuseState) {
    if (state.detail.error.isNotEmpty()) EmWarn(state.detail.error, error = true)
    if (state.detail.result.isNotEmpty()) EmNote(state.detail.result)
}

internal fun echoMuseNativeListenText(value: String): String = when (value) {
    "local" -> "本地聆听"
    "stream" -> "音频上行"
    "degraded" -> "聆听已降级"
    "" -> "聆听状态未知"
    else -> "未识别的聆听状态：$value"
}

private fun echoMuseNativeLinkText(link: com.tencent.kuikly.core.nvi.serialization.json.JSONObject?): String = when {
    link?.opt("authenticated") == false -> "未认证"
    link?.opt("authenticated") == true && link.opt("token_required") == true -> "令牌已校验"
    link?.opt("authenticated") == true && link.opt("token_required") == false -> "未设令牌（局域网信任）"
    else -> "未知"
}

// No new stateVar/stateList fields: device snapshots and operations remain in EchoMuseDetailState.
