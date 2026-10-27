package com.tencent.kuikly.compose.material3

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import com.tencent.kuikly.compose.foundation.Canvas
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.border
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.RowScope
import com.tencent.kuikly.compose.foundation.layout.heightIn
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.size
import com.tencent.kuikly.compose.foundation.selection.selectable
import com.tencent.kuikly.compose.foundation.shape.RoundedCornerShape
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.draw.clip
import com.tencent.kuikly.compose.ui.geometry.Offset
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.graphics.StrokeCap
import com.tencent.kuikly.compose.ui.semantics.Role
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp

/**
 * `SegmentedButton` 系列 —— Kuikly Compose 2.29.2 material3 的**缺失补丁**。
 *
 * Compose-Settings 的 `SettingsSegmented` 依赖 `SegmentedButton` /
 * `SegmentedButtonColors` / `SegmentedButtonDefaults` / `SingleChoiceSegmentedButtonRow`，
 * 2.29.2 制品里这四类符号全部不存在（不是版本差异，是缺位）。
 *
 * ## 与官方 M3 的差异（有意为之）
 * - `itemShape` **统一返回胶囊圆角**，而不是官方"首段左圆角 / 中段直角 / 末段右圆角"。
 *   原因：Kuikly 下每个 `SegmentedButton` 自带 1dp 边框，官方那种拼法在相邻边会叠出双线，
 *   必须引入"只画外侧边框"的绘制逻辑才能修，代价远大于收益。胶囊组在暗色卡片上
 *   层级清晰、点选反馈明确，与现有管理台的分段选择语义一致。
 * - 未实现 `interactionSource` 与悬停涟漪：Kuikly 的指示器 API 与官方不一致。
 * - 选中态对勾用 Canvas 画（项目规范：⛔ 不用文本字符当图标）。
 */

private val SegmentedMinHeight: Dp = 40.dp
private val SegmentedHorizontalPadding: Dp = 12.dp
private val SegmentedIconGap: Dp = 6.dp
private val SegmentedCornerRadius: Dp = 20.dp

@Immutable
class SegmentedButtonColors(
    val activeContainerColor: Color,
    val activeContentColor: Color,
    val activeBorderColor: Color,
    val inactiveContainerColor: Color,
    val inactiveContentColor: Color,
    val inactiveBorderColor: Color,
    val disabledActiveContainerColor: Color,
    val disabledActiveContentColor: Color,
    val disabledActiveBorderColor: Color,
    val disabledInactiveContainerColor: Color,
    val disabledInactiveContentColor: Color,
    val disabledInactiveBorderColor: Color,
) {
    fun copy(
        activeContainerColor: Color = this.activeContainerColor,
        activeContentColor: Color = this.activeContentColor,
        activeBorderColor: Color = this.activeBorderColor,
        inactiveContainerColor: Color = this.inactiveContainerColor,
        inactiveContentColor: Color = this.inactiveContentColor,
        inactiveBorderColor: Color = this.inactiveBorderColor,
        disabledActiveContainerColor: Color = this.disabledActiveContainerColor,
        disabledActiveContentColor: Color = this.disabledActiveContentColor,
        disabledActiveBorderColor: Color = this.disabledActiveBorderColor,
        disabledInactiveContainerColor: Color = this.disabledInactiveContainerColor,
        disabledInactiveContentColor: Color = this.disabledInactiveContentColor,
        disabledInactiveBorderColor: Color = this.disabledInactiveBorderColor,
    ): SegmentedButtonColors = SegmentedButtonColors(
        activeContainerColor = activeContainerColor,
        activeContentColor = activeContentColor,
        activeBorderColor = activeBorderColor,
        inactiveContainerColor = inactiveContainerColor,
        inactiveContentColor = inactiveContentColor,
        inactiveBorderColor = inactiveBorderColor,
        disabledActiveContainerColor = disabledActiveContainerColor,
        disabledActiveContentColor = disabledActiveContentColor,
        disabledActiveBorderColor = disabledActiveBorderColor,
        disabledInactiveContainerColor = disabledInactiveContainerColor,
        disabledInactiveContentColor = disabledInactiveContentColor,
        disabledInactiveBorderColor = disabledInactiveBorderColor,
    )
}

object SegmentedButtonDefaults {

    /** 相邻段之间的间隙，也是 `SettingsSegmented.buttonSpace` 的默认值。 */
    val BorderWidth: Dp = 1.dp

    val IconSize: Dp = 16.dp

