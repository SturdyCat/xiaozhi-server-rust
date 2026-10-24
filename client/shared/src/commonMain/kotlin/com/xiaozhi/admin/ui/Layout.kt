package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.directives.velse
import com.tencent.kuikly.core.directives.vif
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.View

/**
 * 分区布局原语：**左右（[splitRow]）/ 上下（[splitColumn]）**，以及两者之间 1px 分隔线的
 * 唯一出处。
 *
 * 为什么不各页自画：分隔线的粗细、颜色、有无、留白在各处手写后会漂移（历史上侧栏、
 * 配置工作区、EchoMuse 设备卡就各有一份）。这里把「分区 + 分区之间的东西」收敛成一种
 * 结构，任意嵌套即可组合出全站布局：
 *
 * ```
 * splitColumn {                      // 上下：标题 │ 内容
 *     auto { 标题 }
 *     grow { splitRow { ... } }      // 内容里再左右：菜单 │ 面板
 * }
 * ```
 *
 * 分区只有三种尺寸语义：
 * - [SplitScope.fixed]：主轴固定尺寸（左右=宽，上下=高），如左菜单、侧边栏；
 * - [SplitScope.grow]：按 [SplitScope.grow] 的 weight 分剩余空间，如右面板；
 * - [SplitScope.auto]：主轴尺寸由内容决定（上下布局的标题行等），不额外包容器。
 *
 * ⚠️ 分区闭包内继续调 [splitRow] / [splitColumn] 即可嵌套，不要手写
 * `View { attr { flexDirectionRow() } }` + 裸 1px 线。
 */

// ===================== 分隔线 =====================

/** 卡片内 1px 横向分隔线（行间分隔）。[trailingGap] 为线下方留白，0 时不占额外空间。 */
fun ViewContainer<*, *>.dividerH(trailingGap: Float = AdminSpace.sm) {
    View {
        attr {
            height(1f)
            // 显式拉伸交叉轴（宽度）：父容器可能是 alignItemsCenter 的行，靠默认值会塌成 0 宽
            alignSelfStretch()
            backgroundColor(AdminColors.divider)
            if (trailingGap > 0f) marginBottom(trailingGap)
        }
    }
}

/**
 * 左右分栏之间 1px 纵向分隔线（stretch 版）。
 *
 * ⚠️ 只在**交叉轴尺寸已确定**的行里可靠（如挂在 `flex(1f)` 定高容器上的主壳分栏）。
 * 高度由内容决定的卡片内分栏请用 [dividerVAt]，否则线高恒为 0（见该函数说明）。
 */
fun ViewContainer<*, *>.dividerV() {
    View {
        attr {
            width(1f)
            // 显式拉伸交叉轴（高度）：父行若设了 alignCenter，默认值会让线塌成 0 高（看不见）
            alignSelfStretch()
            backgroundColor(AdminColors.divider)
        }
    }
}

/**
 * 纵向分隔线（绝对定位版）：`left` 由所在行的定宽分区累计得出，`top/bottom = 0`
 * 把线拉满整行高度。
 *
 * **为什么不能只靠 [dividerV] 的 alignSelfStretch**：Kuikly 的 flex 交叉轴 stretch 只在
 * 「父容器交叉轴尺寸已定」时才生效（`LayoutImpl`：`alignItem == STRETCH &&
 * isCrossDimDefined` 才预填充子项）。卡片内左右分栏的高度由内容决定，父高要等子项排完
 * 才算出，源码此处至今是：
 *
 * ```
 * // TODO(prenaux): Correctly set the height of items with undefined (auto) crossAxis dimension.
 * ```
 *
 * 结果就是线节点在树里、宽度 1px、**高度恒为 0**——「卡片里没有分割线」的真正原因
 * （主壳分栏挂在定高容器上，同一条线却看得见，差异就在这里）。
 *
 * 绝对定位子项走**父尺寸算出之后的二次布局**（`LayoutImpl` 的 <Loop G> 遍历
 * `firstAbsoluteChild`）：top/bottom 同时给出、自身又不设高度时才回填成父高，因此这一版
 * 能取到真实行高。线不占主轴位置（1px 压在交界处），调用方无需再留 1px 间隙。
 */
private fun ViewContainer<*, *>.dividerVAt(left: Float) {
    View {
        attr {
            absolutePosition(top = 0f, left = left, bottom = 0f)
            width(1f)
            backgroundColor(AdminColors.divider)
        }
    }
}

