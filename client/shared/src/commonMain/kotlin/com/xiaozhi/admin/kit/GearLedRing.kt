package com.xiaozhi.admin.kit

import androidx.compose.runtime.Composable
import com.tencent.kuikly.compose.foundation.Canvas
import com.tencent.kuikly.compose.foundation.layout.size
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.geometry.Offset
import com.tencent.kuikly.compose.ui.geometry.Size
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.graphics.drawscope.Stroke
import com.tencent.kuikly.compose.ui.unit.Dp

/** 官方 LedRing 的 Dot 2 设备插画；Canvas 原生绘制，不是文字头像或图标位图。
 * 几何与 dashboard.jsx 对齐：外壳 .49/.47、灯环 .38、12 段各占 72%、线宽 .045、中心点 .09。
 * 径向渐变降级为深灰同心层；外壳颜色仍在本零件内收口，状态色由 GearUI 语义 token 传入。
 */
internal const val LED_SEGMENTS = 12
private val LedShellOuter = Color(0xFF262626)
private val LedShellInner = Color(0xFF303030)
internal fun ledSegmentCenterRadius(size: Float): Float = size * 0.38f
internal fun ledDotDiameter(size: Float): Float = size * 0.18f

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
        drawCircle(color = LedShellOuter, radius = full * 0.49f, center = center)
        drawCircle(color = LedShellInner, radius = full * 0.47f, center = center)
        val radius = ledSegmentCenterRadius(full)
        for (index in 0 until LED_SEGMENTS) {
            drawArc(color = segColor, startAngle = -90f + index * (360f / LED_SEGMENTS),
                sweepAngle = 360f / LED_SEGMENTS * 0.72f, useCenter = false,
                topLeft = Offset(center.x - radius, center.y - radius), size = Size(radius * 2, radius * 2),
                style = Stroke(width = full * 0.045f))
        }
        drawCircle(color = LedShellOuter, radius = full * 0.36f, center = center)
        drawCircle(color = dotColor, radius = ledDotDiameter(full) / 2f, center = center)
    }
}
