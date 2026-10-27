package com.tencent.kuikly.compose.material3

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.border
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.size
import com.tencent.kuikly.compose.foundation.selection.selectable
import com.tencent.kuikly.compose.foundation.shape.CircleShape
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.semantics.Role
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp

/**
 * `RadioButton` —— Kuikly Compose 2.29.2 material3 的**缺失补丁**。
 *
 * Compose-Settings 的 `SettingsRadioButton` 依赖它，而 2.29.2 制品里没有该符号
 * （`RadioButton` / `RadioButtonColors` / `RadioButtonDefaults` 全缺）。
 *
 * 实现用 `Box` + `border` + 内点组合，不依赖 Canvas：官方 M3 用 Canvas 做动画过渡，
 * 但本项目的单选是"点一下立即变"，没有状态动画需求，组合实现更少踩 Kuikly 绘制的坑。
 *
 * 语义：`Modifier.selectable(role = Role.RadioButton)` —— 无障碍与键盘行为对齐官方，
 * 点击由外层 `Modifier.toggleable` 或本组件的 `onClick` 承担（`onClick = null` 时不可点）。
 */

private val RadioSize: Dp = 20.dp
private val RadioRingWidth: Dp = 2.dp
private val RadioDotSize: Dp = 10.dp

@Immutable
class RadioButtonColors(
    val selectedColor: Color,
    val unselectedColor: Color,
    val disabledSelectedColor: Color,
    val disabledUnselectedColor: Color,
) {
    fun copy(
        selectedColor: Color = this.selectedColor,
        unselectedColor: Color = this.unselectedColor,
        disabledSelectedColor: Color = this.disabledSelectedColor,
        disabledUnselectedColor: Color = this.disabledUnselectedColor,
    ): RadioButtonColors = RadioButtonColors(
        selectedColor = selectedColor,
        unselectedColor = unselectedColor,
        disabledSelectedColor = disabledSelectedColor,
        disabledUnselectedColor = disabledUnselectedColor,
    )
}

object RadioButtonDefaults {

    @Composable
    fun colors(
        selectedColor: Color = MaterialTheme.colorScheme.primary,
        unselectedColor: Color = MaterialTheme.colorScheme.onSurfaceVariant,
        disabledSelectedColor: Color = selectedColor.copy(alpha = 0.38f),
        disabledUnselectedColor: Color = unselectedColor.copy(alpha = 0.38f),
    ): RadioButtonColors = RadioButtonColors(
        selectedColor = selectedColor,
        unselectedColor = unselectedColor,
        disabledSelectedColor = disabledSelectedColor,
        disabledUnselectedColor = disabledUnselectedColor,
    )
}

@Composable
fun RadioButton(
    selected: Boolean,
    onClick: (() -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: RadioButtonColors = RadioButtonDefaults.colors(),
) {
    val color = when {
        !enabled && selected -> colors.disabledSelectedColor
        !enabled -> colors.disabledUnselectedColor
        selected -> colors.selectedColor
        else -> colors.unselectedColor
    }
    val clickModifier = if (onClick != null) {
        Modifier.selectable(
            selected = selected,
            enabled = enabled,
            role = Role.RadioButton,
            onClick = onClick,
        )
    } else {
        Modifier
    }

    Box(
        modifier = modifier.size(RadioSize).then(clickModifier),
        contentAlignment = Alignment.Center,
    ) {
        // 外环：选中/未选中都画，颜色由 colors 决定（暗色主题下未选中用次级文字色）
        Box(modifier = Modifier.size(RadioSize).border(RadioRingWidth, color, CircleShape))
        if (selected) {
            Box(modifier = Modifier.size(RadioDotSize).background(color, CircleShape))
        }
    }
}
