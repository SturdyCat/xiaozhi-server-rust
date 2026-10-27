package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.views.View
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

/**
 * Echo Dot 2 顶部光环（官方 `dashboard.jsx` 的 `LedRing` 组件）。
 *
 * 官方是 SVG：12 段等距圆弧（`circumference / 12`，段占 72%、缝占 28%）套在
 * 径向渐变外壳里，中央一个状态点，`pending`/`offline` 时脉冲。Kuikly 没有 SVG、
 * 没有径向渐变，这里用**官方 flex + 绝对定位**把它画出来：
 *
 * ```
 * ┌ View(size × size) ──────────────┐
 * │  外壳：近黑圆 + 细描边            │ ← 替代 #505050→#2c2c2c→#161616 渐变
 * │  12 段：按 30° 均分，绝对定位小圆条 │ ← 段色随设备状态（listening/speaking/…）
 * │  中央：状态点                     │ ← 官方 state.dot
 * └──────────────────────────────────┘
 *
 * 为什么不用图片资产：① 段色要随状态变，一张图只能画一个状态；② Dot 2 的光环是
 * 「设备身份」而非装饰，尺寸要跟卡片缩放走；③ 仓库不打包位图资产。
 *
 * ⛔ 段的位置用三角函数在构建期算好、写进 `absolutePosition`，不要在 `attr` 里算：
 * `attr` 每次 observable 变化都会重跑，把几何算进去等于每帧重算 12 次坐标。
 */
private const val LED_SEGMENTS = 12

/**
 * 画一个 Dot 2 光环。
 *
 * @param size 边长（光环外径）；调用方按布局给，不在这里写死
 * @param segColor 12 段的颜色（官方由设备状态决定：绿=监听/蓝=说话/黄=思考/红=静音/橙=离线）
 * @param dotColor 中央状态点颜色
 */
fun ViewContainer<*, *>.ledRing(
    size: Float,
    segColor: Color,
    dotColor: Color,
) {
    val center = size / 2f
    // 官方 `r = size * 0.38`：段所在的圆半径
    val radius = size * 0.38f
    val segW = (size * 0.03f).coerceAtLeast(2f)
    val segH = (size * 0.075f).coerceAtLeast(4f)
    val dotSize = size * 0.12f

    View {
        attr {
            width(size)
            height(size)
        }
        // 外壳：官方渐变没法在 Kuikly 表达，用「深灰圆 + 一档更亮的描边」表达同一层次。
        // 不用 border() 是为了少一个跨端差异点：描边宽度在 iOS/H5 的取整不同，
        // 而这一圈只是给光环一个边界，用两层同心圆更稳。
        View {
            attr {
                width(size)
                height(size)
                borderRadius(center)
                backgroundColor(Color(0xFF262626))
            }
        }
        View {
            attr {
                width(size * 0.94f)
                height(size * 0.94f)
                borderRadius(size * 0.47f)
                backgroundColor(Color(0xFF303030))
            }
        }
        for (i in 0 until LED_SEGMENTS) {
            // 从 12 点方向起顺时针均分（官方 `rotate(-90)` 同义）
            val angle = 2.0 * PI * i / LED_SEGMENTS - PI / 2.0
            val left = center + (radius * cos(angle)).toFloat() - segW / 2f
            val top = center + (radius * sin(angle)).toFloat() - segH / 2f
            View {
                attr {
                    absolutePosition(top = top, left = left)
                    width(segW)
                    height(segH)
                    borderRadius(segW / 2f)
                    backgroundColor(segColor)
                }
            }
        }
        View {
            attr {
                absolutePosition(top = center - dotSize / 2f, left = center - dotSize / 2f)
                width(dotSize)
                height(dotSize)
                borderRadius(dotSize / 2f)
                backgroundColor(dotColor)
            }
        }
    }
}