// ===================== 间隔 =====================

/**
 * 横向间隔（左右排列里的「间距」）。
 *
 * Kuikly 的 flex 样式没有 `gap` 属性（见官方 `FlexStyle`：只有 direction/justify/align/
 * flex/wrap 与四个 margin），所以间距只能用「占位元素」或子项 margin 表达。这里把
 * 「占位」收敛成一个语义函数：调用处写 [hGap]，不写裸
 * `View { attr { width(AdminSpace.sm) } }`——后者在全站散落 20+ 处，
 * 既看不出意图，也没法统一改。
 *
 * ⚠️ 只在**横向容器**（[splitRow]、`flexDirectionRow`）里用；纵向留白请用 margin 或 [vGap]。
 */
fun ViewContainer<*, *>.hGap(width: Float = AdminSpace.sm) {
    View { attr { this.width(width) } }
}

/** 纵向间隔（上下排列里的「间距」）。同 [hGap]，纵向容器里用。 */
fun ViewContainer<*, *>.vGap(height: Float = AdminSpace.sm) {
    View { attr { this.height(height) } }
}

// ===================== 左右 / 上下 =====================

/**
 * 左右布局：分区依次横排，相邻分区之间插 1px 纵向分隔线（[divider]=false 时改为
 * [spacing] 宽的空白）。[fillHeight]=true 时整行 flex(1f) 撑满剩余高度（挂定高父容器）。
 * [alignCenter]=true 时子项在交叉轴居中（头部行「标题 ▸ 操作」用；分隔线自带
 * alignSelfStretch，不会因此塌掉）。
 */
fun ViewContainer<*, *>.splitRow(
    fillHeight: Boolean = false,
    divider: Boolean = true,
    spacing: Float = 0f,
    alignCenter: Boolean = false,
    body: SplitScope.() -> Unit,
) = split(
    horizontal = true,
    fillHeight = fillHeight,
    divided = divider,
    trailingGap = 0f,
    spacing = spacing,
    alignCenter = alignCenter,
    body = body,
)

/**
 * 上下布局：分区依次纵排，相邻分区之间插 1px 横向分隔线（保留 [trailingGap] 留白，
 * 与卡片「标题 │ 内容」的间距节奏一致；紧贴排列的栏位传 0）。
 */
fun ViewContainer<*, *>.splitColumn(
    fillHeight: Boolean = false,
    divider: Boolean = true,
    trailingGap: Float = AdminSpace.sm,
    spacing: Float = 0f,
    alignCenter: Boolean = false,
    body: SplitScope.() -> Unit,
) = split(
    horizontal = false,
    fillHeight = fillHeight,
    divided = divider,
    trailingGap = trailingGap,
    spacing = spacing,
    alignCenter = alignCenter,
    body = body,
)

private fun ViewContainer<*, *>.split(
    horizontal: Boolean,
    fillHeight: Boolean,
    divided: Boolean,
    trailingGap: Float,
    spacing: Float,
    alignCenter: Boolean,
    body: SplitScope.() -> Unit,
) {
    View {
        attr {
            if (horizontal) flexDirectionRow() else flexDirectionColumn()
            if (fillHeight) flex(1f)
            if (alignCenter) alignItemsCenter()
        }
        val scope = SplitScope(horizontal, divided, trailingGap, spacing)
        scope.body()
        scope.render(this)
    }
}

/**
 * 分区收集作用域：由 [splitRow] / [splitColumn] 创建，只在对应闭包内使用。
 *
 * 分区先登记、最后统一渲染——分隔线要画在「相邻两个**实际显示**的分区之间」，只有拿到
 * 全部登记（含各自的条件）才判断得准（如窄屏隐藏侧栏时，其后的竖线必须一起消失）。
 */
