package com.xiaozhi.admin

import androidx.compose.runtime.Composable
import com.gearui.components.button.Button
import com.gearui.components.button.ButtonType
import com.gearui.foundation.layout.Spacing
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier

/**
 * 「全局配置」各 section 共用的**三块小零件**（不是表单层，只是文案/摘要行）：
 *
 * | 零件 | 用途 | 旧 DSL 对应 |
 * | --- | --- | --- |
 * | [HintLine] | 只读说明行（服务端 schema 的整段提示、口径说明） | `textNote(…, micro = true)` |
 * | [WarnLine] | 警示说明行（就地错误、偏离原因） | `textNote(…, tone = "warn" / "error")` |
 * | [SectionSummaryRow] | 摘要 + 右侧刷新按钮（只读投影类卡片的标准头行） | 手写 `View { Text + secondaryButton }` |
 *
 * 为什么抽出来：能力总览 / 能力健康 / MCP 工具 / OTA 入口四张卡的头行结构完全一样
 * （左摘要、右刷新、加载中禁用），各抄一份必然会漂移（旧实现就是四份手写 `View`）。
 * 颜色只取 GearUI 语义色（`mutedForeground` / `warning` / `destructive`），**不引入自研色值**。
 */

/** 只读说明行：次要文字色、`caption` 字号。 */
@Composable
internal fun HintLine(text: String) {
    Text(
        text = text,
        modifier = Modifier.padding(top = Spacing.xs),
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
        modifier = Modifier.padding(top = Spacing.xs),
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
        )
    }
}
