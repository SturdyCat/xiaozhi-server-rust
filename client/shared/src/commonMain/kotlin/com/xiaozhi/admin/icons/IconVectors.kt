package com.xiaozhi.admin.icons

import com.tencent.kuikly.compose.ui.graphics.Path

/**
 * 图标矢量几何 —— 一个图层 = 一段 SVG `pathData` + 绘制模式（stroke/fill），内嵌进 commonMain 当数据。
 *
 * K1（否定式约束 · 为什么是 pathData 而不是资源文件，方法对齐 xiaoya-player 的 TvSvg/TvIcons）：
 * - **不放 PNG / 不放 SVG 文件**。位图放大即糊；SVG 文件则要求渲染端自带 SVG 解码能力——
 *   本仓库曾经为 19 个图标养了三端实现（mac 原生 XzSvgView/XzSvgDocument/XzSvgPath +
 *   Web 内联 DOM WebSvgView/BundledSvgSources + 资源打包链），macApp 一次资源没进包就全空白。
 *   这里只把图标的**几何数据**（`d` 属性）内嵌成常量，由 [SvgPathParser] 解析后用 Kuikly `Canvas`
 *   即时绘制：三端（Web/macOS/Android/iOS/OHOS）共用同一份代码与同一份数据，
 *   不依赖任何一端的图片解码、资源打包或宿主注册，**任意尺寸都清晰**，颜色随 `tint` 参数走。
 * - 坐标系：viewBox 恒为 `0 0 24 24`（y 轴向下、原点左上，自研图标沿用原 SVG 资产的坐标系），
 *   无 Material Symbols `0 -960 960 960` 的负 y 原点问题，无需原点平移。
 *
 * K2（已知边界 · lineJoin）: 旧 Kuikly `CanvasContext` 未暴露 lineJoin，描边拐角按各端默认 miter
 * 呈现；图标线宽仅 1.2–1.5px（1.8 × size/24），miter 与 round 的视觉差异不可辨，各端默认一致，
 * 不做端内补偿。lineCap 显式取 round（与原 SVG 一致）。
 *
 * T04 变更（唯一改动）：绘制目标从 Kuikly DSL 的 `CanvasContext` 换成 Compose 的 [Path]
 * （`com.tencent.kuikly.compose.ui.graphics.Path`），因为图标渲染层已改由 Compose `Canvas`
 * 承载。**`SvgPathParser` 与 26 组 pathData 逐字节未动**（K1 几何不变），
 * 只有 [IconOp.emit] 的形参类型与其内部四次曲线调用的方法名映射：
 * `moveTo/lineTo` 同名；`bezierCurveTo` → `cubicTo`；`quadraticCurveTo` → `quadraticTo`；
 * `closePath` → `close`。语义一一对应，坐标与缩放系数完全一致。
 *
 * @param pathData    SVG `d` 属性原文（可含多条子路径与 `Z`；不支持的写法会在解析时被安全跳过）
 * @param filled      true = 以 tint 填充（fill）；false = 以 tint 描边（stroke，线宽 [strokeWidth]）
 * @param strokeWidth 描边宽度（viewBox 单位；绘制时随 size/24 缩放，原 SVG 资产恒为 1.8）
 */
internal class IconLayer internal constructor(
    internal val pathData: String,
    internal val filled: Boolean,
    internal val strokeWidth: Float = STROKE_WIDTH,
) {
    /** 解析结果按图层惰性缓存一次：图层是 object 单例字段，进程内每层至多解析一回 */
    internal val ops: List<IconOp> by lazy { SvgPathParser.parse(pathData) }

    internal companion object {
        /** 原 SVG 资产的统一描边宽度（viewBox 单位） */
        internal const val STROKE_WIDTH = 1.8f
    }
}

/** 解析产物：一条归一化的绘制指令，坐标为 viewBox 单位，绘制时统一乘缩放系数 */
internal sealed class IconOp {
    internal abstract fun emit(path: Path, scale: Float)

    internal class Move(internal val x: Float, internal val y: Float) : IconOp() {
        override fun emit(path: Path, scale: Float) = path.moveTo(x * scale, y * scale)
    }

