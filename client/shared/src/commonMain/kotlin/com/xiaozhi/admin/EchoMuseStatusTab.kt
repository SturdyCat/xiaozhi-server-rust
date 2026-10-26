package com.xiaozhi.admin

import com.xiaozhi.admin.ui.*

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.pager.Pager
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 状态 tab：设备事实（§6.5 逐行）+ 资源三块 + 输出音量 + 蓝牙摘要 + 系统诊断折叠区。
 *
 * **本 tab 的核心纪律：取不到的字段一律 `—`，不填 0、不估算、不留空行。**
 * 每个可能缺失的字段都以 **lambda** 传给 `infoRow({ label }, { value })`——
 * 一是在 attr 闭包内读 observable 才被 Kuikly 跟踪（§6.6 铁律之一），二是缺测判定
 * 与展示同处一行，不会出现"行还在但值是旧快照"的错位。
 *
 * 「恒为 `—`」的两行是**诚实口径**，不是没做完：
 * - `Latency`：RTT 是官方控制器自己 ping 出来的，本服务端没有这项测量（未测量）；
 * - `Flash 磨损`：固件不上报 eMMC 寿命（未采集）。
 * 这两行保留而不是删掉，是为了对齐官方信息架构——用户找得到位置，也看得到为什么空。
 *
 * 数据来源（已对 `runtime.rs` 逐个字段核实）：`stats`（设备上行标量白名单）、
 * `volume_state`/`mute_state`/`listen_state`/`playback_stats`/`pair_requested_at`/`link`，
 * 以及设备列表行（`EchoMuseDeviceRow`：ip/version/board/boot_reason/last_seen/settings 键数）。
 */

/** 缺测占位：全页统一（`—`，不是空串——空串看起来像"还没渲染"）。 */
internal const val EM_DASH = "—"

/** 数值 → 文本；null / 非有限 = [EM_DASH]（缺测绝不与 0 混）。 */
private fun emNum(value: Double?, decimals: Int = 0, unit: String = ""): String {
    if (value == null || !value.isFinite()) return EM_DASH
    val text = when (decimals) {
        0 -> kotlin.math.round(value).toLong().toString()
        1 -> (kotlin.math.round(value * 10.0) / 10.0).toString()
        else -> (kotlin.math.round(value * 100.0) / 100.0).toString()
    }
    return text + unit
}

/** 信号强度（dBm）。 */
internal fun emRssi(value: Double?): String = emNum(value, unit = " dBm")

/** WiFi 频率（MHz；设备上报的就是 MHz，不换算）。 */
internal fun emMhz(value: Double?): String = emNum(value, unit = " MHz")

/** 链路速率（Mbps）。 */
internal fun emMbps(value: Double?): String = emNum(value, unit = " Mbps")

/** 温度（℃）。 */
private fun emCelsius(value: Double?): String = emNum(value, unit = " ℃")

/** MB → GB 文本（一位小数）；null = `—`。 */
private fun emGb(mb: Double?): String = if (mb == null) EM_DASH else emNum(mb / 1024.0, decimals = 1)

/** 占比（0～1 → 百分比文本）。 */
private fun emRatio(part: Double?, total: Double?): Double? {
    if (part == null || total == null || total <= 0.0) return null
    return (part / total).coerceIn(0.0, 1.0)
}

/** 从快照取一层数值（`obj[key]`）。 */
private fun num(obj: com.tencent.kuikly.core.nvi.serialization.json.JSONObject?, key: String): Double? =
    (obj?.opt(key) as? Number)?.toDouble()

/** `runtime.stats[key]`。 */
private fun EchoMuseDetailState.stat(key: String): Double? = num(runtime?.optJSONObject("stats"), key)

/** 设备口径的「未采集」副标：写明是因为没采，不是"值为 0"。 */
private const val EM_NOT_COLLECTED = "未采集（固件不上报该字段）"

