package com.xiaozhi.admin.kit
import com.xiaozhi.admin.kit.LocalAdminSpacing

import androidx.compose.runtime.Composable
import com.gearui.foundation.primitives.ScrollView
import com.gearui.theme.Theme
import com.tencent.kuikly.compose.foundation.background
import com.tencent.kuikly.compose.foundation.layout.Arrangement
import com.tencent.kuikly.compose.foundation.layout.Box
import com.tencent.kuikly.compose.foundation.layout.Column
import com.tencent.kuikly.compose.foundation.layout.ColumnScope
import com.tencent.kuikly.compose.foundation.layout.IntrinsicSize
import com.tencent.kuikly.compose.foundation.layout.PaddingValues
import com.tencent.kuikly.compose.foundation.layout.Row
import com.tencent.kuikly.compose.foundation.layout.RowScope
import com.tencent.kuikly.compose.foundation.layout.Spacer
import com.tencent.kuikly.compose.foundation.layout.fillMaxHeight
import com.tencent.kuikly.compose.foundation.layout.fillMaxWidth
import com.tencent.kuikly.compose.foundation.layout.height
import com.tencent.kuikly.compose.foundation.layout.padding
import com.tencent.kuikly.compose.foundation.layout.width
import com.tencent.kuikly.compose.ui.Alignment
import com.tencent.kuikly.compose.ui.Modifier
import com.tencent.kuikly.compose.ui.graphics.Color
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.compose.ui.unit.dp

/**
 * 分区布局原语（Compose 版）。
 *
 * 旧 `ui/Layout.kt` 是 Kuikly DSL 的 `ViewContainer` 扩展；Compose 里没有 `ViewContainer`，
 * 内容槽一律变成 `@Composable () -> Unit`。**语义与视觉参数一个不改**，只换表达方式。
 *
 * ## 旧版为什么那么复杂，新版为什么不需要
 *
 * 旧实现里有一条 `dividerVAt(left)` 的绝对定位竖线 —— 因为 Kuikly 的 flex
 * 交叉轴 `stretch` 只在「父容器交叉轴尺寸已定」时生效，卡片这种高度由内容决定的容器里
 * 竖线高度恒为 0。Compose 用 `Modifier.height(IntrinsicSize.Min)` 就能让 Row 先按内容
 * 算出行高、再让子项的 `fillMaxHeight()` 取到它，因此**不再需要绝对定位**这条兜底路径。
 *
 * 旧实现里 `SplitScope` 要"先登记、后统一渲染"并跟踪每条分隔线的显示条件（窄屏隐藏侧栏时
 * 竖线必须一起消失）。Compose 里直接在组合期过滤出可见分区再穿插分隔线即可，逻辑保持等价。
 */

// ===================== 分隔线 =====================

/** 卡片内 1dp 横向分隔线（行间分隔）。[trailingGap] 为线下方留白，0 时不占额外空间。 */
@Composable
fun dividerH(trailingGap: Dp = LocalAdminSpacing.current.sm) {
    Box(
        Modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(Theme.colors.border)
            .padding(bottom = trailingGap),
    )
}

/**
 * 左右分栏之间 1dp 纵向分隔线的**起点**：必须是 [splitRow] 的直接子项。
 *
 * 高度靠 `fillMaxHeight()` 取父行高度 —— [splitRow] 会在需要画竖线时给 Row 加
 * `Modifier.height(IntrinsicSize.Min)`，所以这里能拿到真实行高；
 * 若在自定义 `Row` 里单独用它，请自行给 Row 限定高度，否则线高为 0。
 */
@Composable
fun dividerV() {
    Box(
        Modifier
            .width(1.dp)
            .fillMaxHeight()
            .background(Theme.colors.border),
    )
}

// ===================== 间隔 =====================

/**
 * 横向间隔（左右排列里的"间距"）。
 *
 * Compose 的 `Arrangement.spacedBy` 已能表达间距，但旧代码里 `hGap` 是**布局节点**
 * （占位宽度参与分配），换成 `spacedBy` 会改变弹性分区的分配结果。为保持版式逐像素一致，
 * 这里仍是占位元素。
 */
@Composable
fun hGap(width: Dp = LocalAdminSpacing.current.sm) {
    Spacer(Modifier.width(width))
}

/** 纵向间隔。语义同 [hGap]。 */
@Composable
fun vGap(height: Dp = LocalAdminSpacing.current.sm) {
    Spacer(Modifier.height(height))
}

// ===================== 分区作用域 =====================

/**
 * 分区收集作用域：由 [splitRow] / [splitColumn] 创建，只在对应闭包内使用。
 *
 * 分区只有三种尺寸语义（与旧版一一对应）：
 * - [fixed]：主轴固定尺寸（左右=宽，上下=高），如左菜单、侧边栏；
 * - [grow]：按权重分剩余空间，如右面板；
 * - [auto]：主轴尺寸由内容决定，如标题行。
 */