    internal class Line(internal val x: Float, internal val y: Float) : IconOp() {
        override fun emit(path: Path, scale: Float) = path.lineTo(x * scale, y * scale)
    }

    internal class Cubic(
        internal val x1: Float, internal val y1: Float,
        internal val x2: Float, internal val y2: Float,
        internal val x: Float, internal val y: Float,
    ) : IconOp() {
        override fun emit(path: Path, scale: Float) =
            path.cubicTo(x1 * scale, y1 * scale, x2 * scale, y2 * scale, x * scale, y * scale)
    }

    internal class Quad(
        internal val x1: Float, internal val y1: Float,
        internal val x: Float, internal val y: Float,
    ) : IconOp() {
        override fun emit(path: Path, scale: Float) =
            // Kuikly Compose 的 Path 提供的是 Compose 口径的 quadraticBezierTo，
            // 没有 Skia 风格的 quadraticTo（实测 2.30.0 klib 里只有 quadraticBezierTo）。
            path.quadraticBezierTo(x1 * scale, y1 * scale, x * scale, y * scale)
    }

    internal object Close : IconOp() {
        override fun emit(path: Path, scale: Float) = path.close()
    }
}

/**
 * SVG `pathData` → [IconOp] 列表的最小解析器（只做图标需要的事，不追求完整 SVG 实现）。
 *
 * 支持的绘制命令（覆盖本图标集与 Material Symbols 全量用到的集合）：
 * `M/m` `L/l` `H/h` `V/v` `C/c` `S/s` `Q/q` `T/t` `A/a` `Z/z`，
 * 含「命令后省略字母的隐式重复」与 `S`/`T` 的**控制点反射**语义；`A/a` 椭圆弧按 SVG 规范
 * F.6.5 端点参数化转三次贝塞尔（单段弧 ≤90° 切分，`4/3·tan(Δθ/4)` 近似）。
 *
 * K3（健壮性）：未知命令吞一个 token 保证循环必然推进；`Z` 的多余数值直接丢弃，
 * 畸形数据下不会死循环、不会抛异常——图标坏了最多不显示，不能崩页面。
 */
internal object SvgPathParser {

    /**
     * 词法：命令字母或数值。`e/E` 刻意**不在**命令集合内——指数的 `e` 才能与数字连成
     * 同一个 token（`1e-5` 不会被切成 `1`、`e`、`-5`）。
     */
    private val TOKEN = Regex("[MmLlHhVvCcSsQqTtAaZz]|[-+]?(?:\\d*\\.\\d+|\\d+\\.?)(?:[eE][-+]?\\d+)?")