class SplitScope internal constructor(
    private val horizontal: Boolean,
    private val divided: Boolean,
    private val trailingGap: Float,
    private val spacing: Float,
) {
    /**
     * [width] 仅左右布局的常驻定宽分区有值：它是下一个分隔线的水平锚点（绝对定位竖线要
     * 知道 left）。弹性分区、条件分区都无法在构建期给出确定位置，只能退回 stretch 版线。
     */
    private class Pane(val show: (() -> Boolean)?, val width: Float?, val paint: ViewBuilder)

    private val panes = mutableListOf<Pane>()

    /**
     * 固定尺寸分区：左右布局 = 宽度，上下布局 = 高度。用于左菜单、侧边栏等。
     *
     * [show] 非空时该分区按条件渲染（如窄屏隐藏侧边栏），其前置分隔线一并跟随。
     */
    fun fixed(
        size: Float,
        show: (() -> Boolean)? = null,
        background: Color = AdminColors.transparent,
        padding: Float = 0f,
        paddingLeft: Float = 0f,
        paddingTop: Float = 0f,
        paddingRight: Float = 0f,
        paddingBottom: Float = 0f,
        content: ViewBuilder,
    ) {
        panes += Pane(show, if (horizontal) size else null) {
            View {
                attr {
                    if (horizontal) width(size) else height(size)
                    flexDirectionColumn()
                    backgroundColor(background)
                    if (padding > 0f) padding(padding)
                    if (paddingLeft > 0f) paddingLeft(paddingLeft)
                    if (paddingTop > 0f) paddingTop(paddingTop)
                    if (paddingRight > 0f) paddingRight(paddingRight)
                    if (paddingBottom > 0f) paddingBottom(paddingBottom)
                }
                content(this)
            }
        }
    }

    /** 弹性分区：按 [weight] 分配主轴剩余空间（默认 1）。用于主内容面板。 */
    fun grow(
        weight: Float = 1f,
        show: (() -> Boolean)? = null,
        background: Color = AdminColors.transparent,
        padding: Float = 0f,
        paddingLeft: Float = 0f,
        paddingTop: Float = 0f,
        paddingRight: Float = 0f,
        paddingBottom: Float = 0f,
        content: ViewBuilder,
    ) {
        panes += Pane(show, null) {
            View {
                attr {
                    flex(weight)
                    flexDirectionColumn()
                    backgroundColor(background)
                    if (padding > 0f) padding(padding)
                    if (paddingLeft > 0f) paddingLeft(paddingLeft)
                    if (paddingTop > 0f) paddingTop(paddingTop)
                    if (paddingRight > 0f) paddingRight(paddingRight)
                    if (paddingBottom > 0f) paddingBottom(paddingBottom)
                }
                content(this)
            }
        }
    }

    /**
     * 内容自适应分区：主轴尺寸由内容决定，**不额外包容器**（不引入多余的布局节点），
     * 适合上下布局的标题行、说明行，或左右布局的图标等固定内容块。
     */
    fun auto(show: (() -> Boolean)? = null, content: ViewBuilder) {
        panes += Pane(show, null) { content(this) }
    }

    /**
     * 分发渲染：分隔线只在「前一个分区也真实显示」时才画。
     *
     * 左右布局的竖线**优先用绝对定位**（[dividerVAt]）：位置由前面的常驻定宽分区累计得出，
     * 高度交给 Kuikly 的绝对定位二次布局撑满整行——这是卡片这类「高度由内容决定」的容器
     * 里唯一能画出等高分栏线的办法（stretch 版线高恒为 0，原因见 [dividerVAt]）。
     * 位置算不出来时（前面有弹性/条件分区）退回 [dividerV] 的 stretch 版。
     */
    internal fun render(target: ViewContainer<*, *>) {
        // 前面所有条件分区的显示判定（快照）：任一不显示，当前分区就是第一个可见分区
        val guards = mutableListOf<() -> Boolean>()
        // 绝对定位竖线：left + 显示条件（null = 恒显示）。分区挂完再统一挂，绘制在最上层
        val absLines = mutableListOf<Pair<Float, (() -> Boolean)?>>()
        var offset = 0f          // 前面常驻定宽分区的总宽 = 下一条竖线的 left
        var offsetKnown = true

        fun ViewContainer<*, *>.flowSeparator() {
            if (divided) {
                if (horizontal) dividerV() else dividerH(trailingGap)
            } else if (spacing > 0f) {
                if (horizontal) View { attr { width(spacing) } } else View { attr { height(spacing) } }
            }
        }

        panes.forEachIndexed { index, pane ->
            val show = pane.show
            val before = guards.toList()
            // 线的显示条件：前面条件分区都可见，且本分区可见
            val lineCond: (() -> Boolean)? = when {
                before.isEmpty() && show == null -> null
                before.isEmpty() -> show
                show == null -> ({ before.all { it() } })
                else -> ({ before.all { it() } && show() })
            }
            val lineLeft = offset
            val lineAbsolute = divided && horizontal && offsetKnown

            if (index > 0) {
                if (lineAbsolute) {
                    absLines += lineLeft to lineCond
                } else if (lineCond == null) {
                    target.flowSeparator()
                } else {
                    target.vif({ lineCond() }) { flowSeparator() }
                }
                // 用空白间距代替分隔线时，间距本身占主轴位置
                if (!divided && spacing > 0f) offset += spacing
            }

            if (show == null) pane.paint(target) else target.vif(show) { pane.paint(target) }

            // 主轴推进：只有常驻定宽分区能确定下一条线的位置
            val width = pane.width
            when {
                !offsetKnown -> Unit
                show != null || width == null -> offsetKnown = false
                else -> offset += width
            }
            if (show != null) guards += show
        }

        for ((left, cond) in absLines) {
            if (cond == null) target.dividerVAt(left) else target.vif({ cond() }) { dividerVAt(left) }
        }
    }
}

