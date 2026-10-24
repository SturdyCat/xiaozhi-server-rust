package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

/**
 * 头部行：**上下布局的「上一段」标准形态**——左边标题（可挂状态/徽标），右边操作。
 *
 * 全站统一入口，两种典型用法：
 * - 卡片头：[HeaderTone.SECTION]（大标题）+ [bottomGap]，下接 [dividerH] 或内容；
 * - **条目标题行**：[HeaderTone.PRIMARY]（body/500/textPrimary）+ [topGap]，用于列表条目
 *   （OTA 条目、MCP 设备块、能力行、会话行…），下方接副标题与详情。
 *
 * 结构上左右就是官方 flex（`flexDirectionRow` + `alignItemsCenter`，标题组 `flex(1f)`），
 * 所以「上：标题+操作 / 下：内容」与「左：菜单 / 右：面板」是同一套原语的两层嵌套：
 *
 * ```
 * splitCard(header = { headerRow("设备 A", meta = { 在线徽标 }, actions = { 删除 }) }) {
 *     menuPaneRow(...)        // 下半部分左右：菜单 │ 1px 竖线 │ 面板
 * }
 * ```
 *
 * [title] 支持 String 与 ViewBuilder 两种：静态标题用前者，面包屑/可点标题用后者。
 * [tone] 控制标题字号与颜色（卡片分组标题 vs 条目名 vs 工具条说明）。
 * [topGap]/[bottomGap] 为行上/下留白；落在定高工具条里时都传 0。
 */
fun ViewContainer<*, *>.headerRow(
    title: String,
    meta: ViewBuilder? = null,
    actions: ViewBuilder? = null,
    tone: HeaderTone = HeaderTone.SECTION,
    topGap: Float = 0f,
    bottomGap: Float = AdminSpace.sm,
    /** 整行可点（可跳转条目）：悬停指针 + 点击回调；null = 纯展示行。 */
    onClick: (() -> Unit)? = null,
) = headerRow(
    title = { headerTitle(title, tone) },
    meta = meta,
    actions = actions,
    topGap = topGap,
    bottomGap = bottomGap,
    onClick = onClick,
)

/** 自定义标题内容的 [headerRow]（面包屑、动态摘要、可点标题等）。 */
fun ViewContainer<*, *>.headerRow(
    title: ViewBuilder,
    meta: ViewBuilder? = null,
    actions: ViewBuilder? = null,
    topGap: Float = 0f,
    bottomGap: Float = AdminSpace.sm,
    onClick: (() -> Unit)? = null,
) {
    // 官方 flex 属性直接表达「左右两端」：标题组占据剩余宽度，操作组内容自适应靠右
    View {
        attr {
            flexDirectionRow()
            alignItemsCenter()
            if (topGap > 0f) paddingTop(topGap)
            if (bottomGap > 0f) paddingBottom(bottomGap)
            if (onClick != null) cursor(CursorType.POINTER)
        }
        if (onClick != null) event { click { onClick() } }
        View {
            attr {
                flex(1f)
                flexDirectionRow()
                alignItemsCenter()
            }
            title(this)
            if (meta != null) meta(this)
        }
        if (actions != null) actions(this)
    }
}

/** 标题字号/颜色档位。 */
enum class HeaderTone {
    /** 卡片分组标题：section(21)、500、textPrimary。 */
    SECTION,

    /** **条目名**（列表项标题行）：body(17)、500、textPrimary。 */
    PRIMARY,

    /** 页面/工具条说明：body(17)、textSecondary（如「12 台设备在线」这类摘要）。 */
    BODY,

    /** 密集工具条：caption(15)、textSecondary。 */
    CAPTION,

    /** 最紧凑的栏名（如工作流左上的小标题）：micro(13)、500、textSecondary。 */
    MICRO,
}

/** [headerRow] 的标题文本件（[tone] 决定字号与颜色）；自定义标题槽里可直接复用。 */
fun ViewContainer<*, *>.headerTitle(title: String, tone: HeaderTone = HeaderTone.SECTION) {
    Text {
        attr {
            when (tone) {
                HeaderTone.SECTION -> {
                    fontSize(AdminType.section)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                }
                HeaderTone.PRIMARY -> {
                    fontSize(AdminType.body)
                    fontWeightMedium()
                    color(AdminColors.textPrimary)
                }
                HeaderTone.BODY -> {
                    fontSize(AdminType.body)
                    color(AdminColors.textSecondary)
                }
                HeaderTone.CAPTION -> {
                    fontSize(AdminType.caption)
                    color(AdminColors.textSecondary)
                }
                HeaderTone.MICRO -> {
                    fontSize(AdminType.micro)
                    fontWeightMedium()
                    color(AdminColors.textSecondary)
                }
            }
            text(title)
        }
    }
}