internal fun ViewContainer<*, *>.echoMuseStatusTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    val detail = state.detail
    echoMuseDeviceFacts(row, state, toast, ctx)
    echoMuseResources(state)
    echoMuseVolume(detail, ctx)
    echoMuseBluetoothSummary(state)
    echoMuseDiagnostics(detail, ctx)
}

// ============================================================
// 设备事实（§6.5 逐行；原「概览」面板的九行 + 复制信息全部在此）
// ============================================================

private fun ViewContainer<*, *>.echoMuseDeviceFacts(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    ctx: Pager,
) {
    val detail = state.detail
    groupLabel("设备")
    // 设备号/设备名：未改名时标题就是设备号，「设备名」行不重复渲染（原概览面板同规则）
    infoRow({ "设备号" }, { row.deviceId }, topGap = 0f)
    if (row.renamed) {
        infoRow({ "设备名" }, { row.label })
    }
    // IP：离线时官方会标「(last seen)」，我方同样把口径写出来（ip 来自最近一次 register）
    infoRow(
        { "IP" },
        { if (row.ip.isEmpty()) EM_DASH else row.ip + if (row.online) "" else "（最后上报）" },
    )
    infoRow({ "固件版本" }, { row.version.ifEmpty { EM_DASH } })
    if (row.board.isNotEmpty()) {
        infoRow({ "板型" }, { row.board })
    }
    if (row.baseOs.isNotEmpty()) {
        infoRow({ "系统" }, { row.baseOs })
    }
    // 内核：与「最近启动」同源（register 上报）；旧服务端/旧记录没有就整行不渲染
    if (row.kernelRelease.isNotEmpty()) {
        infoRow(
            { "内核" },
            { if (row.kernelArch.isEmpty()) row.kernelRelease else "${row.kernelRelease}（${row.kernelArch}）" },
        )
    }
    infoRow({ "WiFi 网络" }, { detail.runtime?.optJSONObject("stats")?.text("wifiSsid").orEmpty().ifEmpty { EM_DASH } })
    // 我方即服务端本身，没有 Home Assistant 卫星这层概念——固定文案而不是留空
    infoRow({ "语音助手" }, { "本服务端（无 Home Assistant 卫星）" })
    infoRow({ "状态" }, { if (row.online) "在线" else "离线" })
    if (row.metaText.isNotEmpty()) detailLine({ row.metaText }, topGap = 2f)

    // 音量：只读上报（下发在下面的「输出音量」块）
    infoRow({ "输出音量" }, { emNum(num(detail.runtime?.optJSONObject("volume_state"), "level")) })
    // 物理静音：三态，且**与音量为 0 是两件事**（设备主权：ADC 静音 + 拒绝 mic_start）
    infoRow({ "物理静音" }, { emMuteText(detail) })
    detailLine("物理静音与「输出音量为 0」是不同状态：静音由设备侧开关决定，音量 0 只是不出声。", topGap = 2f)

    // 链路：TLS/token 三态（§3.2）；不是 RSSI/频段（那两项在下面的资源块里）
    infoRow({ "链路（TLS/token）" }, { emLinkText(detail) })
    // 固件不上报 eMMC 寿命
    infoRow({ "Flash 磨损" }, { EM_DASH })
    detailLine(EM_NOT_COLLECTED, topGap = 2f)

    // 最近启动：原因 + 标识 + 最近接入（boot_reason/boot_id 来自 register 上报）
    infoRow(
        { "最近启动" },
        {
            if (row.bootReason.isEmpty() && row.bootId.isEmpty()) {
                EM_DASH
            } else {
                listOf(
                    row.bootReason.ifEmpty { "原因未知" },
                    row.bootId.take(8).ifEmpty { "标识未知" },
                    formatEpochSeconds(row.lastSeen).ifEmpty { "接入时间未知" },
                ).joinToString(" · ")
            }
        },
    )
    // 我方没有 fleet / local override 的概念：只说本机存了几项，不编造 M
    infoRow({ "配置" }, { "本机设置（${row.settings.length()} 项已保存）" })
    detailLine("在「配置」tab 修改并下发；这里只反映服务端已存了多少项。", topGap = 2f)

    // 配对入口：设备长按 action 5s 才会有 pair_requested_at；无值不渲染该行
    val pairAt = (detail.runtime?.opt("pair_requested_at") as? Number)?.toLong()
    vif({ pairAt != null }) {
        infoRow({ "配对申请" }, { pairAt?.let { formatEpochSeconds(it) } ?: EM_DASH })
    }

    // 「读取运行状态」按钮语义 → 进 tab 自动拉一次 + 这里的「刷新」（快照不自动轮询）
    actionRow {
        secondaryButton(
            "刷新运行状态",
            enabled = { detail.approved && detail.idle },
            loading = { detail.busy == "runtime" },
            loadingText = "读取中…",
            icon = IconToken.Refresh,
        ) { detail.readRuntime(ctx) }
        hGap(AdminSpace.sm)
        // web 端文本可划选复制；此按钮是 macOS/触屏等无划选平台的兜底
        secondaryButton("复制信息", icon = IconToken.Copy) {
            if (copyToClipboard(row.copyText)) toast.show("设备信息已复制到剪贴板", "ok")
            else toast.show("复制失败：当前平台剪贴板不可用", "error")
        }
    }
    textNote(
        text = {
            if (detail.online) "在线仅表示控制连接存在，并非心跳或实时监测；以下状态需手动刷新。"
            else "设备离线：运行状态只保留历史上报，不代表当前值；音量不可下发。"
        },
        micro = true,
    )
}

