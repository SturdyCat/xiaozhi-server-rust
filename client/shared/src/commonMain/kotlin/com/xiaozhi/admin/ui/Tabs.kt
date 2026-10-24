package com.xiaozhi.admin.ui

import com.tencent.kuikly.core.base.Color
import com.tencent.kuikly.core.base.CursorType
import com.tencent.kuikly.core.base.PagerScope
import com.tencent.kuikly.core.base.ViewBuilder
import com.tencent.kuikly.core.base.ViewContainer
import com.tencent.kuikly.core.base.ViewRef
import com.tencent.kuikly.core.nvi.serialization.json.JSONObject
import com.tencent.kuikly.core.reactive.handler.observable
import com.tencent.kuikly.core.views.PageList
import com.tencent.kuikly.core.views.PageListView
import com.tencent.kuikly.core.views.Scroller
import com.tencent.kuikly.core.views.ScrollParams
import com.tencent.kuikly.core.views.TabItem
import com.tencent.kuikly.core.views.Tabs
import com.tencent.kuikly.core.views.Text
import com.tencent.kuikly.core.views.View

// ===================== Tabs 标签页 =====================

/**
 * Tabs 标签页 UI 状态：官方 Tabs ↔ PageList 联动的两根数据线——
 * PageList `scroll` 事件回传 ScrollParams 喂给 Tabs.scrollParams，驱动指示条与 tab 选中态同步。
 * 由各状态类经 PagerScope 创建（如 form.tabUi），section 重建后当前 tab 不丢。
 */
class TabUiState(scope: PagerScope) {
    /** 当前页 index（由 PageList pageIndexDidChanged 回写），重建后 defaultPageIndex 用。 */
    var tabIndex by scope.observable(0)

    /** PageList scroll 事件回传的滚动参数（对齐官方示例：nullable，首帧未滚动时为 null）。 */
    var tabScroll by scope.observable<ScrollParams?>(null)

    /** PageList 引用：点击 tab 时 scrollToPageIndex 翻页。 */
    var pageListRef: ViewRef<PageListView<*, *>>? = null

    /** 点击 tab：滚动 PageList 到对应页（scroll 事件回流后 Tabs 指示条/选中态自动同步）。 */
    fun switchTo(index: Int) {
        pageListRef?.view?.scrollToPageIndex(index, false)
    }
}

/** 单个标签页：title = tab 项文案；content 在该页的 Scroller 闭包内执行（节点挂进该页）。 */
class TabPage(val title: String, val content: ViewBuilder)

/**
 * 标签栏（官方 [Tabs] 封装，**无 PageList 版**）：只画 tab 项与滑动指示条，
 * 内容由调用方用 `vif` 就地切换。
 *
 * 与 [tabbedPanel] 的分工：那个是「整页翻页」（需要显式 pageWidth/pageHeight、
 * 每页自带 Scroller，适合整块工作区）；这里是「面板内分区」，内容随容器高度自适应，
 * 适合卡片里的小范围切分（如 EchoMuse 设备管理的 状态/信息/日志/维护）。
 *
 * 为什么能脱离 PageList 工作：官方 Tabs 的指示条位置与 tab 选中态只从
 * `scrollParams`（滚动进度）或 `defaultInitIndex` 推导，并由 [TabsAttr.scrollParams]
 * 在变更时回调 `updateIndicatorPositionIfNeed`。这里按「固定宽 tab × 选中下标」
 * 合成一份「已滚到第 index 页」的 [ScrollParams] 喂回去，于是：
 * 点 tab → 调用方改下标 observable → attr 闭包重跑 → 指示条滑动 + TabItem.selected 回写。
 *
 * ⚠️ 因此每个 tab **等宽**（[itemWidth]）：等宽才能让合成的滚动参数与真实 tab 帧对上
 * （自适应宽度下指示条会算偏）。tab 文案因此要短。
 * ⚠️ 必须在目标容器闭包内**非限定**调用（经 ctx. 调用会把节点挂到 Pager 根容器）。
 */
fun ViewContainer<*, *>.tabBar(
    tabs: List<String>,
    selectedIndex: () -> Int,
    onSelect: (Int) -> Unit,
    itemWidth: Float = 96f,
    barHeight: Float = 44f,
    indicatorInset: Float = 30f,
    /** 标签栏底色；null = 透明（沿用父容器底色，如卡片内）。 */
    background: Color? = null,
) {
    if (tabs.isEmpty()) return
    Tabs {
        attr {
            // Tabs 源码强约束：必须显式 height，否则运行时 throwRuntimeError
            this.height(barHeight)
            background?.also { backgroundColor(it) }
            val index = selectedIndex().coerceIn(0, tabs.lastIndex)
            defaultInitIndex(index)
            scrollParams(
                ScrollParams(
                    offsetX = itemWidth * index,
                    offsetY = 0f,
                    contentWidth = itemWidth * tabs.size,
                    contentHeight = barHeight,
                    viewWidth = itemWidth,
                    viewHeight = barHeight,
                    isDragging = false,
                ),
            )
            indicatorAlignCenter()
            // 官方指示条：TabItem 底部短条（absolutePosition 到 tab 帧内的右下），随下标滑动
            indicatorInTabItem {
                View {
                    attr {
                        absolutePosition(left = indicatorInset, right = indicatorInset, bottom = 5f)
                        height(3f)
                        borderRadius(1.5f)
                        backgroundColor(AdminColors.accent)
                    }
                }
            }
        }
        tabs.forEachIndexed { index, title ->
            TabItem { state ->
                attr {
                    width(itemWidth)
                    this.height(barHeight)
                    allCenter()
                    // tab 是点击目标：悬停指针
                    cursor(CursorType.POINTER)
                }
                Text {
                    attr {
                        text(title)
                        fontSize(AdminType.body)
                        fontWeightMedium()
                        // state.selected 是官方 TabItem 的响应式选中态（由 Tabs 回写）
                        if (state.selected) color(AdminColors.accentTintText) else color(AdminColors.textSecondary)
                    }
                }
                event { click { onSelect(index) } }
            }
        }
    }
}

