package com.xiaozhi.admin.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import com.tencent.kuikly.compose.foundation.shape.RoundedCornerShape
import com.tencent.kuikly.compose.material3.CheckboxColors
import com.tencent.kuikly.compose.material3.CheckboxDefaults
import com.tencent.kuikly.compose.material3.ListItemColors
import com.tencent.kuikly.compose.material3.ListItemDefaults
import com.tencent.kuikly.compose.material3.LocalListItemHeadlineStyle
import com.tencent.kuikly.compose.material3.LocalListItemSupportingStyle
import com.tencent.kuikly.compose.material3.MaterialTheme
import com.tencent.kuikly.compose.material3.RadioButtonColors
import com.tencent.kuikly.compose.material3.RadioButtonDefaults
import com.tencent.kuikly.compose.material3.SegmentedButtonColors
import com.tencent.kuikly.compose.material3.SegmentedButtonDefaults
import com.tencent.kuikly.compose.material3.SliderColors
import com.tencent.kuikly.compose.material3.SliderDefaults
import com.tencent.kuikly.compose.material3.SwitchColors
import com.tencent.kuikly.compose.material3.SwitchDefaults
import com.tencent.kuikly.compose.ui.graphics.Color as ComposeColor
import com.tencent.kuikly.compose.ui.graphics.Shape
import com.tencent.kuikly.compose.ui.text.TextStyle
import com.tencent.kuikly.compose.ui.text.font.FontWeight
import com.tencent.kuikly.compose.ui.unit.dp
import com.tencent.kuikly.compose.ui.unit.sp
import com.xiaozhi.admin.ui.AdminColors
import com.xiaozhi.admin.ui.AdminShape
import com.xiaozhi.admin.ui.AdminToken
import com.xiaozhi.admin.ui.AdminType

/**
 * 管理台 · Compose 侧主题桥。
 *
 * ## 为什么需要这一层
 * Compose-Settings 的磁贴全部走 Material 3 默认配色与字号，而管理台是**暗色原生风**
 * （[AdminColors] 一套 token，从 `#1C1C1E` 窗口底到 `#F5F5F7` 主文字）。两者直接拼在一起，
 * 磁贴会是浅色卡片 + 紫色主色，与周围 DSL 内容完全割裂。
 *
 * Kuikly 的 `MaterialTheme` 虽然签名上接受 `colorScheme`，但实测不参与各组件默认值计算
 * （组件默认色在构造 `XxxDefaults.colors()` 时就固化了），所以**颜色只能逐组件显式传**
 * （`colors = adminListItemColors()` 这类）。字号同理：上游 `SettingsXxx` 没有 `textStyles`
 * 参数，只能通过 [LocalListItemHeadlineStyle]/[LocalListItemSupportingStyle] 注入
 * （见 `sharedCompose` 的 `ListItem` 补丁）。
 *
 * ## 唯一真源
 * 颜色一律从 [AdminColors] **转换**得到（`toComposeColor()`），不在本文件复制 hex 字面量；
 * 字号一律从 [AdminType] 取。改 token 只改 `ui/Theme.kt` 一处。
 */

/** 管理台 token 的 Compose 侧投影（值全部来自 [AdminColors]）。 */
object AdminC {

    /** 同一份 ARGB Long 同时喂给 Compose Color 与 DSL 的 Kuikly Color。 */
    private fun c(argb: Long): ComposeColor = ComposeColor(argb)

    val windowBg = c(AdminToken.WINDOW_BG)
    val sidebarBg = c(AdminToken.SIDEBAR_BG)
    val sidebarSelectedBg = c(AdminToken.SIDEBAR_SELECTED_BG)
    val cardBg = c(AdminToken.CARD_BG)
    val cardHover = c(AdminToken.CARD_HOVER)
    val fieldBg = c(AdminToken.FIELD_BG)
    val insetBg = c(AdminToken.INSET_BG)

    val textPrimary = c(AdminToken.TEXT_PRIMARY)
    val textSecondary = c(AdminToken.TEXT_SECONDARY)
    val textTertiary = c(AdminToken.TEXT_TERTIARY)
    val textPlaceholder = c(AdminToken.TEXT_PLACEHOLDER)
    val textOnAccent = c(AdminToken.TEXT_ON_ACCENT)
    val textAccent = c(AdminToken.TEXT_ACCENT)

    val divider = c(AdminToken.DIVIDER)
    val dividerStrong = c(AdminToken.DIVIDER_STRONG)

    val accent = c(AdminToken.ACCENT)
    val accentTintBg = c(AdminToken.ACCENT_TINT_BG)
    val accentTintText = c(AdminToken.ACCENT_TINT_TEXT)

    val danger = c(AdminToken.DANGER)
    val dangerTintText = c(AdminToken.DANGER_TINT_TEXT)
    val warning = c(AdminToken.WARNING)
    val warningTintText = c(AdminToken.WARNING_TINT_TEXT)

    val switchOff = c(AdminToken.SWITCH_OFF)
    val knob = c(AdminToken.KNOB)
    val trackBg = c(AdminToken.TRACK_BG)

    val disabledBg = c(AdminToken.DISABLED_BG)
    val disabledText = c(AdminToken.DISABLED_TEXT)

    val transparent = c(AdminToken.TRANSPARENT)
}

/**
 * 磁贴标题字号（[AdminType.body]，与 DSL 侧 `switchRow` 的标签同档）。
 * 不带 color：颜色由 `ListItemColors.headlineColor` 经 `LocalContentColor` 下发。
 */