/** 物理静音三态：`是` / `否` / `未知`（设备没上报时为未知，绝不默认"否"）。 */
private fun emMuteText(detail: EchoMuseDetailState): String =
    when (detail.runtime?.optJSONObject("mute_state")?.opt("muted") as? Boolean) {
        true -> "是（设备侧已静音，麦克风被关）"
        false -> "否"
        null -> "未知（设备未上报）"
    }

/** 链路三态文案（与 `/runtime.link` 的两字段一一对应）。 */
private fun emLinkText(detail: EchoMuseDetailState): String {
    val link = detail.runtime?.optJSONObject("link") ?: return "未知（设备未上报）"
    val authed = link.opt("authenticated") as? Boolean
    val needToken = link.opt("token_required") as? Boolean
    return when {
        authed == true && needToken == true -> "明文 ws · token 已校验"
        authed == true -> "明文 ws · 未设 token（局域网信任）"
        authed == false -> "未认证（连接未通过 token 校验）"
        else -> "未知（设备未上报）"
    }
}

// ============================================================
// 资源：三个自组合资源条 + 三个指标块
// ============================================================

/**
 * 资源块。`stats` 整个缺失时**不渲染半截数字**，只说「等待设备上报…」——
 * 这是设备上行遥测，不是服务端能推导的东西。
 */
