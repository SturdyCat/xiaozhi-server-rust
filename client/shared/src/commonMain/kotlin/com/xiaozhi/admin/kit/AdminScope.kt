package com.xiaozhi.admin.kit

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import com.tencent.kuikly.compose.ui.unit.Dp
import com.tencent.kuikly.core.pager.Pager
import com.xiaozhi.admin.contracts.AdminFieldHelpState

/**
 * Compose 侧的共享作用域（CompositionLocal）+ 断点常量。
 *
 * ## 为什么需要 CompositionLocal 而不是继续 `getPager()`
 *
 * 旧 DSL 里任意容器都能 `getPager()` 回溯到页面，拿到 `pageViewWidth` 与页面级状态。
 * Compose 的组合里没有这种"容器祖先链"，等价的机制是 CompositionLocal：
 * 页面根部（T06 的 `AdminShell`）提供一次，深层组件直接 `LocalAdmin*` 读取，
 * 不必逐层传参，也不会出现「A 页的 ⓘ 弹在 B 页」的串页问题。
 *
 * ## 与旧常量的一一对应（K9：数值一字不改）
 *
 * | 旧符号 | 旧值 | 新符号 |
 * |---|---|---|
 * | `AdminPage.NARROW_BREAKPOINT` | 768 | [GearUiBreakpoints.NARROW] |
 * | `pagerData.pageViewWidth >= 900f`（双列） | 900 | [GearUiBreakpoints.TWO_COLUMN] |
 * | `workspaceIsCompact`（工作区紧凑） | 700 | [GearUiBreakpoints.WORKSPACE_COMPACT] |
 * | `AdminPage.ADMIN_SIDEBAR_WIDTH` | 240 | [GearUiBreakpoints.SIDEBAR_WIDTH] |
 *
 * T06 起壳层改用本对象，旧的四处字面量随之删除。
 */
object GearUiBreakpoints {
    /** 窄屏阈值（< 该值走抽屉侧栏 + 单列）。 */
    const val NARROW = 768f

    /** 双列阈值（>= 该值才左右并排）。 */
    const val TWO_COLUMN = 900f

    /** 工作区紧凑阈值（< 该值卡片内边距降档、字段堆叠）。 */
    const val WORKSPACE_COMPACT = 700f

    /** 侧栏定宽（左右分栏的左列宽度）。 */
    const val SIDEBAR_WIDTH = 240f
}

/**
 * 视口尺寸 + 三个派生断点。
 *
 * 由壳层在 `onSizeChanged` 里写进 [LocalAdminViewport]；组件读它做响应式重排
 * （Compose 的快照机制保证跨阈值时自动重组，等价于旧 `pagerData.pageViewWidth` 的响应式）。
 */
@Immutable
data class AdminViewport(
    val width: Dp,
    val height: Dp,
) {
    /** 窄屏：侧栏改抽屉、双列改单列。 */
    val isNarrow: Boolean get() = width.value < GearUiBreakpoints.NARROW

    /** 够宽，可左右并排两栏。 */
    val twoColumn: Boolean get() = width.value >= GearUiBreakpoints.TWO_COLUMN

    /** 工作区紧凑：卡片内边距、字段排布降档。 */
    val workspaceCompact: Boolean get() = width.value < GearUiBreakpoints.WORKSPACE_COMPACT
}

/**
 * 字段帮助（ⓘ）的**渲染侧契约** —— 定义在 `contracts/FieldHelpState.kt`（T05 搬迁）。
 *
 * 页面持有一份 `FieldHelpState` 实现，经 [LocalFieldHelp] 下发；组件层只依赖契约接口，
 * 不依赖具体状态类 —— 这样 `kit/` 不会反向依赖 `ui/`（T12 要删 `ui/`），也不必为渲染
 * 再引入一层适配。语义见契约文件头注释（一次写入三值、同一时刻只有一个弹层）。
 */

/** 当前页面（Kuikly 的 Pager）。壳层提供，组件层只读（如取宿主能力、页面宽度兜底）。 */
val LocalAdminPager = staticCompositionLocalOf<Pager?> { null }

/** 当前视口尺寸。壳层提供；未提供时给一个"宽屏"默认值，避免组件在预览/测试里崩。 */
val LocalAdminViewport = staticCompositionLocalOf {
    AdminViewport(width = GearUiSpace.pagePadding * 60, height = GearUiSpace.pagePadding * 40)
}

/** 当前页面的字段帮助状态；null = 该页不渲染 ⓘ（同旧 `FieldHelpHost` 未实现的语义）。 */
val LocalFieldHelp = staticCompositionLocalOf<AdminFieldHelpState?> { null }
