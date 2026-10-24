package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.Border
import com.tencent.kuikly.core.base.BorderStyle
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View
import com.xiaozhi.admin.workspaceIsCompact

// ===================== 基础排版 =====================

/** 分组标题：section(21)/500，textPrimary。[bottomGap]=0 用于 [headerRow]（间距由行给出）。 */
fun ViewContainer<*, *>.sectionTitle(title: String, bottomGap: Float = AdminSpace.sm) {
    Text {
        attr {
            fontSize(AdminType.section)
            fontWeightMedium()
            color(AdminColors.textPrimary)
            if (bottomGap > 0f) marginBottom(bottomGap)
            text(title)
        }
    }
}

/**
 * 次级说明文字（表单 hint / 就地提示）：caption(15) 或 micro(13) 字号 + 语义色。
 *
 * tone：`tertiary`（默认，解释"这个字段是干什么的"）/
 * `warn`（能构建但能力受限，如"未配 embedding → 词法模式"）/
 * `error`（不可用/失败，如加载失败 + 怎么修）。
 * 全站提示文字的**唯一**入口，避免每处各写一套字号与色值。
 */
fun ViewContainer<*, *>.textNote(text: String, tone: String = "tertiary", micro: Boolean = false) =
    textNote({ text }, { tone }, micro)

/** Dynamic values are read inside attr, never captured as construction-time snapshots. */
fun ViewContainer<*, *>.textNote(text: () -> String, tone: () -> String = { "tertiary" }, micro: Boolean = false) {
    Text {
        attr {
            fontSize(if (micro) AdminType.micro else AdminType.caption)
            color(when (tone()) {
                "warn" -> AdminColors.warningTintText
                "error" -> AdminColors.dangerTintText
                else -> AdminColors.textTertiary
            })
            text(text())
        }
    }
}

// ===================== 卡片 =====================

/**
 * **卡片壳**（全站卡片的唯一出处）：cardBg、radiusLg(12)、1px divider 边框、内边距
 * cardPadding(20)（紧凑窗口降为 sm）、下外边距 gutter。
 *
 * 不带左右外边距——页面留白由 tab 页 Scroller 提供，双列间距由 [cardRow] 控制，
 * 避免嵌套容器时边距叠加（嵌套卡片在内层卡片里也正好贴 padding）。
 *
 * 内容槽是 [SplitScope]：**卡片本身就是上下布局容器**，分区声明完就自动带 1px 横线，
 * 因此「上下包裹左右」只需在分区里再调 [splitRow]：
 *
 * ```
 * cardSurface {                      // 上下：头部 │ 内容
 *     auto { 卡片头部 }
 *     auto { splitRow { ... } }      // 内容里左右：菜单 │ 面板
 * }
 * ```
 *
 * ⚠️ 不要在调用方重抄 cardBg/圆角/边框/padding（历史上 groupedCard、skeletonCard、
 * EchoMuse 设备卡就各抄了一份，改一处漏两处）。
 */
fun ViewContainer<*, *>.cardSurface(
    withDivider: Boolean = true,
    bottomGap: Boolean = true,
    content: SplitScope.() -> Unit,
) {
    View {
        attr {
            flexDirectionColumn()
            backgroundColor(AdminColors.cardBg)
            borderRadius(AdminShape.radiusLg)
            border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            padding(if (workspaceIsCompact(getPager())) AdminSpace.sm else AdminSpace.cardPadding)
            if (bottomGap) marginBottom(AdminSpace.gutter)
        }
        splitColumn(divider = withDivider, body = content)
    }
}

/**
 * 「头部 │ 内容」两段卡片：[cardSurface] + 自定义 header 槽，即最常见的
 * **上下包裹左右**形态（header 行自适应高度，[body] 里可再左右分栏）。
 *
 * 与 [groupedCard] 的区别只在头部：那里是固定样式的 [sectionTitle]，这里由调用方
 * 提供（如 EchoMuse 设备卡要「标题 + 在线状态 + 删除按钮」的自定义 header，
 * 直接用 [headerRow] 声明即可）。
 */
