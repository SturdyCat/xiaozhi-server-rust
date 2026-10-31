package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.*
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.kit.GearLedRing
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.statusBadge

/** 官方 Card 的信息层级：名称/版本 → Dot 2 设备图 → 状态/IP，使用原生 Kuikly 绘制。 */
@Composable
internal fun EchoMuseDeviceCard(
    row: EchoMuseDeviceRow,
    onOpen: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = LocalAdminSpacing.current
    Card(modifier = modifier.clickable(onClick = onOpen)) {
        Column(modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.fillMaxWidth().background(Theme.colors.muted)
                .padding(horizontal = spacing.md, vertical = spacing.sm)) {
                Row(verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(spacing.xs)) {
                    Text(text = row.title, style = Theme.typography.titleSmall,
                        color = Theme.colors.foreground, maxLines = 1, modifier = Modifier.weight(1f))
                    if (!row.approved) statusBadge(state = "busy", text = "待批准")
                }
                Text(text = row.subtitle.ifEmpty { "版本未知" }, maxLines = 1,
                    style = Theme.typography.caption, color = Theme.colors.mutedForeground)
            }
            Box(modifier = Modifier.fillMaxWidth().padding(vertical = spacing.lg),
                contentAlignment = Alignment.Center) {
                EchoMuseDevicePicture(row, 120.dp)
            }
            Column(modifier = Modifier.fillMaxWidth().padding(horizontal = spacing.md)
                .padding(bottom = spacing.md)) {
                Column(modifier = Modifier.fillMaxWidth().background(Theme.colors.muted)
                    .padding(spacing.sm), horizontalAlignment = Alignment.CenterHorizontally) {
                    EchoMuseConnectionText(row)
                    Text(text = echoMuseDeviceAddress(row), style = Theme.typography.caption,
                        color = Theme.colors.mutedForeground, maxLines = 1)
                }
            }
        }
    }
}

/** 与官方 LedRing 同源的 Dot 2 轮廓，不用文字头像代替设备图。 */
@Composable
internal fun EchoMuseDevicePicture(row: EchoMuseDeviceRow, size: Dp) {
    val color = when {
        !row.approved -> Theme.colors.primary
        !row.online -> Theme.colors.warning
        else -> Theme.colors.mutedForeground
    }
    GearLedRing(size = size, segColor = color, dotColor = color)
}

@Composable
internal fun EchoMuseConnectionText(row: EchoMuseDeviceRow) {
    Text(text = when {
        !row.approved -> "待批准 · ${if (row.online) "在线" else "离线"}"
        row.online -> "在线"
        else -> "离线"
    }, style = Theme.typography.bodyMedium,
        color = if (row.online) Theme.colors.foreground else Theme.colors.warning)
}

internal fun echoMuseDeviceAddress(row: EchoMuseDeviceRow): String =
    row.ip.takeIf { it.isNotEmpty() && it != "127.0.0.1" }
        ?.let { if (row.online) it else "$it（上次地址）" } ?: "IP 未上报"
