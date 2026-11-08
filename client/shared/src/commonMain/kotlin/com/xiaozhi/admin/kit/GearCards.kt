package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.primitives.composite.Card
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.contracts.IconToken
import com.xiaozhi.admin.icons.GearIcon

/**
 * 卡片壳与分区文案行零件（kit 收口，原 `ConfigCards.kt` / `ConfigSectionScaffold.kt`
 * **逐字搬移**，签名/视觉参数零改动——纯搬移不漂移像素）：
 *
 * | 零件 | 用途 | 旧 DSL 对应 |
 * | --- | --- | --- |
 * | [LabeledCard] | 带标题卡片壳：所有卡片共用的最小外壳（卡片堆叠节奏 `bottom=lg` 在这里） | `groupedCard(title, withDivider)` |
 * | [HintLine] | 只读说明行（caption / mutedForeground） | `textNote(…, micro = true)` |
 * | [WarnLine] | 警示说明行（就地错误、偏离原因；K5「降级黄、异常红」） | `textNote(…, tone = "warn" / "error")` |
 * | [SectionSummaryRow] | 摘要 + 右侧动作按钮（只读投影类卡片的标准头行） | 手写 `View { Text + secondaryButton }` |
 *
 * 为什么收口到 kit：卡片壳、说明行、摘要行此前停在配置分发的脚手架文件里，但调用方
 * 早已遍布插件/运维/OTA/控制台等全部页面——kit 是零件库的唯一出处，搬移后出处与索引
 * 一致（AGENTS kit 索引同步）。颜色只取 GearUI 语义 token，不引入自研色值。
 * 正文行 / 标题行见 [bodyLine] / [listRow]（GearLines.kt）。
 */

/**
 * 带标题的卡片壳（`Card` + 标题行）：所有 section 卡片共用的最小外壳。
 *
 * 旧 DSL 的 `groupedCard(title, withDivider)` 在这里只有"卡片 + 标题"两层，
 * 内部分隔线交给各组自己用 `dividerH()`（Compose 版见 `kit/GearLayout.kt`）。
 */
@Composable
fun LabeledCard(
    title: String,
    modifier: Modifier = Modifier,
    trailing: (@Composable () -> Unit)? = null,
    content: @Composable () -> Unit,
) {
    Card(modifier = modifier.fillMaxWidth().padding(bottom = Spacing.lg)) {
        Column(modifier = Modifier.fillMaxWidth().padding(Spacing.lg)) {
            Row(
                // 标题行与正文之间留 md（12dp）：卡片标题是"区块名"，与首行内容要有一档
                // 可感知的呼吸感；sm（8dp）在标题+长正文时视觉上贴在一起。
                modifier = Modifier.fillMaxWidth().padding(bottom = Spacing.md),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = title,
                    modifier = Modifier.weight(1f),
                    style = Theme.typography.titleSmall,
                    color = Theme.colors.foreground,
                )
                if (trailing != null) Box(Modifier.padding(start = Spacing.sm)) { trailing() }
            }
            content()
        }
    }
}

/** 只读说明行：次要文字色、`caption` 字号。与上一个元素留 sm（8dp），说明行不再贴着正文。 */
@Composable
internal fun HintLine(text: String) {
    Text(
        text = text,
        modifier = Modifier.padding(top = Spacing.sm),
        style = Theme.typography.caption,
        color = Theme.colors.mutedForeground,
    )
}

/**
 * 警示说明行：降级/失败原因、就地错误。
 *
 * [error] = true 用 `destructive`（异常），false 用 `warning`（降级/注意事项）——
 * 与 K5「降级黄、异常红」同一套语义色来源。
 */
@Composable
internal fun WarnLine(text: String, error: Boolean = false) {
    Text(
        text = text,
        modifier = Modifier.padding(top = Spacing.sm),
        style = Theme.typography.caption,
        color = if (error) Theme.colors.destructive else Theme.colors.warning,
    )
}

/**
 * 摘要行：左侧摘要文字（可空，空则只显示按钮），右侧动作按钮。
 *
 * 按钮走 GearUI `Button` 的 `loading` / `disabled`，不用外面再套一层"不可点"判断
 * （旧 DSL 是 `secondaryButton(loading = {...})` 自带三态）。
 */
@Composable
internal fun SectionSummaryRow(
    summary: String,
    actionText: String,
    busy: Boolean,
    busyText: String,
    onAction: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(top = Spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = summary,
            modifier = Modifier.weight(1f),
            style = Theme.typography.bodyMedium,
            color = Theme.colors.mutedForeground,
        )
        Button(
            text = if (busy) busyText else actionText,
            onClick = onAction,
            type = ButtonType.OUTLINE,
            loading = busy,
            // 加载中禁点：刷新是幂等只读请求，但仍要防连点造成列表来回闪
            disabled = busy,
            iconWidget = { color -> GearIcon(token = IconToken.Refresh, size = 16.dp, tint = color) },
        )
    }
}
