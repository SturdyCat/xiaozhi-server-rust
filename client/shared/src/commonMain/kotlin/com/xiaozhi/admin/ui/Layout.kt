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

/** 左右分栏之间 1px 纵向分隔线（行内 stretch 自动撑满列高）。 */
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
    private class Pane(val show: (() -> Boolean)?, val paint: ViewBuilder)

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
        panes += Pane(show) {
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
        panes += Pane(show) {
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
        panes += Pane(show) { content(this) }
    }

    /** 分发渲染：分隔线只在「前一个分区也真实显示」时才画。 */
    internal fun render(target: ViewContainer<*, *>) {
        // 前面所有条件分区的显示判定（累积）：任一不显示，当前分区就是第一个可见分区
        val guards = mutableListOf<() -> Boolean>()

        fun drawSeparator() {
            if (divided) {
                if (horizontal) target.dividerV() else target.dividerH(trailingGap)
            } else if (spacing > 0f) {
                if (horizontal) target.View { attr { width(spacing) } } else target.View { attr { height(spacing) } }
            }
        }

        panes.forEachIndexed { index, pane ->
            val show = pane.show
            if (index > 0) {
                when {
                    // 前一个分区常驻：分隔线直接画
                    show == null && guards.isEmpty() -> drawSeparator()
                    // 本分区常驻、但前面有条件分区：线随前置条件一起显示
                    show == null -> target.vif({ guards.all { it() } }) { drawSeparator() }
                    // 本分区条件渲染：线在下方与分区同处一个 vif
                }
            }
            when {
                show == null -> pane.paint(target)
                index == 0 -> target.vif(show) { pane.paint(target) }
                else -> {
                    // 分区不显示时线也必须消失（否则会在第一个可见分区前多出一条）
                    target.vif({ guards.all { it() } && show() }) { drawSeparator() }
                    target.vif(show) { pane.paint(target) }
                }
            }
            if (show != null) guards += show
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
