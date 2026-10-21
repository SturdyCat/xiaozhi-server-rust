package com.xiaozhi.admin

import com.tencent.kuikly.core.base.Attr
import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.views.Canvas

/**
 * 管理台图标几何 —— [IconToken] → 内嵌 pathData（方法对齐 xiaoya-player 的 TvIcons，2026-10 从
 * assets/common/icons/source 的 19 个 .svg 资产迁入代码；原资产三端各养一套宿主实现，macApp 资源没进包即全空白）。
 *
 * K1（唯一约束）：全部图标沿用原 SVG 资产的 viewBox `0 0 24 24` 与描边宽度 1.8（round cap），
 * 仅 settings 含三个 fill 圆角旋钮；绘制经 [appIcon] 用 Kuikly `Canvas` 即时完成，
 * 不经 Image/UIImage、不经任何端资源打包。新增/修改图标直接改这里的 pathData
 * （`M/L/H/V/C/S/Q/T/A/Z` 全支持，见 [SvgPathParser]）；许可 CC0-1.0（原 LICENSE.txt 同文，作者本项目 2026）。
 */
internal object AdminIconVectors {

    /** 描边图层（线宽 1.8、round cap，绘制时随尺寸缩放） */
    private fun stroke(pathData: String) = IconLayer(pathData, filled = false)

    /** 填充图层（settings 旋钮等实心部件） */
    private fun fill(pathData: String) = IconLayer(pathData, filled = true)

    internal val Save = listOf(stroke("M5 3.5h12l3 3v14H4v-17h1ZM8 3.5v6h8v-6M8 20.5v-7h8v7M13 5.5v2"))
    internal val Delete = listOf(stroke("M4 6.5h16M9 6.5v-3h6v3M6 6.5l1 14h10l1-14M10 10v7M14 10v7"))
    internal val Menu = listOf(stroke("M4 6h16M4 12h16M4 18h16"))
    internal val ChevronDown = listOf(stroke("m5 8 7 7 7-7"))
    internal val ChevronUp = listOf(stroke("m5 16 7-7 7 7"))
    internal val ChevronRight = listOf(stroke("m8 5 7 7-7 7"))
    internal val Play = listOf(stroke("m7 4 13 8-13 8Z"))
    internal val Stop = listOf(stroke("M6.5 5H17.5A1.5 1.5 0 0 1 19 6.5V17.5A1.5 1.5 0 0 1 17.5 19H6.5A1.5 1.5 0 0 1 5 17.5V6.5A1.5 1.5 0 0 1 6.5 5Z"))
    internal val Refresh = listOf(stroke("M19.5 8a8 8 0 1 0 .2 7M19.5 3.5V8H15"))
    internal val Check = listOf(stroke("m4 12 5 5L20 6"))
    internal val Close = listOf(stroke("m5 5 14 14M19 5 5 19"))

    /** 设置：三条滑轨（描边）+ 三个圆角旋钮（填充；原 SVG 为 fill=currentColor 的 rect rx=1） */
    internal val Settings = listOf(
        stroke("M4 6h16M4 12h16M4 18h16"),
        fill("M8 4H9A1 1 0 0 1 10 5V7A1 1 0 0 1 9 8H8A1 1 0 0 1 7 7V5A1 1 0 0 1 8 4Z"),
        fill("M15 10H16A1 1 0 0 1 17 11V13A1 1 0 0 1 16 14H15A1 1 0 0 1 14 13V11A1 1 0 0 1 15 10Z"),
        fill("M10 16H11A1 1 0 0 1 12 17V19A1 1 0 0 1 11 20H10A1 1 0 0 1 9 19V17A1 1 0 0 1 10 16Z"),
    )

    internal val Microphone = listOf(stroke("M9 6A3 3 0 0 1 15 6V12A3 3 0 0 1 9 12ZM6 11v1a6 6 0 0 0 12 0v-1M12 18v3M8 21h8"))
    internal val Waveform = listOf(stroke("M3 10v4M6.5 6v12M10 3v18M13.5 8v8M17 5v14M20.5 10v4"))
    internal val Text = listOf(stroke("M4 5V3.5h16V5M12 3.5v17M8 20.5h8"))

