package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.Canvas
import com.tencent.kuikly.compose.foundation.layout.size
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.geometry.CornerRadius
import com.tencent.kuikly.compose.ui.geometry.Offset
import com.tencent.kuikly.compose.ui.geometry.Size
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.graphics.drawscope.rotate
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp

/**
 * Echo Dot 2 顶部光环（官方 `dashboard.jsx` 的 `LedRing` 组件）—— Compose `Canvas` 自绘版。
 *
 * ## 几何与配色语义**一字不改**（K7）
 *
 * 官方是 SVG：12 段等距圆弧（`circumference / 12`，段占 72%、缝占 28%）套在径向渐变外壳里，
 * 中央一个状态点。Kuikly 没有 SVG、没有径向渐变，旧版用「近黑同心圆 + 绝对定位小圆条」表达；
 * 新版直接 Canvas 绘制，**比例常量完全相同**（见下方 4 个 `led*` 公式函数，绘图代码只调用它们，
 * 不重复写数字）：
 *
 * | 量 | 旧值 = 新值 | 公式函数 |
 * |---|---|---|
 * | 段数 | 12 | [LED_SEGMENTS] |
 * | 段所在半径 | `size * 0.38` | [ledSegmentCenterRadius] |
 * | 段宽 | `max(size * 0.03, 2)` | [ledSegmentWidth] |
 * | 段高 | `max(size * 0.075, 4)` | [ledSegmentHeight] |
 * | 中央点直径 | `size * 0.12` | [ledDotDiameter] |
 * | 外壳 | `#262626` 圆 + `#303030` 内圆（`0.94 size`） | [LedShellOuter] / [LedShellInner] |
 * | 起始角 | 12 点方向顺时针（官方 `rotate(-90)`） | 绘制期 `rotate` |
 *
 * ## 颜色语义（由调用方按设备状态决定，本组件不猜）
 *
 * `segColor`：绿=监听 / 蓝=说话 / 黄=思考 / 红=静音 / 橙=离线。
 * `dotColor`：中央状态点（官方 `state.dot`）。
 *
 * ⚠️ 段的位置在**绘制期**算（`DrawScope` 内），尺寸变化自然重算。
 */
internal const val LED_SEGMENTS = 12

/** 外壳外层圆（官方径向渐变 `#505050→#2c2c2c→#161616` 的跨端降级表达，与旧实现同值）。 */
private val LedShellOuter = Color(0xFF262626)

/** 外壳内层圆（比外层亮一档，制造"金属外壳"层次）。 */
private val LedShellInner = Color(0xFF303030)

/** 12 段所在圆的半径。 */
internal fun ledSegmentCenterRadius(size: Float): Float = size * 0.38f

/** 单段宽度（下限 2px，避免小尺寸下段消失）。 */
internal fun ledSegmentWidth(size: Float): Float = (size * 0.03f).coerceAtLeast(2f)

/** 单段高度（下限 4px）。 */
internal fun ledSegmentHeight(size: Float): Float = (size * 0.075f).coerceAtLeast(4f)

/** 中央状态点直径。 */
internal fun ledDotDiameter(size: Float): Float = size * 0.12f

/**
 * 画一个 Dot 2 光环。
 *
 * @param size     边长（光环外径）；调用方按布局给
 * @param segColor 12 段的颜色
 * @param dotColor 中央状态点颜色
 */
@Composable
fun GearLedRing(
    size: Dp,
    segColor: Color,
    dotColor: Color,
    modifier: Modifier = Modifier,
) {
    Canvas(modifier.size(size)) {
        val full = this.size.width
        val center = Offset(full / 2f, full / 2f)

        // 外壳：深灰圆 + 一档更亮的内圆（两层同心圆表达层次，避免跨端渐变差异）
        drawCircle(color = LedShellOuter, radius = full / 2f, center = center)
        drawCircle(color = LedShellInner, radius = full * 0.47f, center = center)

        // 12 段：从 12 点方向起顺时针均分
        val radius = ledSegmentCenterRadius(full)
        val segWidth = ledSegmentWidth(full)
        val segHeight = ledSegmentHeight(full)
        val segCorner = CornerRadius(segWidth / 2f, segWidth / 2f)
        for (index in 0 until LED_SEGMENTS) {
            rotate(degrees = 360f / LED_SEGMENTS * index, pivot = center) {
                drawRoundRect(
                    color = segColor,
                    topLeft = Offset(
                        x = center.x - segWidth / 2f,
                        y = center.y - radius - segHeight / 2f,
                    ),
                    size = Size(segWidth, segHeight),
                    cornerRadius = segCorner,
                )
            }
        }

        // 中央状态点
        drawCircle(color = dotColor, radius = ledDotDiameter(full) / 2f, center = center)
    }
}
