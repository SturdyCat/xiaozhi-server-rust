package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledSelect

/**
 * 日志 tab：**固定来源白名单**下拉 + 读取 + 复制 + 整块输出（原「设备管理 → 日志」整块归位）。
 *
 * 安全边界（与后端 `system/logs?source=` 同一口径）：
 * - `source` 是白名单 **id**，不是文件名——设备端路径拼接在服务端 `log_command` 里固定，
 *   前端拿不到"读任意文件"的能力；
 * - 六项 id 与文案逐字来自 [EchoMuseLogsState]（服务端只认这 6 个 id，多一个会被 400）；
 * - 每次读取只取设备上那份文件的**尾部 48 KB**（崩溃现场够用，且不会把设备读卡）。
 *
 * 读取走 [EchoMuseDetailState.run]（同一台设备只有一把操作槽：`/system/…`、控制台、
 * 资产下发共用），所以这里不自己发请求、也不自己判在线。
 */

/** 日志输出块高度：比其它诊断块高，尽量一屏看完一段崩溃/断连现场。 */
private val EM_LOG_OUTPUT_HEIGHT = 320.dp

@Composable
internal fun EchoMuseLogsTab(state: EchoMuseState, toast: ToastState) {
    val detail = state.detail
    val logs = state.logs
    // 四态文案：非 emOS / 离线 / 正在忙 / 可读，一屏之内说清"为什么点不动"
    EmNote(
        when {
            !detail.emos -> "日志仅支持 base_os 为 emos 的设备。"
            !detail.online -> "设备离线：日志需设备在线才能读取（不用连数据线，但设备必须在网）。"
            detail.busy.isNotEmpty() -> "设备正忙（${detail.busy}）：同一台设备只有一把操作槽，等它结束再读。"
            else -> "来源为固定白名单（不含任意路径），每次读取设备上日志文件的尾部 48 KB。"
        },
    )
    labeledSelect(
        label = "日志来源（固定白名单）",
        selectedId = logs.logSource,
        options = logs.logSources,
        onSelect = { logs.logSource = it },
    )
    actionRow {
        Button(
            text = if (detail.busy.startsWith("system/logs")) "读取中…" else "读取日志",
            onClick = { logs.read() },
            type = ButtonType.OUTLINE,
            loading = detail.busy.startsWith("system/logs"),
            disabled = !logs.canRead,
            iconWidget = { GearIcon(token = IconToken.Text, size = 16.dp) },
        )
        Button(
            text = "复制日志",
            onClick = {
                if (copyToClipboard(logs.copyAll())) toast.show("日志已复制到剪贴板", "ok")
                else toast.show("复制失败：当前平台剪贴板不可用", "error")
            },
            type = ButtonType.OUTLINE,
            disabled = !(logs.loaded && !logs.crossDevice),
            iconWidget = { GearIcon(token = IconToken.Copy, size = 16.dp) },
        )
    }
    if (logs.crossDevice) {
        EmWarn(
            "当前设备已切换：上面这份日志属于另一台设备，请重新读取（不同设备的日志不会串台显示）。",
        )
    }
    if (logs.loaded && !logs.crossDevice) {
        EmNote(
            "日志来源：${logs.loadedSource}" +
                if (logs.historical) "（历史快照，连接状态已变化）" else "（手动读取快照）",
        )
        EmOutputBlock(
            text = logs.output.ifEmpty { "设备命令输出为空（退出码 0）" },
            height = EM_LOG_OUTPUT_HEIGHT,
        )
    }
    if (!logs.loaded) {
        EmNote("尚未读取：选好来源后点「读取日志」。输出为空说明设备命令退出码为 0 且文件为空。")
    }
}