/**
 * 标签页面板：**官方 Tabs + PageList 原生联动结构**（对齐官方文档
 * kuikly.tds.qq.com/API/components/tabs.html 与 TabsExamplePage.kt 示例，不自造样式）：
 * - tab 项 = TabItem(margin + allCenter) + Text（选中态文字色随 state.selected 切换）；
 * - 指示条 = indicatorInTabItem（absolutePosition 底部短条，官方原样）；
 * - PageList 必须显式设 pageItemWidth/pageItemHeight（官方示例同款）——不设时 item 无尺寸约束
 *   会塌成一行（实测「tab 内容都看不到」的根因）；
 * - 点击 tab → scrollToPageIndex 翻页；翻页/拖动 → scroll 事件 → ui.tabScroll → Tabs 同步。
 *
 * ⚠️ pageWidth/pageHeight 必须由调用方按自身布局算好（官方示例亦按导航/tab 高度手算），
 * 且以 lambda 传入、在 PageList attr 闭包内求值——pagerData.pageViewWidth/Height 是响应式
 * 字段，窗口 resize 时 pageItem 尺寸才会重算。
 * ⚠️ 必须在目标容器闭包内**非限定**调用（经 ctx. 调用会把节点挂到根容器，布局逃逸）。
 */
fun ViewContainer<*, *>.tabbedPanel(
    pages: List<TabPage>,
    ui: TabUiState,
    pageWidth: () -> Float,
    pageHeight: () -> Float,
) {
    Tabs {
        attr {
            indicatorAlignCenter()
            height(44f) // Tabs 源码强约束：必须显式 height，否则运行时抛错
            defaultInitIndex(ui.tabIndex)
            // 官方指示条原样：tab 项底部 absolutePosition 短条，随 scrollParams 联动滚动
            indicatorInTabItem {
                View {
                    attr {
                        absolutePosition(left = 15f, right = 15f, bottom = 5f)
                        height(6f)
                        borderRadius(3f)
                        backgroundColor(AdminColors.accent)
                    }
                }
            }
            ui.tabScroll?.also { scrollParams(it) }
        }
        pages.forEachIndexed { index, page ->
            TabItem { state ->
                attr {
                    marginLeft(10f)
                    marginRight(10f)
                    allCenter()
                    // tab 是点击目标：悬停指针
                    cursor(CursorType.POINTER)
                }
                event {
                    click { ui.switchTo(index) }
                }
                Text {
                    attr {
                        text(page.title)
                        fontSize(AdminType.body)
                        if (state.selected) {
                            color(AdminColors.accentTintText)
                        } else {
                            color(AdminColors.textSecondary)
                        }
                    }
                }
            }
        }
    }
    PageList {
        attr {
            flexDirectionRow()
            pageItemWidth(pageWidth())
            pageItemHeight(pageHeight())
            defaultPageIndex(ui.tabIndex)
            // 首屏全量加载：官方文档要求 defaultPageIndex > 0 时 Tabs 首渲染才能正确联动到对应 item
            firstContentLoadMaxIndex(pages.size)
            offscreenPageLimit(1)
        }
        ref { ui.pageListRef = it }
        event {
            scroll { params -> ui.tabScroll = params }
            pageIndexDidChanged { params ->
                ui.tabIndex = (params as JSONObject).optInt("index")
            }
        }
        pages.forEach { page ->
            // 每个 PageList item = 一页（尺寸由 pageItemWidth/Height 约束）；页内 Scroller 纵向滚动
            View {
                attr {
                    flexDirectionColumn()
                }
                Scroller {
                    attr {
                        flex(1f)
                        paddingLeft(AdminSpace.xl)
                        paddingRight(AdminSpace.xl)
                        paddingTop(AdminSpace.xxl)
                        paddingBottom(AdminSpace.xxxl)
                    }
                    // ⚠️ ViewBuilder 值须显式传接收者调用：page.content(this) 把页面内容挂进本页 Scroller
                    // （隐式 page.content() 编译报 "No value passed for parameter 'p1'"）
                    page.content(this)
                }
            }
        }
    }
}
