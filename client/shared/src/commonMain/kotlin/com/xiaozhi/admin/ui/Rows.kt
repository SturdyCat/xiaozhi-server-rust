package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 语义行组件：把「名称—值」「名称—值—说明」「副标题」这类反复手写的行结构收敛成组件。
 *
 * 这些行此前在 OpsPages / OtaCard / McpToolsCard / ConfigOverviewCard / EchoMuse 各卡里
 * 各写一遍，字号、颜色、行距、宽度各不同（同一个「键值行」出现过 72 定宽 + caption、
 * flex 均分两种画法）。这里以官方 [View]/[Text] 为基础封装，只表达**语义**，
 * 视觉由 token 决定，调用方不再写 fontSize/color。
 *
 * 全部值以 String 传入（快照）：这些行渲染的都是不可变数据行；确实需要就地响应式刷新的，
 * 用接收 lambda 的重载（内部在 attr 闭包里求值，符合 Kuikly 的观察契约）。
 */

// ===================== 键值行 =====================

/**
 * 键值行：左固定宽标签（caption / textSecondary），右值（caption / textPrimary，占满余宽）。
 *
 * ```
 * infoRow("固件版本", row.firmware)
 * infoRow("IP", row.ip, labelWidth = 88f)
 * ```
 *
 * [topGap] 默认 xs —— 与 EchoMuse 信息行的既有节奏一致；列表首行传 0。
 */
fun ViewContainer<*, *>.infoRow(
    label: String,
    value: String,
    labelWidth: Float = INFO_LABEL_WIDTH,
    topGap: Float = AdminSpace.xs,
) = infoRow({ label }, { value }, labelWidth, topGap)

/** 动态值的 [infoRow]：label/value 在 attr 闭包内求值，observable 变化会就地刷新。 */
fun ViewContainer<*, *>.infoRow(
    label: () -> String,
    value: () -> String,
    labelWidth: Float = INFO_LABEL_WIDTH,
    topGap: Float = AdminSpace.xs,
) {
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            if (topGap > 0f) paddingTop(topGap)
        }
        Text {
            attr {
                width(labelWidth)
                fontSize(AdminType.caption)
                color(AdminColors.textSecondary)
                text(label())
            }
        }
        Text {
            attr {
                flex(1f)
                fontSize(AdminType.caption)
                color(AdminColors.textPrimary)
                text(value())
            }
        }
    }
}

/** 键值行的标签默认宽度：够放「唤醒词检测位置」这类 7 字标签。 */
const val INFO_LABEL_WIDTH: Float = 72f

// ===================== 指标行 =====================

/**
 * 指标行：`名称 …… 大号值` + 下行一句口径说明（hint）。
 *
 * 运维各页（概览/用量/延迟）的实时数字都是这个形态，此前是 OpsPages 的私有函数；
 * 上移到组件库后其它页面（设备详情、测试台）也能直接用同一套排版。
 *
 * [hint] 为空则不渲染说明行——**不建议省略**：数字脱离口径就是误读来源
 * （"累计 12 轮"到底是本次启动还是历史？）。
 */
fun ViewContainer<*, *>.metricRow(name: String, value: String, hint: String = "") {
    View {
        attr {
            flexDirectionColumn()
            paddingTop(AdminSpace.xs)
            paddingBottom(AdminSpace.xs)
        }
        // 官方 flex：名称占满余宽、值内容自适应贴右
        View {
            attr {
                flexDirectionRow()
                alignItemsCenter()
            }
            Text {
                attr {
                    flex(1f)
                    fontSize(AdminType.body)
                    color(AdminColors.textSecondary)
                    text(name)
                }
            }
            Text {
                attr {
                    fontSize(AdminType.title)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                    text(value)
                }
            }
        }
        if (hint.isNotEmpty()) textNote(hint, micro = true)
    }
}

// ===================== 文本行 =====================

/**
 * 副标题行：条目名下面那行 caption / textSecondary（ID、地址、摘要）。
 * 与 [textNote] 的区别：它是**条目的第二部分**（不是提示/警告），色更实一档。
 * [topGap] 用于与上一行拉开距离（如指标块下方的合计行），默认 0。
 */
fun ViewContainer<*, *>.captionLine(text: String, topGap: Float = 0f) = captionLine({ text }, topGap)

/** 动态值的 [captionLine]。 */
fun ViewContainer<*, *>.captionLine(text: () -> String, topGap: Float = 0f) {
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textSecondary)
            if (topGap > 0f) marginTop(topGap)
            text(text())
        }
    }
}

/**
 * 行内补充说明：caption / textTertiary + 可选顶部小距（4）。
 * 用于「输入 128 · 输出 64」这类跟在主行下面的次级信息。
 */
fun ViewContainer<*, *>.detailLine(text: String, topGap: Float = 4f) = detailLine({ text }, topGap)

/** 动态值的 [detailLine]。 */
fun ViewContainer<*, *>.detailLine(text: () -> String, topGap: Float = 4f) {
    Text {
        attr {
            fontSize(AdminType.caption)
            color(AdminColors.textTertiary)
            if (topGap > 0f) marginTop(topGap)
            text(text())
        }
    }
}

// ===================== 分组小标题 =====================

/**
 * 分组小标题（如侧栏「语音与对话」、工作区菜单分组、表单分组）：
 * micro / textTertiary，上距 md、下距 xs。
 *
 * 此前侧栏 [AdminPage.navGroupHeader]、配置表单 `configGroupHeader`、工作区菜单
 * `workspaceMenuHeading` 三份实现，差异只是左右缩进——缩进由 [inset] 表达。
 */
fun ViewContainer<*, *>.groupLabel(label: String, inset: Float = AdminSpace.sm, topGap: Float = AdminSpace.md) {
    Text {
        attr {
            fontSize(AdminType.micro)
            fontWeightMedium()
            color(AdminColors.textTertiary)
            text(label)
            marginLeft(inset)
            marginRight(inset)
            marginTop(topGap)
            marginBottom(AdminSpace.xs)
        }
    }
}

// ===================== 诊断输出块 =====================

/**
 * 有界滚动输出块（BLE / 节点诊断日志共用）：官方 [Scroller] + 等宽小字，
 * insetBg 底、radiusSm 圆角，高度固定避免日志把卡片撑爆。[content] 是 lambda，
 * 长跑日志更新时在 attr 闭包里重新取值。
 *
 * ⚠️ 不把诊断数据放进可编辑的输入框（官方 Input 会带 IME/选区行为，且语义上可写）。
 */
fun ViewContainer<*, *>.outputBlock(
    content: () -> String,
    height: Float = 180f,
    emptyText: String = "（暂无输出）",
) {
    Scroller {
        attr {
            this.height(height)
            marginTop(AdminSpace.xs)
            padding(AdminSpace.sm)
            borderRadius(AdminShape.radiusSm)
            backgroundColor(AdminColors.insetBg)
        }
        Text {
            attr {
                alignSelfStretch()
                fontSize(AdminType.micro)
                color(AdminColors.textSecondary)
                text(content().ifEmpty { emptyText })
            }
        }
    }
}
