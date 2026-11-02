package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import com.gearui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp
import com.xiaozhi.admin.IS_MACOS

/**
 * admin 间距唯一真源。
 *
 * 值随 [provideAdminSpacing] 注入的 scale 同倍缩放，与字号 fontScale **双轴同源**
 * （见 [adminScale]）。字段命名与旧 [GearUiSpace] 对齐，迁移零侵入；数值与迁移前逐像素一致
 * （scale=1 时即旧值），守 Q9 像素守恒。
 *
 * 注意：间距字段以 dp 存储、按 scale 缩放（对比字号用 sp）。这与字号「行高随字号同倍以 sp 存储」
 * 是两套独立的缩放轴，互不打扰。
 */
data class SpacingScale(val scale: Float = 1f) {
    val xs: Dp get() = (8f * scale).dp
    val sm: Dp get() = (12f * scale).dp
    val md: Dp get() = (16f * scale).dp
    val lg: Dp get() = (20f * scale).dp
    val xl: Dp get() = (24f * scale).dp
    val xxl: Dp get() = (28f * scale).dp
    val xxxl: Dp get() = (32f * scale).dp
    val pagePadding: Dp get() = (24f * scale).dp
    val gutter: Dp get() = (20f * scale).dp
    val cardPadding: Dp get() = (20f * scale).dp
    val fieldGap: Dp get() = (12f * scale).dp
}

/** admin 间距 CompositionLocal（默认 scale=1，与旧 [GearUiSpace] 数值一致）。 */
val LocalAdminSpacing = compositionLocalOf { SpacingScale(1f) }

/** 在子树内注入 [scale] 倍的 admin 间距。 */
@Composable
fun provideAdminSpacing(scale: Float, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalAdminSpacing provides SpacingScale(scale)) {
        content()
    }
}

/**
 * 间距缩放系数：
 * - macOS：固定 1.2（两轴同源）；
 * - 其余（web / iOS / Android）：按视口宽度分级
 *   `<900 → 1.0` / `≥900 → 1.0625` / `≥1200 → 1.125` / `≥1440 → 1.1875`。
 *
 * iOS 视口通常 `<900`，故实际落在 `1.0`（与「iOS=1.0」一致）；若需严格排除 iPad 等宽屏，
 * 可引入 `IS_WEB` 判别（见 [adminScale] 说明）。
 */
fun adminSpacingScale(isMac: Boolean, widthDp: Int): Float = when {
    isMac -> 1.2f
    widthDp >= 1440 -> 1.1875f
    widthDp >= 1200 -> 1.125f
    widthDp >= 900 -> 1.0625f
    else -> 1.0f
}

/**
 * 双轴同源：字号与间距采用同一缩放系数（Mac 两轴都 1.2，web 按视口同档）。
 * 返回 `(fontScale, spacingScale)`。
 */
fun adminScale(isMac: Boolean, widthDp: Int): Pair<Float, Float> {
    val f = adminSpacingScale(isMac, widthDp)
    return f to f
}