private fun ViewContainer<*, *>.echoMuseResources(state: EchoMuseState) {
    val detail = state.detail
    groupLabel("资源")
    vif({ detail.runtime?.optJSONObject("stats") == null }) {
        detailLine("等待设备上报…（设备每轮上报一次标量快照；离线时只能看到历史上报）", topGap = 0f)
    }
    vif({ detail.runtime?.optJSONObject("stats") != null }) {
        emStatBar(
            "CPU",
            { emRatio(detail.stat("cpuPct"), 100.0) },
            { emNum(detail.stat("cpuPct"), unit = "%") },
            hint = {
                val online = detail.stat("coresOnline")
                val total = detail.stat("coresTotal")
                if (online == null || total == null) "核心数未上报" else "在线核心 ${online.toInt()} / ${total.toInt()}"
            },
        )
        emStatBar(
            "内存",
            { emRatio(detail.stat("memUsedMb"), detail.stat("memTotalMb")) },
            { "${emGb(detail.stat("memUsedMb"))} / ${emGb(detail.stat("memTotalMb"))} GB" },
            hint = { "设备上报的 MB 值折算（1 GB = 1024 MB）" },
        )
        emStatBar(
            "存储",
            { emRatio(detail.stat("storageUsedMb"), detail.stat("storageTotalMb")) },
            { "${emGb(detail.stat("storageUsedMb"))} / ${emGb(detail.stat("storageTotalMb"))} GB" },
            hint = { "系统分区占用；OWW 资产装的是 /data，见「更新」tab" },
        )
        splitRow(divider = false, spacing = AdminSpace.md) {
            grow {
                emStatTile(
                    "链路（RSSI / 频段）",
                    { emRssi(detail.stat("wifiRssi")) },
                    { "丢包带 $EM_DASH 未采集（设备不报分钟级丢包）" },
                )
            }
            grow { emStatTile("延迟（RTT）", { EM_DASH }, { "未测量：官方由控制器 ping，本服务端不测" }) }
            grow {
                emStatTile(
                    "温度（当前 / 峰值）",
                    {
                        val now = detail.stat("cpuTempC")
                        val max = detail.stat("maxTempC")
                        if (now == null && max == null) EM_DASH else "${emCelsius(now)} / ${emCelsius(max)}"
                    },
                    { emThermalHint(detail) },
                )
            }
        }
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

/**
 * 资源条（自组合，不用官方 Slider/ProgressBar）：标题 + 读数 + 1px 轨道。
 *
 * 轨道用两个 `flex(weight)` 段拼（Kuikly 没有百分比宽度）；比例在 `attr {}` 里读，
 * 所以同一次重渲染就能跟着新快照变宽窄——这也是 `ratio` 必须传 lambda 的原因。
 */
private fun ViewContainer<*, *>.emStatBar(
    label: String,
    ratio: () -> Double?,
    value: () -> String,
    hint: () -> String,
) {
    View {
        attr {
            flexDirectionColumn()
            marginTop(AdminSpace.sm)
        }
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                    text(label)
                }
            }
            Text {
                attr {
                    fontSize(AdminType.caption)
                    color(AdminColors.textPrimary)
                    text(value())
                }
            }
        }
        View {
            attr {
                height(6f)
                marginTop(AdminSpace.xs)
                flexDirectionRow()
                borderRadius(AdminShape.radiusPill)
                backgroundColor(AdminColors.divider)
            }
            View {
                attr {
                    val used = (ratio() ?: 0.0).coerceIn(0.0, 1.0).toFloat()
                    if (used > 0f) flex(used)
                    height(6f)
                    borderRadius(AdminShape.radiusPill)
                    backgroundColor(AdminColors.accent)
                }
            }
            View {
                attr {
                    val used = (ratio() ?: 0.0).coerceIn(0.0, 1.0).toFloat()
                    if (used < 1f) flex((1f - used).coerceAtLeast(0.001f))
                    height(6f)
                }
            }
        }
        detailLine(hint, topGap = 2f)
    }
}
/** 指标块：小标题 + 大号读数 + 一句副标（副标必填：数字脱离口径就是误读来源）。 */
internal fun ViewContainer<*, *>.emStatTile(label: String, value: () -> String, hint: () -> String) {
    View {
        attr {
            flexDirectionColumn()
            paddingTop(AdminSpace.sm)
        }
        Text {
            attr {
                fontSize(AdminType.micro)
                color(AdminColors.textTertiary)
                text(label)
            }
        }
        Text {
            attr {
                fontSize(AdminType.section)
                fontWeightMedium()
                color(AdminColors.textPrimary)
                marginTop(2f)
                text(value())
            }
        }
        detailLine(hint, topGap = 2f)
    }
}

// ============================================================
// 输出音量（下发）
// ============================================================