// ===================== 常用组合（保留既有语义名） =====================

/**
 * 「左菜单 │ 1px 分隔线 │ 右面板」：[splitRow] 的语义别名——配置工作区、全局配置、
 * EchoMuse 设备卡卡内布局的统一出处，避免各处自画竖线造成宽窄/颜色/有无漂移。
 *
 * [fillHeight]=true 整行 flex(1f) 撑满剩余高度（工作区挂定高父容器），false 行高随内容。
 */
fun ViewContainer<*, *>.menuPaneRow(
    menuWidth: Float,
    fillHeight: Boolean = false,
    menuBackground: Color = AdminColors.transparent,
    panelPaddingLeft: Float = 0f,
    menu: ViewBuilder,
    panel: ViewBuilder,
) {
    splitRow(fillHeight = fillHeight) {
        fixed(menuWidth, background = menuBackground, content = menu)
        grow(paddingLeft = panelPaddingLeft, content = panel)
    }
}

/**
 * 卡片行：[splitRow] 的语义别名。一行最多两列、等宽，列间距 [AdminSpace.gutter]
 * （无分隔线）；right 为 null 时左卡独占整行。
 */
fun ViewContainer<*, *>.cardRow(left: ViewBuilder, right: ViewBuilder? = null) {
    splitRow(divider = false, spacing = AdminSpace.gutter) {
        grow(content = left)
        if (right != null) grow(content = right)
    }
}

/**
 * 双栏页：宽屏（wide()=true）左右等宽并排，窄屏改为 [splitColumn] 纵向堆叠。
 * wide 须为 lambda（如 `{ pagerData.pageViewWidth >= 900f }`），在 vif 闭包内读取
 * pageViewWidth（响应式字段），窗口 resize 跨过阈值时自动 并排 ⇄ 堆叠 切换。
 */
fun ViewContainer<*, *>.twoPane(wide: () -> Boolean, left: ViewBuilder, right: ViewBuilder) {
    vif({ wide() }) {
        cardRow(left, right)
    }
    velse {
        splitColumn(divider = false) {
            auto(content = left)
            auto(content = right)
        }
    }
}

// ===================== 页面滚动区 =====================

/**
 * 页面滚动区（官方 [Scroller]）：`flex(1f)` 占满剩余高度 + 统一左右/上下留白 +
 * 隐藏滚动条。卡片自身的上下间距由 [cardSurface] 的 marginBottom 提供，这里只负责
 * 「页边距」。
 *
 * 全站主内容区（运维各页、文件、EchoMuse 等）共用这一份；各页不再自己写
 * `Scroller { attr { flex(1f); paddingLeft(xl) … } }`，避免页边距各页一个值。
 */
fun ViewContainer<*, *>.pageScroller(
    horizontalPadding: Float = AdminSpace.xl,
    paddingTop: Float = AdminSpace.xxl,
    paddingBottom: Float = AdminSpace.xxxl,
    content: ViewBuilder,
) {
    Scroller {
        attr {
            flex(1f)
            showScrollerIndicator(false)
            paddingLeft(horizontalPadding)
            paddingRight(horizontalPadding)
            this.paddingTop(paddingTop)
            this.paddingBottom(paddingBottom)
        }
        content(this)
    }
}