    fun parse(pathData: String): List<IconOp> {
        val ops = ArrayList<IconOp>()
        val tokens = TOKEN.findAll(pathData).map { it.value }.toList()
        var i = 0
        var cmd = ' '
        var prev = ' '
        var cx = 0f
        var cy = 0f
        var startX = 0f
        var startY = 0f
        // S/s 的反射基准：上一个三次曲线的第二控制点；T/t 的反射基准：上一个二次曲线的控制点
        var ctrl2X = 0f
        var ctrl2Y = 0f
        var quadX = 0f
        var quadY = 0f

        fun next(): Float = if (i < tokens.size) tokens[i++].toFloat() else 0f

        fun line(x: Float, y: Float) = ops.add(IconOp.Line(x, y))

        fun cubic(x1: Float, y1: Float, x2: Float, y2: Float, x: Float, y: Float) =
            ops.add(IconOp.Cubic(x1, y1, x2, y2, x, y))

        fun quad(x1: Float, y1: Float, x: Float, y: Float) = ops.add(IconOp.Quad(x1, y1, x, y))

        /** 椭圆弧 → 三次贝塞尔（SVG 规范 F.6.5 端点参数化 → 中心参数化） */
        fun arc(rxIn: Float, ryIn: Float, rotDeg: Float, largeArc: Boolean, sweep: Boolean, x: Float, y: Float) {
            if (cx == x && cy == y) return
            var rx = kotlin.math.abs(rxIn)
            var ry = kotlin.math.abs(ryIn)
            if (rx == 0f || ry == 0f) {
                line(x, y)
                return
            }
            val phi = rotDeg * (kotlin.math.PI.toFloat() / 180f)
            val cosP = kotlin.math.cos(phi)
            val sinP = kotlin.math.sin(phi)
            val dx2 = (cx - x) / 2f
            val dy2 = (cy - y) / 2f
            val x1p = cosP * dx2 + sinP * dy2
            val y1p = -sinP * dx2 + cosP * dy2
            // 半径不足时按规范等比放大，保证弧存在
            val lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry)
            if (lambda > 1f) {
                val k = kotlin.math.sqrt(lambda)
                rx *= k
                ry *= k
            }
            val rx2 = rx * rx
            val ry2 = ry * ry
            val x1p2 = x1p * x1p
            val y1p2 = y1p * y1p
            val den = rx2 * y1p2 + ry2 * x1p2
            val numRaw = rx2 * ry2 - rx2 * y1p2 - ry2 * x1p2
            val num = if (numRaw < 0f) 0f else numRaw
            val co = (if (largeArc != sweep) 1f else -1f) * kotlin.math.sqrt(if (den == 0f) 0f else num / den)
            val cxp = co * rx * y1p / ry
            val cyp = -co * ry * x1p / rx
            val centerX = cosP * cxp - sinP * cyp + (cx + x) / 2f
            val centerY = sinP * cxp + cosP * cyp + (cy + y) / 2f
            val ux = (x1p - cxp) / rx
            val uy = (y1p - cyp) / ry
            val vx = (-x1p - cxp) / rx
            val vy = (-y1p - cyp) / ry
            val theta1 = kotlin.math.atan2(uy, ux)
            var delta = kotlin.math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)
            val twoPi = kotlin.math.PI.toFloat() * 2f
            if (!sweep && delta > 0f) delta -= twoPi
            if (sweep && delta < 0f) delta += twoPi
            val segs = kotlin.math.ceil(kotlin.math.abs(delta) / (kotlin.math.PI.toFloat() / 2f)).toInt().coerceAtLeast(1)
            val step = delta / segs
            val alpha = 4f / 3f * kotlin.math.tan(step / 4f)
            var theta = theta1
            repeat(segs) {
                val theta2 = theta + step
                val p1x = centerX + rx * cosP * kotlin.math.cos(theta) - ry * sinP * kotlin.math.sin(theta)
                val p1y = centerY + rx * sinP * kotlin.math.cos(theta) + ry * cosP * kotlin.math.sin(theta)
                val p2x = centerX + rx * cosP * kotlin.math.cos(theta2) - ry * sinP * kotlin.math.sin(theta2)
                val p2y = centerY + rx * sinP * kotlin.math.cos(theta2) + ry * cosP * kotlin.math.sin(theta2)
                val d1x = -rx * cosP * kotlin.math.sin(theta) - ry * sinP * kotlin.math.cos(theta)
                val d1y = -rx * sinP * kotlin.math.sin(theta) + ry * cosP * kotlin.math.cos(theta)
                val d2x = -rx * cosP * kotlin.math.sin(theta2) - ry * sinP * kotlin.math.cos(theta2)
                val d2y = -rx * sinP * kotlin.math.sin(theta2) + ry * cosP * kotlin.math.cos(theta2)
                cubic(
                    p1x + alpha * d1x, p1y + alpha * d1y,
                    p2x - alpha * d2x, p2y - alpha * d2y,
                    p2x, p2y,
                )
                theta = theta2
            }
        }

        val reflectCubic = { prev == 'C' || prev == 'c' || prev == 'S' || prev == 's' }
        val reflectQuad = { prev == 'Q' || prev == 'q' || prev == 'T' || prev == 't' }

        while (i < tokens.size) {
            val tk = tokens[i]
            if (tk[0].isLetter()) {
                cmd = tk[0]
                i++
            } else {
                when (cmd) {
                    // 隐式重复：moveto 后紧跟的坐标对按 lineto 处理（SVG 规范），否则同命令再来一段
                    'M' -> cmd = 'L'
                    'm' -> cmd = 'l'
                    // Z 不接收参数：多余数值直接丢弃，避免死循环
                    'Z', 'z' -> {
                        i++
                        continue
                    }
                }
            }
            when (cmd) {
                'M' -> {
                    cx = next(); cy = next()
                    ops.add(IconOp.Move(cx, cy))
                    startX = cx; startY = cy
                }

                'm' -> {
                    cx += next(); cy += next()
                    ops.add(IconOp.Move(cx, cy))
                    startX = cx; startY = cy
                }

                'L' -> {
                    cx = next(); cy = next(); line(cx, cy)
                }

                'l' -> {
                    cx += next(); cy += next(); line(cx, cy)
                }

                'H' -> {
                    cx = next(); line(cx, cy)
                }

                'h' -> {
                    cx += next(); line(cx, cy)
                }

                'V' -> {
                    cy = next(); line(cx, cy)
                }

                'v' -> {
                    cy += next(); line(cx, cy)
                }

                'C' -> {
                    val x1 = next(); val y1 = next()
                    val x2 = next(); val y2 = next()
                    val x = next(); val y = next()
                    cubic(x1, y1, x2, y2, x, y)
                    ctrl2X = x2; ctrl2Y = y2
                    cx = x; cy = y
                }

                'c' -> {
                    val x1 = cx + next(); val y1 = cy + next()
                    val x2 = cx + next(); val y2 = cy + next()
                    val x = cx + next(); val y = cy + next()
                    cubic(x1, y1, x2, y2, x, y)
                    ctrl2X = x2; ctrl2Y = y2
                    cx = x; cy = y
                }

                'S' -> {
                    val x2 = next(); val y2 = next()
                    val x = next(); val y = next()
                    val x1 = if (reflectCubic()) 2f * cx - ctrl2X else cx
                    val y1 = if (reflectCubic()) 2f * cy - ctrl2Y else cy
                    cubic(x1, y1, x2, y2, x, y)
                    ctrl2X = x2; ctrl2Y = y2
                    cx = x; cy = y
                }

                's' -> {
                    val x2 = cx + next(); val y2 = cy + next()
                    val x = cx + next(); val y = cy + next()
                    val x1 = if (reflectCubic()) 2f * cx - ctrl2X else cx
                    val y1 = if (reflectCubic()) 2f * cy - ctrl2Y else cy
                    cubic(x1, y1, x2, y2, x, y)
                    ctrl2X = x2; ctrl2Y = y2
                    cx = x; cy = y
                }

                'Q' -> {
                    val x1 = next(); val y1 = next()
                    val x = next(); val y = next()
                    quad(x1, y1, x, y)
                    quadX = x1; quadY = y1
                    cx = x; cy = y
                }

                'q' -> {
                    val x1 = cx + next(); val y1 = cy + next()
                    val x = cx + next(); val y = cy + next()
                    quad(x1, y1, x, y)
                    quadX = x1; quadY = y1
                    cx = x; cy = y
                }

                'T' -> {
                    val x = next(); val y = next()
                    val x1 = if (reflectQuad()) 2f * cx - quadX else cx
                    val y1 = if (reflectQuad()) 2f * cy - quadY else cy
                    quad(x1, y1, x, y)
                    quadX = x1; quadY = y1
                    cx = x; cy = y
                }

                't' -> {
                    val x = cx + next(); val y = cy + next()
                    val x1 = if (reflectQuad()) 2f * cx - quadX else cx
                    val y1 = if (reflectQuad()) 2f * cy - quadY else cy
                    quad(x1, y1, x, y)
                    quadX = x1; quadY = y1
                    cx = x; cy = y
                }

                'A' -> {
                    val rx = next(); val ry = next(); val rot = next()
                    val largeArc = next() != 0f; val sweep = next() != 0f
                    val x = next(); val y = next()
                    arc(rx, ry, rot, largeArc, sweep, x, y)
                    cx = x; cy = y
                }

                'a' -> {
                    val rx = next(); val ry = next(); val rot = next()
                    val largeArc = next() != 0f; val sweep = next() != 0f
                    val x = cx + next(); val y = cy + next()
                    arc(rx, ry, rot, largeArc, sweep, x, y)
                    cx = x; cy = y
                }

                'Z', 'z' -> {
                    ops.add(IconOp.Close)
                    cx = startX; cy = startY
                }

                // 未知命令：吞掉一个 token，保证循环必然推进（避免畸形数据下死循环）
                else -> i++
            }
            prev = cmd
        }
        return ops
    }
}