private fun ViewContainer<*, *>.echoMuseVolume(detail: EchoMuseDetailState, ctx: Pager) {
    groupLabel("输出音量（下发）")
    vif({ !detail.emos || !detail.speaker }) {
        textNote(
            text = {
                if (!detail.emos) "首版音量管理仅支持 emOS 设备。"
                else "设备未上报 speaker 能力，输出音量不可下发。"
            },
            micro = true,
        )
    }
    labeledIntField(
        "输出音量（0～127）",
        { detail.volumeDraft },
        { detail.volumeDraft = it },
        "填写整数；读取运行状态不会覆盖此草稿",
    )
    actionRow {
        primaryButton(
            "下发音量",
            enabled = { detail.canVolume && detail.idle },
            loading = { detail.busy == "volume" },
            loadingText = "下发中…",
            icon = IconToken.Speaker,
        ) { detail.setVolume(ctx) }
    }
    textNote("请求成功仅代表排队，实际上报音量请看上面的「输出音量」行（重新读取运行状态后更新）。", micro = true)
}

// ============================================================
// 蓝牙摘要
// ============================================================

/** 蓝牙代理摘要：状态来自 [EchoMuseBleState]（通用 GATT 工具在「配置 → 06 蓝牙」里）。 */
private fun ViewContainer<*, *>.echoMuseBluetoothSummary(state: EchoMuseState) {
    groupLabel("蓝牙代理")
    infoRow(
        { "状态" },
        { state.ble.status.ifEmpty { "未使用（通用 GATT 工具在「配置 → 06 蓝牙」）" } },
        topGap = 0f,
    )
    detailLine({ state.ble.advertsSummary }, topGap = 2f)
    textNote("仅面向你拥有或获授权的 IoT 外设；扫描/连接/读写全部在配置 tab 手动作业，本页只显示状态。", micro = true)
}

// ============================================================
// 系统诊断（原始输出，默认收起）
// ============================================================

/**
 * 系统诊断：默认**收起**（原文可能几百行，一进状态页就铺开会把资源块挤到屏外）。
 * 展开后给「读取一次」按钮 + 原文输出块 + 历史快照标注（会话变了要标明不是当前值）。
 */
private fun ViewContainer<*, *>.echoMuseDiagnostics(detail: EchoMuseDetailState, ctx: Pager) {
    groupLabel("系统诊断（原始输出）")
    actionRow {
        secondaryButton(
            "展开原始输出",
            dynamicText = { if (detail.infoOpen) "收起原始输出" else "展开原始输出" },
            icon = if (detail.infoOpen) IconToken.ChevronUp else IconToken.ChevronDown,
        ) { detail.infoOpen = !detail.infoOpen }
        hGap(AdminSpace.sm)
        secondaryButton(
            "读取系统信息",
            enabled = { detail.canSystem && detail.idle },
            loading = { detail.busy == "system/info" },
            loadingText = "读取中…",
            icon = IconToken.Info,
        ) { detail.readInfo(ctx) }
    }
    textNote(
        text = {
            when {
                !detail.emos -> "系统信息仅支持 base_os 为 emos 的设备。"
                !detail.online -> "设备离线：系统信息暂不可用。"
                else -> "手动读取一次快照（设备命令退出码 0 才算成功）；不会自动刷新。"
            }
        },
        micro = true,
    )
    vif({ detail.infoOpen }) {
        vif({ detail.infoLoaded }) {
            textNote(
                text = {
                    if (detail.infoHistorical) "系统信息：历史快照（连接状态已变化）" else "系统信息：手动读取快照"
                },
                micro = true,
            )
            outputBlock(
                content = { detail.infoOutput.ifEmpty { "设备命令输出为空（退出码 0）" } },
                height = 220f,
            )
        }
        vif({ !detail.infoLoaded }) {
            detailLine("尚未读取：点上面的「读取系统信息」。", topGap = 0f)
        }
    }
}
