package com.tencent.kuikly.compose.material3

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.heightIn
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.shape.RoundedCornerShape
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.text.TextStyle
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp

/**
 * `ListItem` —— Kuikly Compose 2.29.2 material3 的**缺失补丁**（本模块自建，非上游重复定义）。
 *
 * ## 为什么需要它
 * Compose-Settings 的全部磁贴（`SettingsSwitch`/`SettingsCheckbox`/`SettingsMenuLink`/
 * `SettingsSlider`/`SettingsSegmented`）都是 `ListItem` 的包装（见 `SettingsTileScaffold`）。
 * 而 Kuikly 2.29.2 的 `material3` 制品里**只有** `ListItemContainerColor` 这类 token 常量，
 * 没有 `ListItem` 组件本体、`ListItemColors`、`ListItemDefaults`——实测见
 * `docs/` 与本文件顶部的符号探测记录，属于真实缺位。
 *
 * ## 与官方 M3 的差异
 * - `ListItemColors` 的字段是**公开 val**（官方为 private + `@Composable` 访问器）：
 *   上游 `SettingsGroup` 直接读 `colors.headlineColor`，公开属性是它唯一能编译的形态。
 * - 不实现 `interactionSource`/`indication` 参数：Kuikly 的 `Modifier.selectable` 签名
 *   与其不一致，磁贴的点击语义由外层 `Modifier.toggleable` 承担，这里不需要。
 * - `RectangleShape` 在 Kuikly 不存在，直角用 `RoundedCornerShape(0.dp)` 表达。
 *
 * ⚠️ 本文件只做「让上游组件能编译」的**最小实现**，不做视觉改造；配色由调用方通过
 * `colors` 注入（管理台在 `shared` 侧注入 AdminColors，见 `AdminComposeTheme.kt`）。
 */

/** 磁贴内容区最小高度（官方 M3 单行 ListItem 的 56dp 触控基线）。 */
private val ListItemMinHeight: Dp = 56.dp
private val ListItemHorizontalPadding: Dp = 16.dp
private val ListItemVerticalPadding: Dp = 8.dp
private val ListItemIconGap: Dp = 16.dp
private val ListItemContentGap: Dp = 2.dp

/**
 * 标题/副标题文字样式的**可注入槽位**（为本项目追加，官方没有）。
 *
 * 上游 `SettingsXxx` 没有 `textStyles` 参数，而 `ListItem` 内部若硬编码
 * `MaterialTheme.typography.*`，管理台的 `AdminType` 字号（14pt 基准 + 视口缩放）就进不来。
 * 由宿主在页面根用 `AdminComposeTheme` 提供，磁贴内部按 `局部值 ?: 官方默认` 取值，
 * 不提供时行为与官方一致。
 */
val LocalListItemHeadlineStyle = staticCompositionLocalOf<TextStyle?> { null }

/** 见 [LocalListItemHeadlineStyle]。 */
val LocalListItemSupportingStyle = staticCompositionLocalOf<TextStyle?> { null }

/** 磁贴配色。字段公开：上游 `SettingsGroup` 以属性方式读取 `headlineColor`。 */
@Immutable
class ListItemColors(
    val containerColor: Color = Color.Transparent,
    val headlineColor: Color = Color.Unspecified,
    val supportingColor: Color = Color.Unspecified,
    val leadingIconColor: Color = Color.Unspecified,
    val trailingIconColor: Color = Color.Unspecified,
    val overlineColor: Color = Color.Unspecified,
    val disabledHeadlineColor: Color = Color.Unspecified,
    val disabledSupportingColor: Color = Color.Unspecified,
    val disabledLeadingIconColor: Color = Color.Unspecified,
    val disabledTrailingIconColor: Color = Color.Unspecified,
    val disabledOverlineColor: Color = Color.Unspecified,
) {
    fun copy(
        containerColor: Color = this.containerColor,
        headlineColor: Color = this.headlineColor,
        supportingColor: Color = this.supportingColor,
        leadingIconColor: Color = this.leadingIconColor,
        trailingIconColor: Color = this.trailingIconColor,
        overlineColor: Color = this.overlineColor,
        disabledHeadlineColor: Color = this.disabledHeadlineColor,
        disabledSupportingColor: Color = this.disabledSupportingColor,
        disabledLeadingIconColor: Color = this.disabledLeadingIconColor,
        disabledTrailingIconColor: Color = this.disabledTrailingIconColor,
        disabledOverlineColor: Color = this.disabledOverlineColor,
    ): ListItemColors = ListItemColors(
        containerColor = containerColor,
        headlineColor = headlineColor,
        supportingColor = supportingColor,
        leadingIconColor = leadingIconColor,
        trailingIconColor = trailingIconColor,
        overlineColor = overlineColor,
        disabledHeadlineColor = disabledHeadlineColor,
        disabledSupportingColor = disabledSupportingColor,
        disabledLeadingIconColor = disabledLeadingIconColor,
        disabledTrailingIconColor = disabledTrailingIconColor,
        disabledOverlineColor = disabledOverlineColor,
    )
}

