package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.key
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.dialog.ConfirmDialog
import com.gearui.components.progress.LinearProgress
import com.gearui.components.progress.ProgressStatus
import com.gearui.components.slider.Slider
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.semantics.contentDescription
import com.tencent.kuikly.compose.ui.semantics.semantics
import com.xiaozhi.admin.kit.GearUiBreakpoints
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.twoPane

/** Device / Resources follow the official StatusTab; runtime remains the sole device snapshot. */
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
    DisposableEffect(row.deviceId, row.sessionId, row.online, row.approved) {
        onDispose { detail.confirmReboot = false }
    }
    val runtime = detail.runtime.takeIf { bound }
    val stats = runtime?.optJSONObject("stats")
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.sm)) {
        if (runtime != null && !detail.runtimeCurrent) EmNote("历史上报 · 不代表当前设备状态")
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val wide = maxWidth.value >= GearUiBreakpoints.WORKSPACE_COMPACT
            twoPane(wide = { wide }, left = {
                EchoMuseNativePanel("设备") {
                    EmInfoRow("IP 地址", row.ip.ifEmpty { "未知" } + if (!row.online && row.ip.isNotEmpty()) "（最近接入）" else "")
                    EmInfoRow("固件版本", row.version.ifEmpty { "未知" })
                    EmInfoRow("无线网络", stats?.text("wifiSsid")?.ifEmpty { "未知" } ?: "未知")
                    Row(Modifier.fillMaxWidth()) {
                        Text("状态", style = Theme.typography.bodyMedium, color = Theme.colors.mutedForeground, modifier = Modifier.weight(1f))
                        EchoMuseConnectionText(row)
                    }
                    val volume = detail.volumeReported.takeIf { bound }
                    EmInfoRow("输出音量", volume?.let(EchoMuseVolume::percent) ?: "未上报", topGap = LocalAdminSpacing.current.sm)
                    key(row.deviceId, row.sessionId, detail.canVolume, detail.runtimeCurrent) {
                        // GearUI owns the gesture-local dragValue: runtime reports update the idle
                        // value, never the thumb being dragged; cancellation also restores the report.
                        Slider(value = volume?.toFloat() ?: 0f, onValueChange = {},
                            valueRange = 0f..127f, modifier = Modifier.fillMaxWidth().semantics { contentDescription = "设备音量" },
                            enabled = bound && detail.canVolume && detail.runtimeCurrent && volume != null && (detail.idle || detail.busy == "runtime"),
                            onChangeEnd = { value -> EchoMuseVolume.fromSlider(value)?.let { detail.setVolume(pager, it) } })
                    }
                    detail.volumePending?.let { EmNote("等待设备上报 · 目标 ${EchoMuseVolume.percent(it)}") }
                    EmInfoRow("管理链路", echoMuseNativeLinkText(runtime?.optJSONObject("link")))
                    EmInfoRow("上次启动", row.bootReason.ifEmpty { "未上报" })
                }
            }, right = {
                EchoMuseNativePanel("资源") {
                    val cpu = emNumber(stats, "cpuPct")
                    val cores = emNumber(stats, "coresOnline")
                    val totalCores = emNumber(stats, "coresTotal")
                    val coreText = if (cores != null) " · ${emNum(cores)}/${emNum(totalCores)} 核" else ""
                    EchoMuseNativeResource("CPU", emNum(cpu, unit = "%") + coreText, cpu?.div(100))
                    val usedRam = emNumber(stats, "memUsedMb")
                    val totalRam = emNumber(stats, "memTotalMb")
                    EchoMuseNativeResource("内存", "${emNum(usedRam)} / ${emNum(totalRam)} MB", emRatio(usedRam, totalRam))
                    val usedStorage = emNumber(stats, "storageUsedMb")
                    val totalStorage = emNumber(stats, "storageTotalMb")
                    EchoMuseNativeResource("存储", "${emGb(usedStorage)} / ${emGb(totalStorage)} GB", emRatio(usedStorage, totalStorage))
                    Row(Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.md), horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.md)) {
                        Column(Modifier.weight(1f)) {
                            Text("无线链路", style = Theme.typography.bodySmall, color = Theme.colors.mutedForeground)
                            Text(emRssi(emNumber(stats, "wifiRssi")), style = Theme.typography.titleSmall, color = Theme.colors.foreground)
                            EmNote(listOfNotNull(emNumber(stats, "wifiFreqMhz")?.let(::emMhz), emNumber(stats, "linkSpeedMbps")?.let(::emMbps)).joinToString(" · "))
                        }
                        Column(Modifier.weight(1f)) {
                            Text("CPU 温度", style = Theme.typography.bodySmall, color = Theme.colors.mutedForeground)
                            Text(emCelsius(emNumber(stats, "cpuTempC")), style = Theme.typography.titleSmall, color = Theme.colors.foreground)
                            emNumber(stats, "maxTempC")?.let { EmNote("最高 ${emCelsius(it)}") }
                        }
                    }
                    val coreLimit = emNumber(stats, "thermalCoreLimit")
                    if (coreLimit != null && totalCores != null && coreLimit < totalCores) EmWarn("温控限核：${emNum(coreLimit)} / ${emNum(totalCores)}")
                }
            })
        }
        EchoMuseNativeOperationFeedback(state)
    }
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
    }
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