    @Composable
    fun colors(
        activeContainerColor: Color = MaterialTheme.colorScheme.secondaryContainer,
        activeContentColor: Color = MaterialTheme.colorScheme.onSecondaryContainer,
        activeBorderColor: Color = Color.Transparent,
        inactiveContainerColor: Color = Color.Transparent,
        inactiveContentColor: Color = MaterialTheme.colorScheme.onSurface,
        inactiveBorderColor: Color = MaterialTheme.colorScheme.outline,
        disabledActiveContainerColor: Color = activeContainerColor.copy(alpha = 0.38f),
        disabledActiveContentColor: Color = activeContentColor.copy(alpha = 0.38f),
        disabledActiveBorderColor: Color = activeBorderColor.copy(alpha = 0.38f),
        disabledInactiveContainerColor: Color = inactiveContainerColor.copy(alpha = 0.38f),
        disabledInactiveContentColor: Color = inactiveContentColor.copy(alpha = 0.38f),
        disabledInactiveBorderColor: Color = inactiveBorderColor.copy(alpha = 0.38f),
    ): SegmentedButtonColors = SegmentedButtonColors(
        activeContainerColor = activeContainerColor,
        activeContentColor = activeContentColor,
        activeBorderColor = activeBorderColor,
        inactiveContainerColor = inactiveContainerColor,
        inactiveContentColor = inactiveContentColor,
        inactiveBorderColor = inactiveBorderColor,
        disabledActiveContainerColor = disabledActiveContainerColor,
        disabledActiveContentColor = disabledActiveContentColor,
        disabledActiveBorderColor = disabledActiveBorderColor,
        disabledInactiveContainerColor = disabledInactiveContainerColor,
        disabledInactiveContentColor = disabledInactiveContentColor,
        disabledInactiveBorderColor = disabledInactiveBorderColor,
    )

    /** 见文件头：统一胶囊，不做首/中/末差异化圆角（会叠双线）。 */
    @Composable
    fun itemShape(index: Int, count: Int): Shape = RoundedCornerShape(SegmentedCornerRadius)

    /** 选中态对勾。未选中时不占位（与官方一致，避免未选中项左侧留空）。 */
    @Composable
    fun Icon(selected: Boolean, active: Boolean = selected) {
        if (!active) return
        val tint = LocalContentColor.current
        Canvas(modifier = Modifier.size(IconSize)) {
            val w = size.width
            val h = size.height
            val stroke = w * 0.14f
            // 对勾：左下 → 底中 → 右上
            drawLine(
                color = tint,
                start = Offset(w * 0.18f, h * 0.52f),
                end = Offset(w * 0.42f, h * 0.76f),
                strokeWidth = stroke,
                cap = StrokeCap.Round,
            )
            drawLine(
                color = tint,
                start = Offset(w * 0.42f, h * 0.76f),
                end = Offset(w * 0.82f, h * 0.24f),
                strokeWidth = stroke,
                cap = StrokeCap.Round,
            )
        }
    }
}

/** 单选分段容器：等分铺满，段间由 [space] 分隔。 */
@Composable
fun SingleChoiceSegmentedButtonRow(
    modifier: Modifier = Modifier,
    space: Dp = 0.dp,
    content: @Composable RowScope.() -> Unit,
) {
    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(space),
        verticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

@Composable
fun RowScope.SegmentedButton(
    selected: Boolean,
    onClick: () -> Unit,
    shape: Shape,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    label: @Composable () -> Unit = {},
    icon: @Composable () -> Unit = { SegmentedButtonDefaults.Icon(selected) },
    colors: SegmentedButtonColors = SegmentedButtonDefaults.colors(),
) {
    val containerColor = when {
        !enabled && selected -> colors.disabledActiveContainerColor
        !enabled -> colors.disabledInactiveContainerColor
        selected -> colors.activeContainerColor
        else -> colors.inactiveContainerColor
    }
    val contentColor = when {
        !enabled && selected -> colors.disabledActiveContentColor
        !enabled -> colors.disabledInactiveContentColor
        selected -> colors.activeContentColor
        else -> colors.inactiveContentColor
    }
    val borderColor = when {
        !enabled && selected -> colors.disabledActiveBorderColor
        !enabled -> colors.disabledInactiveBorderColor
        selected -> colors.activeBorderColor
        else -> colors.inactiveBorderColor
    }

    Row(
        modifier = modifier
            .weight(1f)
            .clip(shape)
            .background(containerColor)
            .border(SegmentedButtonDefaults.BorderWidth, borderColor, shape)
            .selectable(
                selected = selected,
                enabled = enabled,
                role = Role.RadioButton,
                onClick = onClick,
            )
            .heightIn(min = SegmentedMinHeight)
            .padding(horizontal = SegmentedHorizontalPadding),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CompositionLocalProvider(LocalContentColor provides contentColor) {
            icon()
            Row(modifier = Modifier.padding(start = SegmentedIconGap)) {
                ProvideTextStyle(value = MaterialTheme.typography.labelLarge) {
                    label()
                }
            }
        }
    }
}