object ListItemDefaults {

    /** 官方默认无海拔。Compose-Settings 的 `SettingsTileDefaults.Elevation` 读这个值。 */
    val Elevation: Dp = 0.dp

    /** 直角。Kuikly 无 `RectangleShape`，用 0 圆角等价表达。 */
    val shape: Shape = RoundedCornerShape(0.dp)

    @Composable
    fun colors(
        containerColor: Color = Color.Transparent,
        headlineColor: Color = MaterialTheme.colorScheme.onSurface,
        supportingColor: Color = MaterialTheme.colorScheme.onSurfaceVariant,
        leadingIconColor: Color = MaterialTheme.colorScheme.onSurfaceVariant,
        trailingIconColor: Color = MaterialTheme.colorScheme.onSurfaceVariant,
        overlineColor: Color = MaterialTheme.colorScheme.onSurfaceVariant,
        disabledHeadlineColor: Color = headlineColor.copy(alpha = 0.38f),
        disabledSupportingColor: Color = supportingColor.copy(alpha = 0.38f),
        disabledLeadingIconColor: Color = leadingIconColor.copy(alpha = 0.38f),
        disabledTrailingIconColor: Color = trailingIconColor.copy(alpha = 0.38f),
        disabledOverlineColor: Color = overlineColor.copy(alpha = 0.38f),
    ): ListItemColors = ListItemColors(
        containerColor = containerColor,
        headlineColor = headlineColor,
        supportingColor = supportingColor,
        leadingIconColor = leadingIconColor,
        trailingIconColor = trailingIconColor,
        overlineColor = overlineColor,
        disabledHeadlineColor = disabledHeadlineColor,
        disabledSupportingColor = disabledSupportingColor,
        disabledLeadingIconColor = disabledLeadingIconColor,
        disabledTrailingIconColor = disabledTrailingIconColor,
        disabledOverlineColor = disabledOverlineColor,
    )
}

/**
 * 单行/多行磁贴容器：`[leading] [overline / headline / supporting] [trailing]`。
 *
 * 布局与官方 M3 一致：左右 16dp、上下 8dp、最小高 56dp、图标与文本间 16dp；
 * 文本颜色经 `LocalContentColor` + `ProvideTextStyle` 下发给标题/副标题，
 * 因此调用方的 `Text(...)` 不需要自己指定颜色与字号。
 */
@Composable
fun ListItem(
    headlineContent: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    overlineContent: @Composable (() -> Unit)? = null,
    supportingContent: @Composable (() -> Unit)? = null,
    leadingContent: @Composable (() -> Unit)? = null,
    trailingContent: @Composable (() -> Unit)? = null,
    colors: ListItemColors = ListItemDefaults.colors(),
    tonalElevation: Dp = ListItemDefaults.Elevation,
    shadowElevation: Dp = ListItemDefaults.Elevation,
) {
    // tonalElevation / shadowElevation 保留在签名里以匹配上游调用点；
    // 暗色管理台不吃海拔（卡片底色自己给），故此处不参与绘制。
    Row(
        modifier = modifier
            .background(colors.containerColor)
            .fillMaxWidth()
            .heightIn(min = ListItemMinHeight)
            .padding(horizontal = ListItemHorizontalPadding, vertical = ListItemVerticalPadding),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (leadingContent != null) {
            CompositionLocalProvider(LocalContentColor provides colors.leadingIconColor) {
                Box(
                    modifier = Modifier.padding(end = ListItemIconGap),
                    contentAlignment = Alignment.Center,
                ) {
                    leadingContent()
                }
            }
        }

        Column(modifier = Modifier.weight(1f)) {
            if (overlineContent != null) {
                CompositionLocalProvider(LocalContentColor provides colors.overlineColor) {
                    ProvideTextStyle(value = MaterialTheme.typography.labelMedium) {
                        overlineContent()
                    }
                }
            }
            // 标题是主体信息：先给内容色与标题字号，再让调用方的 Text 继承
            CompositionLocalProvider(LocalContentColor provides colors.headlineColor) {
                ProvideTextStyle(
                    value = LocalListItemHeadlineStyle.current ?: MaterialTheme.typography.bodyLarge,
                ) {
                    headlineContent()
                }
            }
            if (supportingContent != null) {
                Box(modifier = Modifier.padding(top = ListItemContentGap)) {
                    CompositionLocalProvider(LocalContentColor provides colors.supportingColor) {
                        ProvideTextStyle(
                            value = LocalListItemSupportingStyle.current ?: MaterialTheme.typography.bodyMedium,
                        ) {
                            supportingContent()
                        }
                    }
                }
            }
        }

        if (trailingContent != null) {
            CompositionLocalProvider(LocalContentColor provides colors.trailingIconColor) {
                Box(
                    modifier = Modifier.padding(start = ListItemIconGap),
                    contentAlignment = Alignment.Center,
                ) {
                    trailingContent()
                }
            }
        }
    }
}