fun ViewContainer<*, *>.splitCard(
    withDivider: Boolean = true,
    header: ViewBuilder,
    body: ViewBuilder,
) {
    cardSurface(withDivider = withDivider) {
        auto(content = header)
        auto(content = body)
    }
}

/**
 * 分组卡片：`标题 │ 1px 分隔线 │ 内容`——[splitCard] 的固定标题版，视觉壳复用 [cardSurface]。
 * 内容区可继续用 [splitRow] / [splitColumn] 任意组合。
 */
fun ViewContainer<*, *>.groupedCard(title: String, withDivider: Boolean = true, content: ViewBuilder) {
    splitCard(withDivider = withDivider, header = { sectionTitle(title) }, body = content)
}

/**
 * 弹层 / 浮层外壳：与 [cardSurface] 同一套外观（cardBg、radiusLg、1px 边框、cardPadding），
 * 差别是宽度由调用方按窗口算（故无左右外边距、无下外边距）。
 * [bordered]=false 用于无边框的轻量弹层（下拉选择等）。
 */
fun ViewContainer<*, *>.dialogSurface(
    width: () -> Float,
    bordered: Boolean = true,
    content: ViewBuilder,
) {
    // 局部别名：避免 attr 内 `width(...)` 与 ViewAttr.width 同名解析歧义
    val widthOf = width
    View {
        attr {
            this.width(widthOf())
            flexDirectionColumn()
            backgroundColor(AdminColors.cardBg)
            borderRadius(AdminShape.radiusLg)
            if (bordered) border(Border(1f, BorderStyle.SOLID, AdminColors.divider))
            padding(AdminSpace.cardPadding)
        }
        content(this)
    }
}

/**
 * 卡片内的**子面板**（inset 底）：insetBg + radiusSm + padding(sm)，可选顶部间距 [topGap]。
 *
 * 用于「一条可编辑条目的块」（NLU 意图/槽位）、「一段诊断结果」、「一条 BLE 广播」这类
 * 嵌在卡片里的小块。此前 5 处各写一遍
 * `padding(sm); backgroundColor(insetBg); borderRadius(radiusSm)`，其中两处漏了圆角、
 * 两处的 marginTop 写在 attr 里而不是属性上——收敛后视觉与节奏一致。
 */
fun ViewContainer<*, *>.insetPanel(
    topGap: Float = AdminSpace.sm,
    content: ViewBuilder,
) {
    View {
        attr {
            flexDirectionColumn()
            if (topGap > 0f) marginTop(topGap)
            padding(AdminSpace.sm)
            backgroundColor(AdminColors.insetBg)
            borderRadius(AdminShape.radiusSm)
        }
        content(this)
    }
}

/**
 * 骨架卡片（P1-4 加载态）：[cardSurface] 同外观，内容为 `rows` 条灰条占位
 * （宽度从满到窄，模拟"标题 + 指标行"的视觉节奏）。
 *
 * 刻意**不做 shimmer 动画**：跨端动画实现成本高（各端渲染管线不同），静态灰条已能
 * 明确表达"内容在路上"——与 [groupedCard] 同宽高节奏，数据到达后切换无跳动。
 * 用法：运维页在 `loading && !loaded` 时先渲染 1~2 张骨架（见 OpsPages 的 opsSkeleton）。
 */
fun ViewContainer<*, *>.skeletonCard(rows: Int = 3) {
    cardSurface(withDivider = false) {
        // 骨架条：高 12 圆角灰条，Column 子项默认 stretch 满宽（同 dividerH），
        // 用 marginRight 递减视觉宽度（0/24/48…），模拟"标题 + 指标行"的层次
        auto {
            for (i in 0 until rows) {
                View {
                    attr {
                        height(12f)
                        borderRadius(AdminShape.radiusPill)
                        backgroundColor(AdminColors.divider)
                        marginRight(i * AdminSpace.xl)
                        marginTop(if (i == 0) 0f else AdminSpace.xs)
                    }
                }
            }
        }
    }
}
