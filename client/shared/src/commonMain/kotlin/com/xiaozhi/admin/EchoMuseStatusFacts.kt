package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.components.user.User
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow

/**
 * 状态 tab 的**设备事实**半边：身份头 + §6.5 的逐行事实 + 副本入口。
 *
 * **本文件的核心纪律：取不到的字段一律 [EM_DASH]，不填 0、不估算、不留空行。**
 *
 * 「恒为 `—`」的两行是**诚实口径**，不是没做完：
 * - `延迟（RTT）`：RTT 是官方控制器自己 ping 出来的，本服务端没有这项测量（未测量）；
 * - `Flash 磨损`：固件不上报 eMMC 寿命（未采集）。
 * 这两行保留而不是删掉，是为了对齐官方信息架构——用户找得到位置，也看得到为什么空。
 *
 * 数据来源（已对 `runtime.rs` 逐字段核实）：`stats`（设备上行标量白名单）+ 设备列表行
 * （`EchoMuseDeviceRow`：ip/version/board/boot_reason/last_seen/settings 键数）。
 */

/** 设备口径的「未采集」副标：写明是因为没采，不是"值为 0"。 */
private const val EM_NOT_COLLECTED = "未采集（固件不上报该字段）"

/**
 * 身份头：`User`（头像 + 名称 + 副标题）。
 *
 * 用 GearUI 的 `User` 而不是自绘一行：头像回落（未设图时取名称首字）与在线点两件事
 * 本来就属于身份组件，页面不该各写一份。旧 DSL 的 `ledRing` 光环随 `ui/LedRing.kt`
 * 一起退出——光环颜色只有"在线/离线"两态时，与头像的在线点信息重复。
 */
@Composable
internal fun EmDeviceHead(row: EchoMuseDeviceRow) {
    Column(modifier = Modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.md, bottom = LocalAdminSpacing.current.xs)) {
        User(
            name = row.title,
            description = listOfNotNull(
                row.subtitle.takeIf { it.isNotEmpty() },
                (if (row.online) "在线" else "离线") + row.metaText.takeIf { it.isNotEmpty() }?.let { " · $it" }.orEmpty(),
            ).joinToString("\n"),
            fallback = row.title.take(2),
        )
    }
}

/** 设备事实（§6.5 逐行；原「概览」面板的九行 + 复制信息全部在此）。 */
@Composable
internal fun EchoMuseDeviceFactsView(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
) {
    val detail = state.detail
    val pager = LocalAdminPagerOrThrow()

    EmGroupLabel("设备", topGap = LocalAdminSpacing.current.sm)
    // 设备号/设备名：未改名时标题就是设备号，「设备名」行不重复渲染
    EmInfoRow("设备号", row.deviceId)
    if (row.renamed) {
        EmInfoRow("设备名", row.label)
    }
    // IP：离线时官方会标「(last seen)」，我方同样把口径写出来（ip 来自最近一次 register）
    EmInfoRow(
        "IP",
        if (row.ip.isEmpty()) EM_DASH else row.ip + if (row.online) "" else "（最后上报）",
    )
    EmInfoRow("固件版本", row.version.ifEmpty { EM_DASH })
    if (row.board.isNotEmpty()) EmInfoRow("板型", row.board)
    if (row.baseOs.isNotEmpty()) EmInfoRow("系统", row.baseOs)
    // 内核：与「最近启动」同源（register 上报）；旧服务端/旧记录没有就整行不渲染
    if (row.kernelRelease.isNotEmpty()) {
        EmInfoRow(
            "内核",
            if (row.kernelArch.isEmpty()) row.kernelRelease else "${row.kernelRelease}（${row.kernelArch}）",
        )
    }
    EmInfoRow(
        "WiFi 网络",
        detail.runtime?.optJSONObject("stats")?.text("wifiSsid").orEmpty().ifEmpty { EM_DASH },
    )
    // 我方即服务端本身，没有 Home Assistant 卫星这层概念——固定文案而不是留空
    EmInfoRow("语音助手", "本服务端（无 Home Assistant 卫星）")
    EmInfoRow("状态", if (row.online) "在线" else "离线")
    if (row.metaText.isNotEmpty()) EmNote(row.metaText)

    // 音量：只读上报（下发在下面的「输出音量」块）
    EmInfoRow("输出音量", emNum(emNumber(detail.runtime?.optJSONObject("volume_state"), "level")))
    // 物理静音：三态，且**与音量为 0 是两件事**（设备主权：ADC 静音 + 拒绝 mic_start）
    EmInfoRow("物理静音", emMuteText(detail))
    EmNote("物理静音与「输出音量为 0」是不同状态：静音由设备侧开关决定，音量 0 只是不出声。")

    // 链路：TLS/token 三态；不是 RSSI/频段（那两项在资源块里）
    EmInfoRow("链路（TLS/token）", emLinkText(detail))
    // 固件不上报 eMMC 寿命
    EmInfoRow("Flash 磨损", EM_DASH)
    EmNote(EM_NOT_COLLECTED)

    // 最近启动：原因 + 标识 + 最近接入（boot_reason/boot_id 来自 register 上报）
    EmInfoRow(
        "最近启动",
        if (row.bootReason.isEmpty() && row.bootId.isEmpty()) {
            EM_DASH
        } else {
            listOf(
                row.bootReason.ifEmpty { "原因未知" },
                row.bootId.take(8).ifEmpty { "标识未知" },
                formatEpochSeconds(row.lastSeen).ifEmpty { "接入时间未知" },
            ).joinToString(" · ")
        },
    )
    // 我方没有 fleet / local override 的概念：只说本机存了几项，不编造 M
    EmInfoRow("配置", "本机设置（${row.settings.length()} 项已保存）")
    EmNote("在「配置」tab 修改并下发；这里只反映服务端已存了多少项。")

    // 配对入口：设备长按 action 5s 才会有 pair_requested_at；无值不渲染该行
    val pairAt = (detail.runtime?.opt("pair_requested_at") as? Number)?.toLong()
    if (pairAt != null) {
        EmInfoRow("配对申请", formatEpochSeconds(pairAt).ifEmpty { EM_DASH })
    }

    // 「读取运行状态」→ 进 tab 自动拉一次 + 这里的「刷新」（快照不自动轮询）
    actionRow {
        Button(
            text = if (detail.busy == "runtime") "读取中…" else "刷新运行状态",
            onClick = { detail.readRuntime(pager) },
            type = ButtonType.OUTLINE,
            loading = detail.busy == "runtime",
            disabled = !(detail.approved && detail.idle),
            iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
        )
        // web 端文本可划选复制；此按钮是 macOS/触屏等无划选平台的兜底
        Button(
            text = "复制信息",
            onClick = {
                if (copyToClipboard(row.copyText)) toast.show("设备信息已复制到剪贴板", "ok")
                else toast.show("复制失败：当前平台剪贴板不可用", "error")
            },
            type = ButtonType.OUTLINE,
            iconWidget = { GearIcon(token = IconToken.Copy, size = 16.dp) },
        )
    }
    EmNote(
        if (detail.online) "在线仅表示控制连接存在，并非心跳或实时监测；以下状态需手动刷新。"
        else "设备离线：运行状态只保留历史上报，不代表当前值；音量不可下发。",
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
