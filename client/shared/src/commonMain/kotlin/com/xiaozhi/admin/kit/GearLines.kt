package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import com.gearui.foundation.primitives.Text
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Color

/**
 * 正文行与标题行零件（kit 收口的两个真缺口）：
 *
 * | 零件 | 用途 | 与既有零件的分工 |
 * | --- | --- | --- |
 * | [bodyLine] | 正文行（bodyMedium / foreground） | caption 说明行走 [HintLine] / [WarnLine]；正文行从这里取 |
 * | [listRow] | 「标题吃剩余宽度 + 尾部徽标/按钮」行 | [fieldLabelRow] 是表单标签行（label 字号 + ⓘ + trailing 控制动作）；列表/清单行从这里取 |
 *
 * 抽出来：插件动态 UI（A2UI `text` / `result` 节点）、插件清单行、扩展点解析行此前
 * 各自手写 `Text` + padding / `Row` + `weight(1f)`，字号、间距与对齐必然漂移。
 * 颜色只取 GearUI 语义 token；标题行在 RowScope 内用 `weight` 分配剩余宽度（AGENTS
 * 组合身份约定），调用方不自算 weight/间距。
 */

/** 正文行：bodyMedium / foreground、占满宽度、与上一元素留 sm（与 [HintLine] 同节奏）。 */
@Composable
fun bodyLine(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = Theme.colors.foreground,
) {
    Text(
        text = text,
        modifier = modifier.fillMaxWidth().padding(top = LocalAdminSpacing.current.sm),
        style = Theme.typography.bodyMedium,
        color = color,
    )
}

/**
 * 标题行：左标题吃剩余宽度（RowScope `weight`），右可选尾部插槽（徽标/按钮），垂直居中。
 * 标题字号固定 bodyMedium / foreground；需要别的字号时先在 kit 里立项，不要在调用处覆写。
 */
@Composable
fun listRow(
    title: String,
    modifier: Modifier = Modifier,
    trailing: (@Composable () -> Unit)? = null,
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            modifier = Modifier.weight(1f),
            style = Theme.typography.bodyMedium,
            color = Theme.colors.foreground,
        )
        if (trailing != null) trailing()
    }
}