class SplitScope internal constructor(private val horizontal: Boolean) {

    internal class Pane(
        val show: (() -> Boolean)?,
        val size: Dp?,
        val weight: Float?,
        val background: Color,
        val padding: PaddingValues,
        val content: @Composable () -> Unit,
    )

    internal val panes = mutableListOf<Pane>()

    /**
     * 固定尺寸分区：左右布局 = 宽度，上下布局 = 高度。用于左菜单、侧边栏等。
     *
     * [show] 非空时该分区按条件渲染（如窄屏隐藏侧边栏），相邻分隔线一并跟随。
     */
    fun fixed(
        size: Dp,
        show: (() -> Boolean)? = null,
        background: Color = Color.Transparent,
        padding: Dp = 0.dp,
        paddingLeft: Dp = 0.dp,
        paddingTop: Dp = 0.dp,
        paddingRight: Dp = 0.dp,
        paddingBottom: Dp = 0.dp,
        content: @Composable () -> Unit,
    ) {
        panes += Pane(
            show = show,
            size = size,
            weight = null,
            background = background,
            padding = paddingOf(padding, paddingLeft, paddingTop, paddingRight, paddingBottom),
            content = content,
        )
    }

    /** 弹性分区：按 [weight] 分配主轴剩余空间（默认 1）。用于主内容面板。 */
    fun grow(
        weight: Float = 1f,
        show: (() -> Boolean)? = null,
        background: Color = Color.Transparent,
        padding: Dp = 0.dp,
        paddingLeft: Dp = 0.dp,
        paddingTop: Dp = 0.dp,
        paddingRight: Dp = 0.dp,
        paddingBottom: Dp = 0.dp,
        content: @Composable () -> Unit,
    ) {
        panes += Pane(
            show = show,
            size = null,
            weight = weight,
            background = background,
            padding = paddingOf(padding, paddingLeft, paddingTop, paddingRight, paddingBottom),
            content = content,
        )
    }

    /** 内容自适应分区：主轴尺寸由内容决定，不额外包容器。 */
    fun auto(show: (() -> Boolean)? = null, content: @Composable () -> Unit) {
        panes += Pane(
            show = show,
            size = null,
            weight = null,
            background = Color.Transparent,
            padding = PaddingValues(0.dp),
            content = content,
        )
    }

    /**
     * 单侧覆盖优先于统一 [padding]：旧实现的口径是「单侧 > 0 时用单侧，否则用统一值」。
     * `0.dp` 与"不指定"无法区分，故照旧实现语义（单侧给 0 就等同走统一值）。
     */
    private fun paddingOf(base: Dp, left: Dp, top: Dp, right: Dp, bottom: Dp): PaddingValues =
        PaddingValues(
            start = if (left.value > 0f) left else base,
            top = if (top.value > 0f) top else base,
            end = if (right.value > 0f) right else base,
            bottom = if (bottom.value > 0f) bottom else base,
        )

    /** 组合期求值：条件分区按当前快照判定（读状态即建立订阅，跨断点时自动重排）。 */
    internal fun visible(): List<Pane> = panes.filter { it.show?.invoke() ?: true }
}

/**
 * 左右布局：分区依次横排，相邻分区之间插 1dp 纵向分隔线（[divider] = false 时改为 [spacing] 宽的空白）。
 *
 * [fillHeight] = true 时整行撑满剩余高度（挂定高父容器）；
 * false 时用 `IntrinsicSize.Min` 让行高由内容决定、同时让竖线取到行高。
 * [alignCenter] = true 时子项在交叉轴居中（头部行「标题 ▸ 操作」用）。
 */
@Composable
fun splitRow(
    fillHeight: Boolean = false,
    divider: Boolean = true,
    spacing: Dp = 0.dp,
    alignCenter: Boolean = false,
    body: SplitScope.() -> Unit,
) {
    val scope = SplitScope(horizontal = true).apply(body)
    val modifier = when {
        fillHeight -> Modifier.fillMaxHeight()
        divider -> Modifier.height(IntrinsicSize.Min)
        else -> Modifier
    }
    Row(
        modifier = modifier,
        verticalAlignment = if (alignCenter) Alignment.CenterVertically else Alignment.Top,
    ) {
        renderRow(scope, divider, spacing)
    }
}

/**
 * 上下布局：分区依次纵排，相邻分区之间插 1dp 横向分隔线
 * （保留 [trailingGap] 留白，与卡片「标题 │ 内容」的间距节奏一致；紧贴排列的栏位传 0）。
 */
@Composable
fun splitColumn(
    fillHeight: Boolean = false,
    divider: Boolean = true,
    trailingGap: Dp = LocalAdminSpacing.current.sm,
    spacing: Dp = 0.dp,
    alignCenter: Boolean = false,
    body: SplitScope.() -> Unit,
) {
    val scope = SplitScope(horizontal = false).apply(body)
    val modifier = if (fillHeight) Modifier.fillMaxHeight() else Modifier.fillMaxWidth()
    Column(
        modifier = modifier,
        horizontalAlignment = if (alignCenter) Alignment.CenterHorizontally else Alignment.Start,
    ) {
        renderColumn(scope, divider, trailingGap, spacing)
    }
}

