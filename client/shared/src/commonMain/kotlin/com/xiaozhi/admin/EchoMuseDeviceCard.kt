package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.avatar.Avatar
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.primitives.Text
import com.gearui.primitives.composite.Card
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.clickable
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.xiaozhi.admin.kit.LocalAdminSpacing
import com.xiaozhi.admin.kit.statusBadge

/** 第一阶段只替换列表卡片；所有管理动作进入同一个 Kuikly 弹窗。 */
@Composable
internal fun EchoMuseDeviceCard(row: EchoMuseDeviceRow, onOpen: () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth()
            .padding(bottom = LocalAdminSpacing.current.gutter)
            .clickable(onClick = onOpen),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(LocalAdminSpacing.current.cardPadding),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.sm),
        ) {
            Avatar(fallback = row.title.take(2), online = row.online, contentDescription = row.title)
            Column(modifier = Modifier.weight(1f)) {
                Text(text = row.title, style = Theme.typography.titleSmall, color = Theme.colors.foreground)
                EmNote(row.deviceId)
                if (row.subtitle.isNotEmpty()) EmNote(row.subtitle)
                if (row.ip.isNotEmpty()) EmNote(row.ip)
                if (!row.online || !row.approved) {
                    Row(horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.xs)) {
                        if (!row.online) statusBadge(state = "error", text = "离线")
                        if (!row.approved) statusBadge(state = "busy", text = "待批准")
                    }
                }
            }
            Button(text = "管理", onClick = onOpen, type = ButtonType.OUTLINE)
        }
    }
}
