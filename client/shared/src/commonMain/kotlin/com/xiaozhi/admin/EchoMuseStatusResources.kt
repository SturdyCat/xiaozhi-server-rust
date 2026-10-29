package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.meter.Meter
import com.gearui.components.progress.ProgressStatus
import com.xiaozhi.admin.kit.GearUiSpace
import com.xiaozhi.admin.kit.splitRow

/**
 * 状态 tab 的**资源**半边：CPU / 内存 / 存储三条量程条 + 三个指标块。
 *
 * ## 为什么用 `Meter` 而不是自绘进度条
 *
 * 旧 DSL 的 `emStatBar` 是"两个 `flex` 段拼出的 1px 轨道"——那是 Kuikly 没有百分比宽度时的
 * 变通。Compose/GearUI 的 `Meter` 就是**有量程的标量**：`Meter(value, label, min, max, status,
 * format)`，语义（读数 + 量程 + 轨道）与旧实现一致，还自带无障碍的 `progressBarRangeInfo`。
 *
 * ## 三道纪律
 *
 * 1. **缺测不画半截条**：`Meter` 内部对 `min < max` 有 `require`，所以量程缺失（如
 *    `memTotalMb` 未上报）时**不调用** `Meter`，退回 `EmStatTile` 显示 `—` 与原因——
 *    传 0/0 会直接抛异常，而传 0..100 会画出一条"看起来很健康"的假条；
 * 2. **颜色只在越界时升级**（K5 四色语义）：<75% 主色 / ≥75% 黄 / ≥90% 红。全程品牌绿会让
 *    "快满了"和"很闲"长得一样；
 * 3. `stats` 整个缺失时不渲染任何数字，只说「等待设备上报…」——这是设备上行遥测，
 *    不是服务端能推导的东西。
 */
@Composable
internal fun EchoMuseResourcesView(state: EchoMuseState) {
    val detail = state.detail
    EmGroupLabel("资源", topGap = GearUiSpace.sm)

    if (detail.runtime?.optJSONObject("stats") == null) {
        EmNote("等待设备上报…（设备每轮上报一次标量快照；离线时只能看到历史上报）")
        return
    }

    // ---- CPU ----
    val cpu = detail.stat("cpuPct")
    if (cpu == null) {
        EmStatTile("CPU", EM_DASH, "设备未上报 cpuPct")
    } else {
        Meter(
            value = cpu.toFloat().coerceIn(0f, 100f),
            label = "CPU",
            min = 0f,
            max = 100f,
            status = emLoadStatus(cpu / 100.0),
            format = { "${kotlin.math.round(it).toInt()}%" },
        )
        val online = detail.stat("coresOnline")
        val total = detail.stat("coresTotal")
        EmNote(
            if (online == null || total == null) "核心数未上报"
            else "在线核心 ${online.toInt()} / ${total.toInt()}",
        )
    }

    // ---- 内存 ----
    val memUsed = detail.stat("memUsedMb")
    val memTotal = detail.stat("memTotalMb")
    if (memUsed != null && memTotal != null && memTotal > 0.0) {
        Meter(
            value = memUsed.toFloat().coerceIn(0f, memTotal.toFloat()),
            label = "内存",
            min = 0f,
            max = memTotal.toFloat(),
            status = emLoadStatus(memUsed / memTotal),
            format = { "${emGb(it.toDouble())} / ${emGb(memTotal)} GB" },
        )
        EmNote("设备上报的 MB 值折算（1 GB = 1024 MB）")
    } else {
        EmStatTile("内存", "${emGb(memUsed)} / ${emGb(memTotal)} GB", "设备未上报内存总量，量程不明按缺测显示")
    }

    // ---- 存储 ----
    val storageUsed = detail.stat("storageUsedMb")
    val storageTotal = detail.stat("storageTotalMb")
    if (storageUsed != null && storageTotal != null && storageTotal > 0.0) {
        Meter(
            value = storageUsed.toFloat().coerceIn(0f, storageTotal.toFloat()),
            label = "存储",
            min = 0f,
            max = storageTotal.toFloat(),
            status = emLoadStatus(storageUsed / storageTotal),
            format = { "${emGb(it.toDouble())} / ${emGb(storageTotal)} GB" },
        )
        EmNote("系统分区占用；OWW 资产装的是 /data，见「更新」tab")
    } else {
        EmStatTile("存储", "${emGb(storageUsed)} / ${emGb(storageTotal)} GB", "设备未上报存储总量，量程不明按缺测显示")
    }

    // ---- 三个无法用占用量程表达的指标（链路 / 延迟 / 温度） ----
    splitRow(divider = false, spacing = GearUiSpace.md) {
        grow {
            EmStatTile(
                "链路（RSSI）",
                emRssi(detail.stat("wifiRssi")),
                "丢包带 $EM_DASH 未采集（设备不报分钟级丢包）",
            )
        }
        grow {
            EmStatTile("延迟（RTT）", EM_DASH, "未测量：官方由控制器 ping，本服务端不测")
        }
        grow {
            EmStatTile(
                "温度（当前 / 峰值）",
                run {
                    val now = detail.stat("cpuTempC")
                    val max = detail.stat("maxTempC")
                    if (now == null && max == null) EM_DASH else "${emCelsius(now)} / ${emCelsius(max)}"
                },
                emThermalHint(detail),
            )
        }
    }
}

/**
 * 占用率 → 状态档（K5 四色语义的单调版本）。
 *
 * 阈值取 75% / 90%：这是"还能干、开始挤、要动手"的分界，与资源条自身的量程无关，
 * 所以放在渲染层而不是状态层（状态层不该关心显示阈值）。
 */
private fun emLoadStatus(ratio: Double?): ProgressStatus {
    if (ratio == null || !ratio.isFinite()) return ProgressStatus.PRIMARY
    return when {
        ratio >= 0.9 -> ProgressStatus.DANGER
        ratio >= 0.75 -> ProgressStatus.WARNING
        else -> ProgressStatus.PRIMARY
    }
}

/** 温度块的副标：`thermalCoreLimit < coresTotal` 是可真实判定的降频信号（三字段都在白名单）。 */
private fun emThermalHint(detail: EchoMuseDetailState): String {
    val limit = detail.stat("thermalCoreLimit") ?: return "降频判定所需字段未上报"
    val total = detail.stat("coresTotal") ?: return "降频判定所需字段未上报"
    return if (limit < total) {
        "已降频：热限 ${limit.toInt()} 核 < 共 ${total.toInt()} 核"
    } else {
        "未降频：热限 ${limit.toInt()} 核 ≥ 共 ${total.toInt()} 核"
    }
}

/** `runtime.stats[key]`（设备上行标量白名单）；未上报 = `null`，绝不填 0。 */
private fun EchoMuseDetailState.stat(key: String): Double? =
    emNumber(runtime?.optJSONObject("stats"), key)