@Composable
private fun RowScope.renderRow(scope: SplitScope, divider: Boolean, spacing: Dp) {
    val panes = scope.visible()
    panes.forEachIndexed { index, pane ->
        if (index > 0) {
            when {
                divider -> dividerV()
                spacing.value > 0f -> hGap(spacing)
            }
        }
        val cellModifier = Modifier
            .let { if (pane.size != null) it.width(pane.size) else it }
            .let { if (pane.weight != null) it.weight(pane.weight) else it }
            .let { if (pane.background != Color.Transparent) it.background(pane.background) else it }
            .padding(pane.padding)
        Box(cellModifier) { pane.content() }
    }
}

@Composable
private fun ColumnScope.renderColumn(
    scope: SplitScope,
    divider: Boolean,
    trailingGap: Dp,
    spacing: Dp,
) {
    val panes = scope.visible()
    panes.forEachIndexed { index, pane ->
        if (index > 0) {
            when {
                divider -> dividerH(trailingGap)
                spacing.value > 0f -> vGap(spacing)
            }
        }
        val cellModifier = Modifier
            .let { if (pane.size != null) it.height(pane.size) else it }
            .let { if (pane.weight != null) it.weight(pane.weight) else it }
            .let { if (pane.background != Color.Transparent) it.background(pane.background) else it }
            .padding(pane.padding)
        Box(cellModifier) { pane.content() }
    }
}

// ===================== 常用组合（保留既有语义名） =====================

/**
 * 「左菜单 │ 1dp 分隔线 │ 右面板」：配置工作区、全局配置、EchoMuse 设备卡卡内布局的统一出处，
 * 避免各处自画竖线造成宽窄/颜色/有无漂移。语义与旧 [menuPaneRow] 完全一致。
 */
@Composable
fun menuPaneRow(
    menuWidth: Dp,
    fillHeight: Boolean = false,
    menuBackground: Color = Color.Transparent,
    panelPaddingLeft: Dp = 0.dp,
    menu: @Composable () -> Unit,
    panel: @Composable () -> Unit,
) {
    splitRow(fillHeight = fillHeight) {
        fixed(menuWidth, background = menuBackground, content = menu)
        grow(paddingLeft = panelPaddingLeft, content = panel)
    }
}

/** 卡片行：一行最多两列、等宽，列间距 [LocalAdminSpacing.current.gutter]（无分隔线）；right 为 null 时左卡独占整行。 */
@Composable
fun cardRow(left: @Composable () -> Unit, right: (@Composable () -> Unit)? = null) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(LocalAdminSpacing.current.gutter),
    ) {
        Box(Modifier.weight(1f)) { left() }
        if (right != null) Box(Modifier.weight(1f)) { right() }
    }
}

/**
 * 双栏页：宽屏左右等宽并排，窄屏纵向堆叠。
 *
 * [wide] 在组合期求值 —— 读取 `LocalAdminViewport` 等快照状态时会自动建立订阅，
 * 窗口 resize 跨过阈值即重排（等价于旧版 `vif` 里读 `pageViewWidth` 的效果）。
 */
@Composable
fun twoPane(wide: () -> Boolean, left: @Composable () -> Unit, right: @Composable () -> Unit) {
    if (wide()) {
        cardRow(left, right)
    } else {
        Column(Modifier.fillMaxWidth()) {
            left()
            right()
        }
    }
}

// ===================== 页面滚动区 =====================

/**
 * 页面滚动区：占满剩余高度 + 统一左右/上下留白。
 *
 * 用 GearUI 的 `ScrollView`（`GearLazyColumn` 基座）而不是 `Modifier.verticalScroll` ——
 * 实测 Kuikly Compose 2.30.0 的 `foundation` 包**没有** `verticalScroll` 修饰符，
 * 官方滚动能力在 `foundation.primitives.ScrollView` / `GearLazyColumn`。
 *
 * 卡片自身的上下间距由卡片壳的 `marginBottom` 提供，这里只负责「页边距」。
 */
@Composable
fun pageScroller(
    horizontalPadding: Dp = LocalAdminSpacing.current.xl,
    paddingTop: Dp = LocalAdminSpacing.current.xxl,
    paddingBottom: Dp = LocalAdminSpacing.current.xxxl,
    content: @Composable ColumnScope.() -> Unit,
) {
    ScrollView(
        modifier = Modifier
            .fillMaxHeight()
            .padding(
                start = horizontalPadding,
                end = horizontalPadding,
                top = paddingTop,
                bottom = paddingBottom,
            ),
        content = content,
    )
}