    /** 意图分流：三个节点圆 + 两条分连线（圆 = 两段半圆弧，坐标与原 SVG 一致） */
    internal val Intent = listOf(
        stroke("M4 12a2 2 0 1 0 4 0a2 2 0 1 0-4 0ZM16 5a2 2 0 1 0 4 0a2 2 0 1 0-4 0ZM16 19a2 2 0 1 0 4 0a2 2 0 1 0-4 0ZM8 12h3l5-7M11 12l5 7"),
    )

    internal val Model = listOf(stroke("m12 3 9 5-9 5-9-5 9-5ZM3 12l9 5 9-5M3 16l9 5 9-5"))
    internal val Speaker = listOf(stroke("M4 9h4l5-5v16l-5-5H4ZM16 8a6 6 0 0 1 0 8M19 5a10 10 0 0 1 0 14"))
    internal val Info = listOf(stroke("M3 12a9 9 0 1 0 18 0a9 9 0 1 0-18 0ZM12 11v6M12 7v.5"))
}

/** [IconToken] 的图层序列；与枚举的一一对应由 shared 构建护栏 `verifyAdminIcons` 锁定 */
internal fun IconToken.vectors(): List<IconLayer> = when (this) {
    IconToken.Save -> AdminIconVectors.Save
    IconToken.Delete -> AdminIconVectors.Delete
    IconToken.Menu -> AdminIconVectors.Menu
    IconToken.ChevronDown -> AdminIconVectors.ChevronDown
    IconToken.ChevronUp -> AdminIconVectors.ChevronUp
    IconToken.ChevronRight -> AdminIconVectors.ChevronRight
    IconToken.Play -> AdminIconVectors.Play
    IconToken.Stop -> AdminIconVectors.Stop
    IconToken.Refresh -> AdminIconVectors.Refresh
    IconToken.Check -> AdminIconVectors.Check
    IconToken.Close -> AdminIconVectors.Close
    IconToken.Settings -> AdminIconVectors.Settings
    IconToken.Microphone -> AdminIconVectors.Microphone
    IconToken.Waveform -> AdminIconVectors.Waveform
    IconToken.Text -> AdminIconVectors.Text
    IconToken.Intent -> AdminIconVectors.Intent
    IconToken.Model -> AdminIconVectors.Model
    IconToken.Speaker -> AdminIconVectors.Speaker
    IconToken.Info -> AdminIconVectors.Info
}

/**
 * 图标渲染 —— 内嵌矢量路径经 Canvas 即时绘制，三端表现一致（纯矢量、任意尺寸清晰、零资源依赖）。
 *
 * - 尺寸：画布即图标尺寸（`size × size`），viewBox 24 等比缩放落笔，无需容器再定宽高。
 * - 颜色：绘制时直接以 `tint` 落色；tint/token 闭包内的 observable 读取由 CanvasView 的
 *   响应式绑定跟踪，选中态变色会自动重绘（与原 SvgView 的 attr 跟踪同语义）。
 * - 不可交互：Canvas 仅是兄弟覆盖层，不参与点击/无障碍（同原 SvgView 契约）。
 *
 * @param token     图标（闭包，随响应式更新切换图形）
 * @param size      边长（逻辑像素；侧栏 18、按钮/菜单 16～20）
 * @param tint      着色（选中态 accent、禁用态 disabled 等，由调用方决定）
 * @param attrSetup 额外 attr（AdminButtons 的绝对定位/透明度等覆盖层定位用）
 */
fun ViewContainer<*, *>.appIcon(
    token: () -> IconToken,
    size: Float = 20f,
    tint: () -> Color = { AdminColors.textSecondary },
    attrSetup: Attr.() -> Unit = {},
) {
    Canvas({
        attr {
            size(size, size)
            touchEnable(false)
            attrSetup()
        }
    }) { context, width, _ ->
        val color = tint()
        val scale = width / 24f
        for (layer in token().vectors()) {
            context.beginPath()
            for (op in layer.ops) op.emit(context, scale)
            if (layer.filled) {
                context.fillStyle(color)
                context.fill()
            } else {
                context.strokeStyle(color)
                context.lineWidth(layer.strokeWidth * scale)
                context.lineCapRound()
                context.stroke()
            }
        }
    }
}
