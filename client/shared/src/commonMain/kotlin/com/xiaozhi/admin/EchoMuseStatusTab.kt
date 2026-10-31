package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledIntField
import com.xiaozhi.admin.kit.splitRow
import com.tencent.kuikly.compose.ui.unit.dp

/**
 * 状态 tab（Compose 版）：设备身份 → 双列（设备事实 │ 资源）→ 输出音量 → 蓝牙摘要 → 系统诊断。
 *
 * ## 为什么拆成三个文件
 *
 * 旧 `EchoMuseStatusTab.kt` 是 543 行 DSL。重写后按**职责**分成三块，任何一块都不再接近
 * 600 行上限，且各自的改动理由互不相干：
 *
 * | 文件 | 职责 | 变动的理由 |
 * |---|---|---|
 * | 本文件 | 组合顺序 + 音量下发 + 蓝牙摘要 + 诊断折叠 | 交互/布局 |
 * | `EchoMuseStatusFacts.kt` | §6.5 的设备事实逐行 + 复制信息 | 服务端上报了哪些字段 |
 * | `EchoMuseStatusResources.kt` | 资源量程条 + 三个指标块 | 遥测口径与阈值 |
 *
 * ## 分区顺序照抄官方，像素不照抄
 *
 * 官方 Status 的顺序是「设备身份（光环）→ 左右双列 Device/Resources → 音量 → 蓝牙 → 诊断」。
 * 这里保留顺序（用户找得到位置），但**卡片、行、条全部由本项目的组件库出**：
 * 光环 → `User` 的身份卡；自绘资源条 → `Meter`；折叠区 → 标题 + 条件渲染。
 */
@Composable
internal fun EchoMuseStatusTab(row: EchoMuseDeviceRow, state: EchoMuseState, toast: ToastState) {
    val detail = state.detail
    val pager = LocalAdminPagerOrThrow()

    EmDeviceHead(row)
    splitRow(divider = false, spacing = LocalAdminSpacing.current.lg) {
        grow { EchoMuseDeviceFactsView(row = row, state = state, toast = toast) }
        grow { EchoMuseResourcesView(state = state) }
    }
    EmVolumeBlock(detail = detail, onSend = { detail.setVolume(pager) })
    EmBluetoothSummary(state = state)
    EmDiagnostics(detail = detail, onRead = { detail.readInfo(pager) })
}

/**
 * 输出音量（下发）。
 *
 * 语义分层很重要：上面「设备事实」里的「输出音量」是**设备上报值**（只读），
 * 这里下发的是**目标值草稿**。两者用同一个数字会让用户以为下发立刻见效——
 * 实际是排队，生效要重新读运行状态。
 */
@Composable
private fun EmVolumeBlock(detail: EchoMuseDetailState, onSend: () -> Unit) {
    EmGroupLabel("输出音量（下发）")
    if (!detail.emos || !detail.speaker) {
        EmNote(
            if (!detail.emos) "首版音量管理仅支持 emOS 设备。"
            else "设备未上报 speaker 能力，输出音量不可下发。",
        )
    }
    labeledIntField(
        label = "输出音量（0～127）",
        value = detail.volumeDraft,
        onValueChange = { detail.volumeDraft = it },
        placeholder = "填写整数",
        note = "读取运行状态不会覆盖此草稿",
    )
    actionRow {
        Button(
            text = if (detail.busy == "volume") "下发中…" else "下发音量",
            onClick = onSend,
            type = ButtonType.FILL,
            loading = detail.busy == "volume",
            disabled = !(detail.canVolume && detail.idle),
            iconWidget = { GearIcon(token = IconToken.Speaker, size = 16.dp) },
        )
    }
    EmNote("请求成功仅代表排队，实际上报音量请看上面的「输出音量」行（重新读取运行状态后更新）。")
}

/**
 * 蓝牙代理摘要。
 *
 * 只显示状态：通用 GATT 工具（扫描 / 连接 / 读写 / 通知）在「配置 → 06 蓝牙」里，
 * 本页重复一遍会把状态页变成第二套工具面板。
 */
@Composable
private fun EmBluetoothSummary(state: EchoMuseState) {
    EmGroupLabel("蓝牙代理")
    EmInfoRow("状态", state.ble.status.ifEmpty { "未使用（通用 GATT 工具在「配置 → 06 蓝牙」）" })
    EmNote(state.ble.advertsSummary)
    EmNote("仅面向你拥有或获授权的 IoT 外设；扫描/连接/读写全部在配置 tab 手动作业，本页只显示状态。")
}

/**
 * 系统诊断（原始输出，默认**收起**）。
 *
 * 默认收起是因为原文可能几百行：一进状态页就铺开会把资源块挤到屏外。
 * 展开后给「读取一次」+ 原文块 + 历史快照标注（会话变了要标明这不是当前值）。
 */
@Composable
private fun EmDiagnostics(detail: EchoMuseDetailState, onRead: () -> Unit) {
    EmGroupLabel("系统诊断（原始输出）")
    actionRow {
        Button(
            text = if (detail.infoOpen) "收起原始输出" else "展开原始输出",
            onClick = { detail.infoOpen = !detail.infoOpen },
            type = ButtonType.OUTLINE,
            iconWidget = {
                GearIcon(
                    token = if (detail.infoOpen) IconToken.ChevronUp else IconToken.ChevronDown,
                    size = 16.dp,
                )
            },
        )
        Button(
            text = if (detail.busy == "system/info") "读取中…" else "读取系统信息",
            onClick = onRead,
            type = ButtonType.OUTLINE,
            loading = detail.busy == "system/info",
            disabled = !(detail.canSystem && detail.idle),
            iconWidget = { GearIcon(token = IconToken.Info, size = 16.dp) },
        )
    }
    EmNote(
        when {
            !detail.emos -> "系统信息仅支持 base_os 为 emos 的设备。"
            !detail.online -> "设备离线：系统信息暂不可用。"
            else -> "手动读取一次快照（设备命令退出码 0 才算成功）；不会自动刷新。"
        },
    )
    if (!detail.infoOpen) return
    if (!detail.infoLoaded) {
        EmNote("尚未读取：点上面的「读取系统信息」。")
        return
    }
    EmNote(if (detail.infoHistorical) "系统信息：历史快照（连接状态已变化）" else "系统信息：手动读取快照")
    EmOutputBlock(
        text = detail.infoOutput.ifEmpty { "设备命令输出为空（退出码 0）" },
        height = 220.dp,
        modifier = Modifier.fillMaxWidth(),
    )
}