@Composable
fun adminTileTitleStyle(): TextStyle = TextStyle(
    fontSize = AdminType.body.sp,
    fontWeight = FontWeight.Medium,
)

/** 磁贴副标题字号（[AdminType.caption]）。 */
@Composable
fun adminTileSubtitleStyle(): TextStyle = TextStyle(
    fontSize = AdminType.caption.sp,
    fontWeight = FontWeight.Normal,
)

/**
 * 页面根包裹：注入磁贴字号，并提供 `MaterialTheme` 默认排版兜底。
 * 用法：`AdminComposeTheme { SettingsGroup(...) { ... } }`。
 */
@Composable
fun AdminComposeTheme(content: @Composable () -> Unit) {
    val title = adminTileTitleStyle()
    val subtitle = adminTileSubtitleStyle()
    MaterialTheme {
        CompositionLocalProvider(
            LocalListItemHeadlineStyle provides title,
            LocalListItemSupportingStyle provides subtitle,
            content = content,
        )
    }
}

/**
 * 磁贴圆角：与 DSL 侧 [com.xiaozhi.admin.ui.cardSurface] 的 `radiusLg`(12) 一致。
 *
 * 上游默认 `ListItemDefaults.shape` 是直角；直接沿用会让 Compose 页的卡片与旧页面的
 * 圆角卡片并排时明显割裂。所有 `SettingsXxx` 显式传本 shape。
 */
@Composable
fun adminTileShape(): Shape = RoundedCornerShape(AdminShape.radiusLg.dp)

/** 磁贴配色：分组卡片底 + 主/次文字，与 `cardBg` 上的 DSL 内容同色系。 */
@Composable
fun adminListItemColors(
    containerColor: ComposeColor = AdminC.cardBg,
    headlineColor: ComposeColor = AdminC.textPrimary,
    supportingColor: ComposeColor = AdminC.textSecondary,
): ListItemColors = ListItemDefaults.colors(
    containerColor = containerColor,
    headlineColor = headlineColor,
    supportingColor = supportingColor,
    leadingIconColor = AdminC.textSecondary,
    trailingIconColor = AdminC.textSecondary,
    overlineColor = AdminC.textTertiary,
    disabledHeadlineColor = AdminC.disabledText,
    disabledSupportingColor = AdminC.textTertiary,
    disabledLeadingIconColor = AdminC.textPlaceholder,
    disabledTrailingIconColor = AdminC.textPlaceholder,
    disabledOverlineColor = AdminC.textPlaceholder,
)

/** 开关配色：开=accent、关=switchOff、滑块=knob（与 DSL 侧 `switchRow` 一致）。 */
@Composable
fun adminSwitchColors(): SwitchColors = SwitchDefaults.colors(
    checkedThumbColor = AdminC.knob,
    checkedTrackColor = AdminC.accent,
    checkedBorderColor = AdminC.transparent,
    uncheckedThumbColor = AdminC.knob,
    uncheckedTrackColor = AdminC.switchOff,
    uncheckedBorderColor = AdminC.transparent,
    disabledCheckedThumbColor = AdminC.knob,
    disabledCheckedTrackColor = AdminC.disabledBg,
    disabledUncheckedThumbColor = AdminC.knob,
    disabledUncheckedTrackColor = AdminC.trackBg,
)

/** 复选配色：勾选块=accent、对勾=白、未选=fieldBg。 */
@Composable
fun adminCheckboxColors(): CheckboxColors = CheckboxDefaults.colors(
    checkedColor = AdminC.accent,
    uncheckedColor = AdminC.dividerStrong,
    checkmarkColor = AdminC.textOnAccent,
    disabledCheckedColor = AdminC.disabledBg,
    disabledUncheckedColor = AdminC.textPlaceholder,
    disabledIndeterminateColor = AdminC.disabledBg,
)

/** 单选配色（本项目补丁组件）：选中=accent，未选中=次级文字色。 */
@Composable
fun adminRadioButtonColors(): RadioButtonColors = RadioButtonDefaults.colors(
    selectedColor = AdminC.accent,
    unselectedColor = AdminC.dividerStrong,
    disabledSelectedColor = AdminC.disabledBg,
    disabledUnselectedColor = AdminC.textPlaceholder,
)

/** 滑块配色：轨道用 trackBg，已选段/滑块用 accent。 */
@Composable
fun adminSliderColors(): SliderColors = SliderDefaults.colors(
    thumbColor = AdminC.accent,
    activeTrackColor = AdminC.accent,
    inactiveTrackColor = AdminC.trackBg,
    activeTickColor = AdminC.transparent,
    inactiveTickColor = AdminC.transparent,
    disabledThumbColor = AdminC.disabledBg,
    disabledActiveTrackColor = AdminC.disabledBg,
)

/** 分段按钮配色：选中=accent 描边 + 淡绿底，未选中=fieldBg 底 + divider 描边。 */
@Composable
fun adminSegmentedButtonColors(): SegmentedButtonColors = SegmentedButtonDefaults.colors(
    activeContainerColor = AdminC.accentTintBg,
    activeContentColor = AdminC.accentTintText,
    activeBorderColor = AdminC.accent,
    inactiveContainerColor = AdminC.fieldBg,
    inactiveContentColor = AdminC.textSecondary,
    inactiveBorderColor = AdminC.divider,
    disabledActiveContainerColor = AdminC.insetBg,
    disabledActiveContentColor = AdminC.textPlaceholder,
    disabledActiveBorderColor = AdminC.divider,
    disabledInactiveContainerColor = AdminC.fieldBg,
    disabledInactiveContentColor = AdminC.textPlaceholder,
    disabledInactiveBorderColor = AdminC.divider,
)
