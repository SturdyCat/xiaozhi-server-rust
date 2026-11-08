package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.BoxWithConstraints
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon
import com.xiaozhi.admin.kit.LabeledCard
import com.xiaozhi.admin.kit.actionRow
import com.xiaozhi.admin.kit.labeledSelect

/**
 * Native system-log reader. The source is a server whitelist, and one read is
 * one bounded snapshot; no background polling is started here. The output box
 * owns its bounded scroll, so this entry does not add a second page scroll.
 */
@Composable
fun EchoMuseNativeLogsTab(
    row: EchoMuseDeviceRow,
    state: EchoMuseState,
    toast: ToastState,
    modifier: Modifier = Modifier,
) {
    val logs = state.logs
    val detail = state.detail
    val pager = LocalAdminPagerOrThrow()
    LabeledCard(title = "设备日志", modifier = modifier) {
        labeledSelect(
            label = "日志来源",
            selectedId = logs.logSource,
            options = logs.logSources,
            onSelect = { logs.logSource = it },
            placeholder = "选择日志来源",
        )
        actionRow {
            Button(
                text = if (logs.reading) "读取中…" else "读取日志",
                onClick = { logs.read() },
                type = ButtonType.FILL,
                loading = logs.reading,
                disabled = !logs.canRead || logs.reading,
                iconWidget = { GearIcon(token = IconToken.Refresh, size = 16.dp) },
            )
            Button(
                text = "复制日志",
                onClick = {
                    toast.show(
                        if (copyToClipboard(logs.copyAll())) "日志已复制" else "当前平台无法复制",
                        "info",
                    )
                },
                type = ButtonType.OUTLINE,
                disabled = !logs.loaded || logs.crossDevice || logs.output.isEmpty(),
            )
            Button(
                text = "清空",
                onClick = { logs.clear() },
                type = ButtonType.OUTLINE,
                disabled = !logs.loaded && logs.output.isEmpty(),
            )
        }
        if (!detail.canSystem) {
            EmWarn("读取日志需要已批准、在线且运行 emOS 的设备。")
        }
        if (logs.loadError.isNotEmpty()) {
            EmWarn(logs.loadError, error = true)
        }
        if (logs.historical) {
            EmWarn("以下是历史快照（设备会话或选中设备已变化），请重新读取。")
        }
        if (logs.loaded && !logs.crossDevice) {
            EmNote("来源：${logs.loadedSource.ifEmpty { logs.logSource }} · 手动快照；未伪造时间戳。")
            BoxWithConstraints(Modifier.fillMaxWidth().padding(top = 8.dp)) {
                val outputHeight = (maxHeight.value - 180f).coerceIn(120f, 320f).dp
                EmOutputBlock(logs.output, outputHeight)
            }
        } else if (logs.output.isEmpty() && logs.loadError.isEmpty()) {
            Text("尚未读取日志。", style = Theme.typography.bodySmall, color = Theme.colors.mutedForeground,
                modifier = Modifier.padding(top = 8.dp))
        }
        if (detail.error.isNotEmpty() && logs.loadError.isEmpty() && detail.busy == "system/logs?source=${logs.logSource}") {
            EmWarn(detail.error, error = true)
        }
    }
}
